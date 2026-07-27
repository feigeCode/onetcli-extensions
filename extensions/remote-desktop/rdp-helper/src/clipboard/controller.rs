use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use ironrdp::cliprdr::backend::ClipboardMessage;
use ironrdp_client::rdp::RdpInputEvent;
use tokio::sync::mpsc;

use crate::output_mailbox::OutputSender;
use crate::protocol::HelperEvent;

use super::local::{LocalClipboardEntry, LocalClipboardTransfer};
use super::remote::RemoteClipboardTransfer;
use super::{FIRST_SEQUENCE_ID, LOCAL_CLIPBOARD_TRANSFER_MASK, REMOTE_CLIPBOARD_TRANSFER_BIT};

#[derive(Clone)]
pub struct TextClipboardController {
    pub(super) shared: Arc<Mutex<TextClipboardState>>,
    pub(super) input_tx: mpsc::UnboundedSender<RdpInputEvent>,
    pub(super) output_tx: OutputSender,
}

impl TextClipboardController {
    pub fn set_local_text(&self, text: String) -> anyhow::Result<()> {
        let mut state = lock_state(&self.shared);
        state.local_text = Some(text.clone());
        state.local_files = None;
        drop(state);
        self.send_clipboard(ClipboardMessage::SendInitiateCopy(super::text_formats()))
    }

    pub fn set_local_files(&self, transfer_id: u64, paths: Vec<String>) -> anyhow::Result<()> {
        anyhow::ensure!(
            transfer_id & REMOTE_CLIPBOARD_TRANSFER_BIT == 0,
            "local clipboard transfer ID uses the remote namespace"
        );
        let transfer = LocalClipboardTransfer::collect(transfer_id, paths)?;
        let descriptors = transfer.descriptors();
        let mut state = lock_state(&self.shared);
        state.local_text = None;
        state.local_files = Some(transfer);
        drop(state);
        self.input_tx
            .send(RdpInputEvent::ClipboardFileCopy(descriptors))
            .map_err(|_| anyhow::anyhow!("RDP input channel closed"))
    }

    pub fn cancel_transfer(&self, transfer_id: u64) -> bool {
        let mut state = lock_state(&self.shared);
        let mut cancelled = state.cancel_local_transfer(transfer_id);
        if state.pending_remote == Some(transfer_id) {
            state.pending_remote = None;
            cancelled = true;
        }
        if state
            .remote_transfer
            .as_ref()
            .is_some_and(|transfer| transfer.transfer_id() == transfer_id)
        {
            state.remote_transfer = None;
            cancelled = true;
        }
        cancelled
    }

    pub fn report_transfer_failure(&self, transfer_id: u64, message: &str) {
        let _ = self.output_tx.send(HelperEvent::ClipboardTransferFailed {
            transfer_id,
            message: message.to_string(),
        });
    }

    pub fn shutdown(&self) {
        let mut state = lock_state(&self.shared);
        state.local_text = None;
        state.local_files = None;
        state.pending_remote = None;
        state.remote_transfer = None;
        state.waiting_remote_text = false;
        state.locked_local_files.clear();
    }

    fn send_clipboard(&self, message: ClipboardMessage) -> anyhow::Result<()> {
        self.input_tx
            .send(RdpInputEvent::Clipboard(message))
            .map_err(|_| anyhow::anyhow!("RDP input channel closed"))
    }
}

impl fmt::Debug for TextClipboardController {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("TextClipboardController").finish()
    }
}

pub(super) struct TextClipboardState {
    pub(super) local_text: Option<String>,
    pub(super) local_files: Option<LocalClipboardTransfer>,
    pub(super) locked_local_files: HashMap<u32, LocalClipboardTransfer>,
    pub(super) pending_remote: Option<u64>,
    pub(super) remote_transfer: Option<RemoteClipboardTransfer>,
    pub(super) waiting_remote_text: bool,
    pub(super) next_remote_sequence: u64,
    pub(super) next_stream_id: u32,
}

impl TextClipboardState {
    pub(super) fn new() -> Self {
        Self {
            local_text: None,
            local_files: None,
            locked_local_files: HashMap::new(),
            pending_remote: None,
            remote_transfer: None,
            waiting_remote_text: false,
            next_remote_sequence: FIRST_SEQUENCE_ID,
            next_stream_id: 1,
        }
    }

    pub(super) fn reserve_remote_transfer(&mut self) -> u64 {
        self.remote_transfer = None;
        let transfer_id = REMOTE_CLIPBOARD_TRANSFER_BIT | self.next_remote_sequence;
        self.next_remote_sequence = (self.next_remote_sequence + 1) & LOCAL_CLIPBOARD_TRANSFER_MASK;
        if self.next_remote_sequence == 0 {
            self.next_remote_sequence = FIRST_SEQUENCE_ID;
        }
        self.pending_remote = Some(transfer_id);
        transfer_id
    }

    pub(super) fn allocate_stream_id(&mut self) -> u32 {
        let stream_id = self.next_stream_id;
        self.next_stream_id = self.next_stream_id.wrapping_add(1);
        if self.next_stream_id == 0 {
            self.next_stream_id = 1;
        }
        stream_id
    }

    pub(super) fn cancel_local_transfer(&mut self, transfer_id: u64) -> bool {
        let current_matches = self
            .local_files
            .as_ref()
            .is_some_and(|transfer| transfer.transfer_id() == transfer_id);
        if current_matches {
            self.local_files = None;
        }
        let previous_count = self.locked_local_files.len();
        self.locked_local_files
            .retain(|_, transfer| transfer.transfer_id() != transfer_id);
        current_matches || previous_count != self.locked_local_files.len()
    }

    pub(super) fn local_entry(
        &self,
        request: &ironrdp::cliprdr::pdu::FileContentsRequest,
    ) -> Option<LocalClipboardEntry> {
        let transfer = match request.data_id {
            Some(data_id) => self.locked_local_files.get(&data_id),
            None => self.local_files.as_ref(),
        }?;
        transfer.entry(request.index)
    }
}

impl fmt::Debug for TextClipboardState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TextClipboardState")
            .field("has_text", &self.local_text.is_some())
            .field("has_local_files", &self.local_files.is_some())
            .field("locked_snapshot_count", &self.locked_local_files.len())
            .field("has_remote_transfer", &self.remote_transfer.is_some())
            .finish()
    }
}

pub(super) fn lock_state(
    shared: &Arc<Mutex<TextClipboardState>>,
) -> std::sync::MutexGuard<'_, TextClipboardState> {
    shared.lock().unwrap_or_else(|error| error.into_inner())
}
