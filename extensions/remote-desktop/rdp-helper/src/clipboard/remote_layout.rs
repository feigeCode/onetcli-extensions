use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::Context as _;
use ironrdp::cliprdr::pdu::{ClipboardFileAttributes, FileDescriptor};

use super::path::{
    MAX_CLIPBOARD_ENTRY_COUNT, MAX_SINGLE_FILE_BYTES, MAX_TOTAL_TRANSFER_BYTES,
    RelativeClipboardPath,
};

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

pub struct RemoteClipboardLayout {
    pub(super) directories: Vec<PathBuf>,
    pub(super) files: Vec<RemoteFile>,
    pub(super) top_level_names: Vec<String>,
}

impl RemoteClipboardLayout {
    pub fn validate(descriptors: &[FileDescriptor]) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !descriptors.is_empty(),
            "remote clipboard file list is empty"
        );
        anyhow::ensure!(
            descriptors.len() <= MAX_CLIPBOARD_ENTRY_COUNT,
            "remote clipboard entry count exceeds the limit"
        );
        let mut layout = Self {
            directories: Vec::new(),
            files: Vec::new(),
            top_level_names: Vec::new(),
        };
        validate_descriptors(descriptors, &mut layout)?;
        Ok(layout)
    }
}

pub struct RemoteFile {
    pub(super) descriptor_index: i32,
    pub(super) relative_path: PathBuf,
    pub(super) advertised_size: Option<u64>,
}

fn validate_descriptors(
    descriptors: &[FileDescriptor],
    layout: &mut RemoteClipboardLayout,
) -> anyhow::Result<()> {
    let mut hierarchy = HashMap::<String, bool>::new();
    let mut top_levels = HashSet::new();
    let mut advertised_total = 0_u64;
    for (index, descriptor) in descriptors.iter().enumerate() {
        let path = descriptor_path(descriptor)?;
        let directory = is_directory(descriptor)?;
        validate_parent(&path, &hierarchy)?;
        anyhow::ensure!(
            hierarchy.insert(path.normalized_key(), directory).is_none(),
            "remote clipboard contains duplicate paths"
        );
        if top_levels.insert(path.top_level_key()) {
            layout
                .top_level_names
                .push(path.top_level_name().to_string());
        }
        add_to_layout(
            layout,
            descriptor,
            path,
            i32::try_from(index)?,
            directory,
            &mut advertised_total,
        )?;
    }
    Ok(())
}

fn descriptor_path(descriptor: &FileDescriptor) -> anyhow::Result<RelativeClipboardPath> {
    RelativeClipboardPath::from_wire(&descriptor.name, descriptor.relative_path.as_deref())
}

fn add_to_layout(
    layout: &mut RemoteClipboardLayout,
    descriptor: &FileDescriptor,
    path: RelativeClipboardPath,
    descriptor_index: i32,
    directory: bool,
    advertised_total: &mut u64,
) -> anyhow::Result<()> {
    if directory {
        layout.directories.push(path.as_path_buf());
        return Ok(());
    }
    account_advertised_size(descriptor.file_size, advertised_total)?;
    layout.files.push(RemoteFile {
        descriptor_index,
        relative_path: path.as_path_buf(),
        advertised_size: descriptor.file_size,
    });
    Ok(())
}

fn is_directory(descriptor: &FileDescriptor) -> anyhow::Result<bool> {
    let attributes = descriptor
        .attributes
        .unwrap_or_else(ClipboardFileAttributes::empty);
    anyhow::ensure!(
        attributes.bits() & FILE_ATTRIBUTE_REPARSE_POINT == 0,
        "remote clipboard contains a reparse point"
    );
    Ok(attributes.contains(ClipboardFileAttributes::DIRECTORY))
}

fn validate_parent(
    path: &RelativeClipboardPath,
    hierarchy: &HashMap<String, bool>,
) -> anyhow::Result<()> {
    let Some(parent) = path.parent_key() else {
        return Ok(());
    };
    anyhow::ensure!(
        hierarchy.get(&parent).copied() == Some(true),
        "remote clipboard hierarchy has a missing parent directory"
    );
    Ok(())
}

fn account_advertised_size(size: Option<u64>, total: &mut u64) -> anyhow::Result<()> {
    let Some(size) = size else {
        return Ok(());
    };
    anyhow::ensure!(
        size <= MAX_SINGLE_FILE_BYTES,
        "remote clipboard file exceeds the single-file limit"
    );
    *total = total
        .checked_add(size)
        .context("remote clipboard transfer size overflow")?;
    anyhow::ensure!(
        *total <= MAX_TOTAL_TRANSFER_BYTES,
        "remote clipboard transfer exceeds the total size limit"
    );
    Ok(())
}
