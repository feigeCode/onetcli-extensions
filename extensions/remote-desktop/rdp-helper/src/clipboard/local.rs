use std::fmt;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use ironrdp::cliprdr::pdu::{
    ClipboardFileAttributes, FileContentsFlags, FileContentsRequest, FileContentsResponse,
    FileDescriptor,
};

use super::path::{
    MAX_CLIPBOARD_ENTRY_COUNT, MAX_SINGLE_FILE_BYTES, MAX_TOTAL_TRANSFER_BYTES,
    RelativeClipboardPath, ensure_not_link_or_reparse, utf8_file_name, validate_top_level_paths,
};

const MAX_LOCAL_RESPONSE_CHUNK_BYTES: u64 = 4 * 1024 * 1024;
const SIZE_RESPONSE_BYTES: u32 = 8;

#[derive(Clone)]
pub struct LocalClipboardTransfer {
    transfer_id: u64,
    entries: Vec<LocalClipboardEntry>,
    descriptors: Vec<FileDescriptor>,
}

impl LocalClipboardTransfer {
    pub fn collect(transfer_id: u64, paths: Vec<String>) -> anyhow::Result<Self> {
        anyhow::ensure!(transfer_id != 0, "clipboard transfer ID must not be zero");
        let paths = paths.into_iter().map(PathBuf::from).collect::<Vec<_>>();
        anyhow::ensure!(!paths.is_empty(), "clipboard file list is empty");
        validate_top_level_paths(&paths)?;
        let entries = collect_entries(&paths)?;
        let descriptors = entries
            .iter()
            .map(LocalClipboardEntry::descriptor)
            .collect();
        Ok(Self {
            transfer_id,
            entries,
            descriptors,
        })
    }

    pub fn transfer_id(&self) -> u64 {
        self.transfer_id
    }

    pub fn descriptors(&self) -> Vec<FileDescriptor> {
        self.descriptors.clone()
    }

    pub fn entry(&self, index: i32) -> Option<LocalClipboardEntry> {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.entries.get(index))
            .cloned()
    }
}

impl fmt::Debug for LocalClipboardTransfer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalClipboardTransfer")
            .field("transfer_id", &self.transfer_id)
            .field("entry_count", &self.entries.len())
            .finish()
    }
}

#[derive(Clone)]
pub struct LocalClipboardEntry {
    source_path: PathBuf,
    canonical_path: PathBuf,
    relative_path: RelativeClipboardPath,
    kind: LocalClipboardEntryKind,
    size: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LocalClipboardEntryKind {
    File,
    Directory,
}

impl LocalClipboardEntry {
    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let component = utf8_file_name(path)?.to_string();
        build_entry(
            path.to_path_buf(),
            RelativeClipboardPath::from_local_components(vec![component])?,
        )
    }

    pub fn read(
        &self,
        request: &FileContentsRequest,
    ) -> anyhow::Result<FileContentsResponse<'static>> {
        anyhow::ensure!(
            self.kind == LocalClipboardEntryKind::File,
            "clipboard directory contents cannot be streamed"
        );
        request.flags.validate().map_err(anyhow::Error::msg)?;
        if request.flags.contains(FileContentsFlags::SIZE) {
            return self.size_response(request);
        }
        self.range_response(request)
    }

    fn descriptor(&self) -> FileDescriptor {
        let attributes = match self.kind {
            LocalClipboardEntryKind::File => ClipboardFileAttributes::ARCHIVE,
            LocalClipboardEntryKind::Directory => ClipboardFileAttributes::DIRECTORY,
        };
        let mut descriptor =
            FileDescriptor::new(self.relative_path.name()).with_attributes(attributes);
        if self.kind == LocalClipboardEntryKind::File {
            descriptor = descriptor.with_file_size(self.size);
        }
        match self.relative_path.parent_wire_path() {
            Some(parent) => descriptor.with_relative_path(parent),
            None => descriptor,
        }
    }

    fn size_response(
        &self,
        request: &FileContentsRequest,
    ) -> anyhow::Result<FileContentsResponse<'static>> {
        anyhow::ensure!(
            request.position == 0 && request.requested_size == SIZE_RESPONSE_BYTES,
            "invalid clipboard file size request"
        );
        self.open_verified()?;
        Ok(FileContentsResponse::new_size_response(
            request.stream_id,
            self.size,
        ))
    }

    fn range_response(
        &self,
        request: &FileContentsRequest,
    ) -> anyhow::Result<FileContentsResponse<'static>> {
        anyhow::ensure!(request.requested_size > 0, "clipboard file range is empty");
        anyhow::ensure!(
            request.position <= self.size,
            "clipboard range starts past end"
        );
        let mut file = self.open_verified()?;
        let amount = u64::from(request.requested_size)
            .min(MAX_LOCAL_RESPONSE_CHUNK_BYTES)
            .min(self.size - request.position);
        let mut data = vec![0; usize::try_from(amount)?];
        file.seek(SeekFrom::Start(request.position))?;
        file.read_exact(&mut data)?;
        Ok(FileContentsResponse::new_data_response(
            request.stream_id,
            data,
        ))
    }

    fn open_verified(&self) -> anyhow::Result<std::fs::File> {
        let metadata = ensure_not_link_or_reparse(&self.source_path)?;
        anyhow::ensure!(metadata.is_file(), "clipboard source is no longer a file");
        anyhow::ensure!(metadata.len() == self.size, "clipboard source size changed");
        let canonical = std::fs::canonicalize(&self.source_path)
            .context("clipboard source path cannot be canonicalized")?;
        anyhow::ensure!(
            canonical == self.canonical_path,
            "clipboard source path changed"
        );
        let file = std::fs::File::open(&self.source_path)?;
        anyhow::ensure!(
            file.metadata()?.len() == self.size,
            "clipboard source size changed"
        );
        Ok(file)
    }
}

impl fmt::Debug for LocalClipboardEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalClipboardEntry")
            .field("kind", &self.kind)
            .field("size", &self.size)
            .finish()
    }
}

fn collect_entries(paths: &[PathBuf]) -> anyhow::Result<Vec<LocalClipboardEntry>> {
    let mut entries = Vec::new();
    let mut total_bytes = 0_u64;
    for top_level in paths {
        let name = utf8_file_name(top_level)?.to_string();
        let relative = RelativeClipboardPath::from_local_components(vec![name])?;
        collect_tree(top_level, relative, &mut entries, &mut total_bytes)?;
    }
    Ok(entries)
}

fn collect_tree(
    root: &Path,
    relative: RelativeClipboardPath,
    entries: &mut Vec<LocalClipboardEntry>,
    total_bytes: &mut u64,
) -> anyhow::Result<()> {
    ensure_not_link_or_reparse(root)?;
    let canonical_root =
        std::fs::canonicalize(root).context("clipboard source cannot be canonicalized")?;
    let mut pending = vec![(root.to_path_buf(), relative)];
    while let Some((path, relative)) = pending.pop() {
        anyhow::ensure!(
            entries.len() < MAX_CLIPBOARD_ENTRY_COUNT,
            "clipboard entry count exceeds the limit"
        );
        let entry = build_entry(path.clone(), relative.clone())?;
        account_file(&entry, total_bytes)?;
        let directory = entry.kind == LocalClipboardEntryKind::Directory;
        entries.push(entry);
        if directory {
            push_children(&canonical_root, &path, &relative, &mut pending)?;
        }
    }
    Ok(())
}

fn build_entry(
    path: PathBuf,
    relative_path: RelativeClipboardPath,
) -> anyhow::Result<LocalClipboardEntry> {
    let metadata = ensure_not_link_or_reparse(&path)?;
    let kind = if metadata.is_file() {
        LocalClipboardEntryKind::File
    } else if metadata.is_dir() {
        LocalClipboardEntryKind::Directory
    } else {
        anyhow::bail!("clipboard source has an unsupported file type");
    };
    let canonical_path =
        std::fs::canonicalize(&path).context("clipboard source cannot be canonicalized")?;
    Ok(LocalClipboardEntry {
        source_path: path,
        canonical_path,
        relative_path,
        kind,
        size: metadata.len(),
    })
}

fn account_file(entry: &LocalClipboardEntry, total_bytes: &mut u64) -> anyhow::Result<()> {
    if entry.kind == LocalClipboardEntryKind::Directory {
        return Ok(());
    }
    anyhow::ensure!(
        entry.size <= MAX_SINGLE_FILE_BYTES,
        "clipboard file exceeds the single-file limit"
    );
    *total_bytes = total_bytes
        .checked_add(entry.size)
        .context("clipboard transfer size overflow")?;
    anyhow::ensure!(
        *total_bytes <= MAX_TOTAL_TRANSFER_BYTES,
        "clipboard transfer exceeds the total size limit"
    );
    Ok(())
}

fn push_children(
    canonical_root: &Path,
    directory: &Path,
    relative: &RelativeClipboardPath,
    pending: &mut Vec<(PathBuf, RelativeClipboardPath)>,
) -> anyhow::Result<()> {
    let mut children = std::fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    children.sort_by_key(std::fs::DirEntry::file_name);
    for child in children.into_iter().rev() {
        ensure_not_link_or_reparse(&child.path())?;
        let name = child
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("clipboard path has a non-UTF-8 component"))?;
        let child_relative = relative.child(name)?;
        let canonical = std::fs::canonicalize(child.path())?;
        anyhow::ensure!(
            canonical.starts_with(canonical_root),
            "clipboard source escapes its root"
        );
        pending.push((child.path(), child_relative));
    }
    Ok(())
}
