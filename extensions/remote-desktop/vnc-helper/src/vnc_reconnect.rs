use std::time::{Duration, Instant};

use crate::output_mailbox::OutputSender;
use crate::runtime::{
    RemoteDesktopInput, RemoteDesktopOutput, RemoteDesktopReconnect, RemoteDesktopReconnectReason,
};
use crate::vnc_clipboard::VncClipboardSnapshot;
use crate::vnc_input::VncInputAction;

pub(super) const VNC_POLL_INTERVAL: Duration = Duration::from_millis(8);
const MAX_RECONNECT_WAIT_INPUTS_PER_POLL: usize = 256;

pub(super) enum VncSessionResult {
    Closed,
    InputClosed,
    Reconnect {
        reason: RemoteDesktopReconnectReason,
        diagnostic: String,
        manual: bool,
        was_connected: bool,
    },
}

pub(super) fn session_result_from_action(
    action: VncInputAction,
    was_connected: bool,
) -> Option<VncSessionResult> {
    match action {
        VncInputAction::Continue => None,
        VncInputAction::Closed => Some(VncSessionResult::Closed),
        VncInputAction::InputClosed => Some(VncSessionResult::InputClosed),
        VncInputAction::Reconnect => Some(reconnect_result(
            RemoteDesktopReconnectReason::Manual,
            "manual reconnect".to_string(),
            true,
            was_connected,
        )),
        VncInputAction::Failed(diagnostic) => Some(reconnect_result(
            reconnect_reason(was_connected),
            diagnostic,
            false,
            was_connected,
        )),
    }
}

pub(super) fn reconnect_delay(attempt: usize) -> Duration {
    match attempt {
        0 => Duration::from_secs(1),
        1 => Duration::from_secs(2),
        2 => Duration::from_secs(5),
        _ => Duration::from_secs(10),
    }
}

pub(super) fn reconnect_reason(was_connected: bool) -> RemoteDesktopReconnectReason {
    if was_connected {
        RemoteDesktopReconnectReason::ConnectionLost
    } else {
        RemoteDesktopReconnectReason::SessionError
    }
}

pub(super) fn reconnect_result(
    reason: RemoteDesktopReconnectReason,
    diagnostic: String,
    manual: bool,
    was_connected: bool,
) -> VncSessionResult {
    VncSessionResult::Reconnect {
        reason,
        diagnostic,
        manual,
        was_connected,
    }
}

pub(super) fn merge_cleanup_error(
    result: VncSessionResult,
    error: anyhow::Error,
) -> VncSessionResult {
    match result {
        VncSessionResult::Reconnect {
            reason,
            diagnostic,
            manual,
            was_connected,
        } => reconnect_result(
            reason,
            format!("{diagnostic}; cleanup failed: {error:#}"),
            manual,
            was_connected,
        ),
        VncSessionResult::Closed => {
            tracing::warn!(%error, "VNC cleanup failed after close");
            VncSessionResult::Closed
        }
        VncSessionResult::InputClosed => {
            tracing::warn!(%error, "VNC cleanup failed after input channel closed");
            VncSessionResult::InputClosed
        }
    }
}

pub(super) async fn wait_before_reconnect(
    input_rx: &mut tokio::sync::mpsc::UnboundedReceiver<RemoteDesktopInput>,
    latest_clipboard: &mut Option<VncClipboardSnapshot>,
    delay: Duration,
) -> bool {
    let deadline = Instant::now() + delay;
    loop {
        match handle_wait_input(input_rx, latest_clipboard) {
            WaitAction::Continue => {}
            WaitAction::ReconnectNow => return true,
            WaitAction::Stop => return false,
        }
        if Instant::now() >= deadline {
            return true;
        }
        tokio::time::sleep(VNC_POLL_INTERVAL).await;
    }
}

enum WaitAction {
    Continue,
    ReconnectNow,
    Stop,
}

fn handle_wait_input(
    input_rx: &mut tokio::sync::mpsc::UnboundedReceiver<RemoteDesktopInput>,
    latest_clipboard: &mut Option<VncClipboardSnapshot>,
) -> WaitAction {
    let mut action = WaitAction::Continue;
    for _ in 0..MAX_RECONNECT_WAIT_INPUTS_PER_POLL {
        match input_rx.try_recv() {
            Ok(RemoteDesktopInput::Close) => return WaitAction::Stop,
            Ok(RemoteDesktopInput::Reconnect) => action = WaitAction::ReconnectNow,
            Ok(RemoteDesktopInput::ClipboardText { text } | RemoteDesktopInput::Text { text }) => {
                if let Some(snapshot) = VncClipboardSnapshot::encode(&text) {
                    *latest_clipboard = Some(snapshot);
                }
            }
            Ok(_) => {}
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                return WaitAction::Stop;
            }
        }
    }
    action
}

pub(super) fn send_reconnecting(
    output_tx: &OutputSender,
    reason: RemoteDesktopReconnectReason,
    delay_secs: Option<u64>,
) {
    let _ = output_tx.send(RemoteDesktopOutput::Reconnecting(RemoteDesktopReconnect {
        reason,
        delay_secs,
    }));
}

#[cfg(test)]
#[path = "vnc_reconnect_tests.rs"]
mod tests;
