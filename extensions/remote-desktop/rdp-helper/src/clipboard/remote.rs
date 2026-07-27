use std::fmt;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use ironrdp::cliprdr::pdu::{FileContentsFlags, FileContentsRequest, FileContentsResponse};

use super::path::{MAX_SINGLE_FILE_BYTES, MAX_TOTAL_TRANSFER_BYTES};
use super::remote_layout::{RemoteClipboardLayout, RemoteFile};
use super::staging::TransferDirectory;

const REMOTE_FILE_CHUNK_BYTES: u32 = 1024 * 1024;
const SIZE_REQUEST_BYTES: u32 = 8;

pub struct RemoteTransferSettings<'a> {
    pub transfer_id: u64,
    pub staging_root: &'a Path,
    pub clip_data_id: Option<u32>,
}

pub struct RemoteClipboardTransfer {
    transfer_id: u64,
    clip_data_id: Option<u32>,
    staging: TransferDirectory,
    files: Vec<RemoteFile>,
    top_level_names: Vec<String>,
    current_file: usize,
    expected_size: Option<u64>,
    offset: u64,
    accounted_bytes: u64,
    output_file: Option<std::fs::File>,
    pending: Option<PendingRemoteRequest>,
}

impl RemoteClipboardTransfer {
    pub fn create(
        layout: RemoteClipboardLayout,
        settings: RemoteTransferSettings<'_>,
    ) -> anyhow::Result<Self> {
        let staging = TransferDirectory::create(settings.staging_root, settings.transfer_id)?;
        create_directories(staging.path(), &layout.directories)?;
        Ok(Self {
            transfer_id: settings.transfer_id,
            clip_data_id: settings.clip_data_id,
            staging,
            files: layout.files,
            top_level_names: layout.top_level_names,
            current_file: 0,
            expected_size: None,
            offset: 0,
            accounted_bytes: 0,
            output_file: None,
            pending: None,
        })
    }

    pub fn transfer_id(&self) -> u64 {
        self.transfer_id
    }

    pub fn clip_data_id(&self) -> Option<u32> {
        self.clip_data_id
    }

    pub fn start(&mut self, stream_id: u32) -> anyhow::Result<RemoteTransferAction> {
        if self.files.is_empty() {
            return Ok(self.complete());
        }
        Ok(self.request_size(stream_id))
    }

    pub fn advance(
        &mut self,
        response: FileContentsResponse<'_>,
        next_stream_id: u32,
    ) -> anyhow::Result<RemoteTransferAction> {
        let Some(pending) = self.pending.as_ref() else {
            return Ok(RemoteTransferAction::Ignore);
        };
        if pending.stream_id != response.stream_id() {
            return Ok(RemoteTransferAction::Ignore);
        }
        let pending = self.pending.take().expect("pending response checked");
        anyhow::ensure!(!response.is_error(), "remote clipboard file request failed");
        match pending.kind {
            PendingRemoteRequestKind::Size => self.accept_size(response, next_stream_id),
            PendingRemoteRequestKind::Range { requested } => {
                self.accept_range(response.data(), requested, next_stream_id)
            }
        }
    }

    fn accept_size(
        &mut self,
        response: FileContentsResponse<'_>,
        next_stream_id: u32,
    ) -> anyhow::Result<RemoteTransferAction> {
        let size = response
            .data_as_size()
            .map_err(|_| anyhow::anyhow!("remote clipboard size response is invalid"))?;
        let (advertised_size, relative_path) = {
            let file = self.current_remote_file()?;
            (file.advertised_size, file.relative_path.clone())
        };
        if let Some(advertised) = advertised_size {
            anyhow::ensure!(advertised == size, "remote clipboard file size changed");
        }
        self.account_size(size)?;
        self.output_file = Some(create_output_file(self.staging.path(), &relative_path)?);
        self.expected_size = Some(size);
        self.offset = 0;
        if size == 0 {
            return self.finish_file(next_stream_id);
        }
        Ok(self.request_range(next_stream_id))
    }

    fn accept_range(
        &mut self,
        data: &[u8],
        requested: u32,
        next_stream_id: u32,
    ) -> anyhow::Result<RemoteTransferAction> {
        let expected = self
            .expected_size
            .context("remote clipboard file size is missing")?;
        anyhow::ensure!(
            !data.is_empty(),
            "remote clipboard returned an empty file chunk"
        );
        anyhow::ensure!(
            data.len() <= requested as usize,
            "remote clipboard returned an oversized file chunk"
        );
        let next_offset = self
            .offset
            .checked_add(u64::try_from(data.len())?)
            .context("remote clipboard file offset overflow")?;
        anyhow::ensure!(
            next_offset <= expected,
            "remote clipboard file exceeds its size"
        );
        self.output_file
            .as_mut()
            .context("remote clipboard staging file is missing")?
            .write_all(data)?;
        self.offset = next_offset;
        if self.offset == expected {
            return self.finish_file(next_stream_id);
        }
        Ok(self.request_range(next_stream_id))
    }

    fn finish_file(&mut self, next_stream_id: u32) -> anyhow::Result<RemoteTransferAction> {
        if let Some(mut file) = self.output_file.take() {
            file.flush()?;
        }
        self.current_file += 1;
        self.expected_size = None;
        self.offset = 0;
        if self.current_file == self.files.len() {
            return Ok(self.complete());
        }
        Ok(self.request_size(next_stream_id))
    }

    fn request_size(&mut self, stream_id: u32) -> RemoteTransferAction {
        self.pending = Some(PendingRemoteRequest {
            stream_id,
            kind: PendingRemoteRequestKind::Size,
        });
        RemoteTransferAction::Request(FileContentsRequest {
            stream_id,
            index: self.files[self.current_file].descriptor_index,
            flags: FileContentsFlags::SIZE,
            position: 0,
            requested_size: SIZE_REQUEST_BYTES,
            data_id: self.clip_data_id,
        })
    }

    fn request_range(&mut self, stream_id: u32) -> RemoteTransferAction {
        let remaining = self.expected_size.unwrap_or_default() - self.offset;
        let requested = u32::try_from(remaining.min(u64::from(REMOTE_FILE_CHUNK_BYTES)))
            .unwrap_or(REMOTE_FILE_CHUNK_BYTES);
        self.pending = Some(PendingRemoteRequest {
            stream_id,
            kind: PendingRemoteRequestKind::Range { requested },
        });
        RemoteTransferAction::Request(FileContentsRequest {
            stream_id,
            index: self.files[self.current_file].descriptor_index,
            flags: FileContentsFlags::RANGE,
            position: self.offset,
            requested_size: requested,
            data_id: self.clip_data_id,
        })
    }

    fn current_remote_file(&self) -> anyhow::Result<&RemoteFile> {
        self.files
            .get(self.current_file)
            .context("remote clipboard file index is invalid")
    }

    fn account_size(&mut self, size: u64) -> anyhow::Result<()> {
        anyhow::ensure!(
            size <= MAX_SINGLE_FILE_BYTES,
            "remote clipboard file exceeds the single-file limit"
        );
        self.accounted_bytes = self
            .accounted_bytes
            .checked_add(size)
            .context("remote clipboard transfer size overflow")?;
        anyhow::ensure!(
            self.accounted_bytes <= MAX_TOTAL_TRANSFER_BYTES,
            "remote clipboard transfer exceeds the total size limit"
        );
        Ok(())
    }

    fn complete(&mut self) -> RemoteTransferAction {
        self.staging.retain();
        let paths = self
            .top_level_names
            .iter()
            .map(|name| {
                self.staging
                    .path()
                    .join(name)
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        RemoteTransferAction::Ready(paths)
    }
}

impl fmt::Debug for RemoteClipboardTransfer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RemoteClipboardTransfer")
            .field("transfer_id", &self.transfer_id)
            .field("file_count", &self.files.len())
            .field("current_file", &self.current_file)
            .field("offset", &self.offset)
            .finish()
    }
}

pub enum RemoteTransferAction {
    Request(FileContentsRequest),
    Ready(Vec<String>),
    Ignore,
}

struct PendingRemoteRequest {
    stream_id: u32,
    kind: PendingRemoteRequestKind,
}

enum PendingRemoteRequestKind {
    Size,
    Range { requested: u32 },
}

fn create_directories(root: &Path, directories: &[PathBuf]) -> anyhow::Result<()> {
    for directory in directories {
        std::fs::create_dir(root.join(directory))
            .context("remote clipboard staging directory could not be created")?;
    }
    Ok(())
}

fn create_output_file(root: &Path, relative_path: &Path) -> anyhow::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(root.join(relative_path))
        .context("remote clipboard staging file could not be created")
}
