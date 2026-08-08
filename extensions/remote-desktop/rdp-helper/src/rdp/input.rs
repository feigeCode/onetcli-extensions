use std::time::Duration;

use anyhow::Context as _;
use ironrdp::input::{Database, MouseButton, MousePosition, Operation, Scancode, WheelRotations};
use ironrdp_client::rdp::RdpInputEvent;
use smallvec::SmallVec;

use crate::clipboard::TextClipboardController;
use crate::protocol::{HelperMouseButton, HelperRequest};

use super::{HelperInputSender, InputQueueStatus};

type FastPathEvents = SmallVec<[ironrdp::pdu::input::fast_path::FastPathInputEvent; 2]>;

const INPUT_BACKPRESSURE_RETRY_INTERVAL: Duration = Duration::from_millis(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RdpInputAction {
    Continue,
    Close,
}

pub(crate) struct RdpInputContext<'a> {
    input_tx: &'a HelperInputSender,
    database: &'a mut Database,
    pending_mouse_position: &'a mut Option<MousePosition>,
    clipboard: &'a TextClipboardController,
}

impl<'a> RdpInputContext<'a> {
    pub(crate) fn new(
        input_tx: &'a HelperInputSender,
        database: &'a mut Database,
        pending_mouse_position: &'a mut Option<MousePosition>,
        clipboard: &'a TextClipboardController,
    ) -> Self {
        Self {
            input_tx,
            database,
            pending_mouse_position,
            clipboard,
        }
    }
}

pub(crate) fn apply_input_request(
    request: HelperRequest,
    context: &mut RdpInputContext<'_>,
) -> anyhow::Result<RdpInputAction> {
    match request {
        HelperRequest::Resize {
            width,
            height,
            scale_factor,
        } => {
            // A coalesced move uses coordinates from the previous desktop size.
            // Do not replay it after the resize in a potentially different coordinate space.
            *context.pending_mouse_position = None;
            send_resize(context.input_tx, (width, height, scale_factor))?;
        }
        HelperRequest::MouseMove { x, y } => send_mouse_move(context, MousePosition { x, y })?,
        HelperRequest::MouseButton { button, pressed } => {
            send_pointer_operation(context, mouse_button_operation(button, pressed))?
        }
        HelperRequest::Wheel { vertical, units } => send_pointer_operation(
            context,
            Operation::WheelRotations(WheelRotations {
                is_vertical: vertical,
                rotation_units: units,
            }),
        )?,
        HelperRequest::Key {
            code,
            extended,
            pressed,
        } => send_operations(context, [key_operation(code, extended, pressed)?])?,
        HelperRequest::Text { text } => send_text(context, &text)?,
        HelperRequest::ClipboardText { text } => context.clipboard.set_local_text(text)?,
        HelperRequest::ClipboardFiles { transfer_id, paths } => {
            set_local_files(context.clipboard, transfer_id, paths)
        }
        HelperRequest::CancelClipboardTransfer { transfer_id } => {
            context.clipboard.cancel_transfer(transfer_id);
        }
        HelperRequest::Close => return Ok(RdpInputAction::Close),
        HelperRequest::Connect { .. } => {
            anyhow::bail!("Connect request is only valid as the first message")
        }
    }
    Ok(RdpInputAction::Continue)
}

pub(crate) fn shutdown_inputs(
    database: &mut Database,
    input_tx: &HelperInputSender,
) -> anyhow::Result<()> {
    // Shutdown must not wait behind a full ordinary-input queue. IronRDP's graceful-close
    // signal deliberately bypasses that queue, while held inputs will also be released when
    // the RDP session closes.
    let release_result = match input_tx.try_send_with(|| fast_path_event(database.release_all())) {
        Ok(InputQueueStatus::Sent) => Ok(()),
        Ok(InputQueueStatus::Full) => {
            tracing::debug!("skipping RDP input releases because the queue is full at shutdown");
            Ok(())
        }
        Err(error) => Err(error),
    };
    let close_result = input_tx.request_graceful_close();
    release_result.and(close_result)
}

fn send_resize(
    input_tx: &HelperInputSender,
    (width, height, scale_factor): (u16, u16, u32),
) -> anyhow::Result<()> {
    send_reliably(|| {
        input_tx.try_send(RdpInputEvent::Resize {
            width,
            height,
            scale_factor,
            physical_size: None,
        })
    })
}

fn send_mouse_move(
    context: &mut RdpInputContext<'_>,
    position: MousePosition,
) -> anyhow::Result<()> {
    *context.pending_mouse_position = Some(position);
    let status = try_send_operations(context, [Operation::MouseMove(position)])?;
    match status {
        InputQueueStatus::Sent => *context.pending_mouse_position = None,
        InputQueueStatus::Full => {
            tracing::trace!(
                x = position.x,
                y = position.y,
                "coalescing RDP mouse move while input queue is full"
            );
        }
    }
    Ok(())
}

fn send_pointer_operation(
    context: &mut RdpInputContext<'_>,
    operation: Operation,
) -> anyhow::Result<()> {
    if let Some(position) = *context.pending_mouse_position {
        send_reliably(|| {
            try_send_operations(context, [Operation::MouseMove(position), operation.clone()])
        })?;
        *context.pending_mouse_position = None;
        return Ok(());
    }
    send_operations(context, [operation])
}

fn send_operations<const N: usize>(
    context: &mut RdpInputContext<'_>,
    operations: [Operation; N],
) -> anyhow::Result<()> {
    send_reliably(|| try_send_operations(context, operations.clone()))
}

fn send_text(context: &mut RdpInputContext<'_>, text: &str) -> anyhow::Result<()> {
    for character in text.chars() {
        send_operations(
            context,
            [
                Operation::UnicodeKeyPressed(character),
                Operation::UnicodeKeyReleased(character),
            ],
        )?;
    }
    Ok(())
}

fn try_send_operations<const N: usize>(
    context: &mut RdpInputContext<'_>,
    operations: [Operation; N],
) -> anyhow::Result<InputQueueStatus> {
    let input_tx = context.input_tx;
    let database = &mut *context.database;
    input_tx.try_send_with(|| fast_path_event(database.apply(operations)))
}

fn fast_path_event(events: FastPathEvents) -> Option<RdpInputEvent> {
    if events.is_empty() {
        return None;
    }
    Some(RdpInputEvent::FastPath(events))
}

fn send_reliably(
    mut try_send: impl FnMut() -> anyhow::Result<InputQueueStatus>,
) -> anyhow::Result<()> {
    loop {
        match try_send()? {
            InputQueueStatus::Sent => return Ok(()),
            InputQueueStatus::Full => std::thread::sleep(INPUT_BACKPRESSURE_RETRY_INTERVAL),
        }
    }
}

fn set_local_files(clipboard: &TextClipboardController, transfer_id: u64, paths: Vec<String>) {
    if let Err(error) = clipboard.set_local_files(transfer_id, paths) {
        tracing::warn!(?error, transfer_id, "local clipboard transfer rejected");
        clipboard.report_transfer_failure(transfer_id, "local clipboard transfer was rejected");
    }
}

fn mouse_button_operation(button: HelperMouseButton, pressed: bool) -> Operation {
    let button = match button {
        HelperMouseButton::Left => MouseButton::Left,
        HelperMouseButton::Middle => MouseButton::Middle,
        HelperMouseButton::Right => MouseButton::Right,
        HelperMouseButton::X1 => MouseButton::X1,
        HelperMouseButton::X2 => MouseButton::X2,
    };
    if pressed {
        Operation::MouseButtonPressed(button)
    } else {
        Operation::MouseButtonReleased(button)
    }
}

fn key_operation(code: u16, extended: bool, pressed: bool) -> anyhow::Result<Operation> {
    let code = u8::try_from(code).context("RDP scancode must fit in u8")?;
    let scancode = Scancode::from_u8(extended, code);
    Ok(if pressed {
        Operation::KeyPressed(scancode)
    } else {
        Operation::KeyReleased(scancode)
    })
}

#[cfg(test)]
#[path = "input_tests.rs"]
mod tests;
