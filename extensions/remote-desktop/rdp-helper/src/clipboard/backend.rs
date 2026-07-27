use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ironrdp::cliprdr::backend::{ClipboardMessage, CliprdrBackend, CliprdrBackendFactory};
use ironrdp::cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardFormatName, ClipboardGeneralCapabilityFlags,
    FileContentsRequest, FileContentsResponse, FormatDataRequest, FormatDataResponse, LockDataId,
};
use ironrdp::core::{AsAny, IntoOwned};
use ironrdp_client::rdp::RdpInputEvent;

use crate::output_mailbox::OutputSender;
use crate::protocol::HelperEvent;

use super::controller::{TextClipboardState, lock_state};
use super::staging::cleanup_stale_transfers;

#[derive(Clone)]
pub(super) struct TextClipboardBackendFactory {
    shared: Arc<Mutex<TextClipboardState>>,
    input_tx: tokio::sync::mpsc::UnboundedSender<RdpInputEvent>,
    output_tx: OutputSender,
    staging_root: PathBuf,
}

impl TextClipboardBackendFactory {
    pub(super) fn new(
        shared: Arc<Mutex<TextClipboardState>>,
        input_tx: tokio::sync::mpsc::UnboundedSender<RdpInputEvent>,
        output_tx: OutputSender,
        staging_root: PathBuf,
    ) -> Self {
        cleanup_stale_transfers(&staging_root);
        Self {
            shared,
            input_tx,
            output_tx,
            staging_root,
        }
    }
}

impl CliprdrBackendFactory for TextClipboardBackendFactory {
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend> {
        let mut state = lock_state(&self.shared);
        state.pending_remote = None;
        state.remote_transfer = None;
        state.waiting_remote_text = false;
        state.locked_local_files.clear();
        drop(state);
        Box::new(TextClipboardBackend {
            shared: self.shared.clone(),
            input_tx: self.input_tx.clone(),
            output_tx: self.output_tx.clone(),
            temporary_directory: self.staging_root.to_string_lossy().into_owned(),
            staging_root: self.staging_root.clone(),
        })
    }
}

impl fmt::Debug for TextClipboardBackendFactory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TextClipboardBackendFactory")
            .finish()
    }
}

pub(super) struct TextClipboardBackend {
    pub(super) shared: Arc<Mutex<TextClipboardState>>,
    pub(super) input_tx: tokio::sync::mpsc::UnboundedSender<RdpInputEvent>,
    pub(super) output_tx: OutputSender,
    pub(super) temporary_directory: String,
    pub(super) staging_root: PathBuf,
}

impl AsAny for TextClipboardBackend {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }
}

impl fmt::Debug for TextClipboardBackend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("TextClipboardBackend").finish()
    }
}

impl TextClipboardBackend {
    pub(super) fn send_clipboard(&self, message: ClipboardMessage) {
        let _ = self.input_tx.send(RdpInputEvent::Clipboard(message));
    }

    fn send_local_text_response(&self, request: FormatDataRequest) {
        let response = if request.format == ClipboardFormatId::CF_UNICODETEXT {
            lock_state(&self.shared)
                .local_text
                .as_deref()
                .map(FormatDataResponse::new_unicode_string)
                .unwrap_or_else(FormatDataResponse::new_error)
        } else {
            FormatDataResponse::new_error()
        };
        self.send_clipboard(ClipboardMessage::SendFormatData(response.into_owned()));
    }

    fn send_local_file_response(&self, request: FileContentsRequest) {
        let entry = lock_state(&self.shared).local_entry(&request);
        let response = entry
            .and_then(|entry| entry.read(&request).ok())
            .unwrap_or_else(|| FileContentsResponse::new_error(request.stream_id));
        self.send_clipboard(ClipboardMessage::SendFileContentsResponse(response));
    }

    fn begin_remote_copy(&self, available_formats: &[ClipboardFormat]) {
        let file_format = available_formats.iter().find(|format| {
            format
                .name()
                .is_some_and(|name| name.value() == ClipboardFormatName::FILE_LIST.value())
        });
        if let Some(format) = file_format {
            let mut state = lock_state(&self.shared);
            state.waiting_remote_text = false;
            state.reserve_remote_transfer();
            drop(state);
            self.send_clipboard(ClipboardMessage::SendInitiatePaste(format.id()));
            return;
        }
        self.begin_remote_text(available_formats);
    }

    fn begin_remote_text(&self, available_formats: &[ClipboardFormat]) {
        let unicode = available_formats
            .iter()
            .any(|format| format.id() == ClipboardFormatId::CF_UNICODETEXT);
        let mut state = lock_state(&self.shared);
        state.pending_remote = None;
        state.remote_transfer = None;
        state.waiting_remote_text = unicode;
        drop(state);
        if unicode {
            self.send_clipboard(ClipboardMessage::SendInitiatePaste(
                ClipboardFormatId::CF_UNICODETEXT,
            ));
        }
    }
}

impl CliprdrBackend for TextClipboardBackend {
    fn temporary_directory(&self) -> &str {
        &self.temporary_directory
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA
            | ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
    }

    fn on_ready(&mut self) {}

    fn on_request_format_list(&mut self) {
        let state = lock_state(&self.shared);
        if let Some(transfer) = &state.local_files {
            let descriptors = transfer.descriptors();
            drop(state);
            let _ = self
                .input_tx
                .send(RdpInputEvent::ClipboardFileCopy(descriptors));
        } else if state.local_text.is_some() {
            self.send_clipboard(ClipboardMessage::SendInitiateCopy(super::text_formats()));
        } else {
            self.send_clipboard(ClipboardMessage::SendInitiateCopy(Vec::new()));
        }
    }

    fn on_process_negotiated_capabilities(&mut self, _: ClipboardGeneralCapabilityFlags) {}

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        self.begin_remote_copy(available_formats);
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        self.send_local_text_response(request);
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        let waiting = {
            let mut state = lock_state(&self.shared);
            std::mem::take(&mut state.waiting_remote_text)
        };
        if !waiting || response.is_error() {
            return;
        }
        if let Ok(text) = response.to_unicode_string() {
            let _ = self.output_tx.send(HelperEvent::ClipboardText { text });
        }
    }

    fn on_file_contents_request(&mut self, request: FileContentsRequest) {
        self.send_local_file_response(request);
    }

    fn on_file_contents_response(&mut self, response: FileContentsResponse<'_>) {
        self.advance_remote_transfer(response);
    }

    fn on_lock(&mut self, data_id: LockDataId) {
        let mut state = lock_state(&self.shared);
        if let Some(transfer) = state.local_files.clone() {
            state.locked_local_files.insert(data_id.0, transfer);
        }
    }

    fn on_unlock(&mut self, data_id: LockDataId) {
        lock_state(&self.shared)
            .locked_local_files
            .remove(&data_id.0);
    }

    fn on_remote_file_list(
        &mut self,
        files: &[ironrdp::cliprdr::pdu::FileDescriptor],
        clip_data_id: Option<u32>,
    ) {
        self.receive_remote_files(files, clip_data_id);
    }

    fn on_outgoing_locks_cleared(&mut self, clip_data_ids: &[LockDataId]) {
        self.cancel_remote_locks(clip_data_ids);
    }

    fn on_outgoing_locks_expired(&mut self, clip_data_ids: &[LockDataId]) {
        self.cancel_remote_locks(clip_data_ids);
    }
}
