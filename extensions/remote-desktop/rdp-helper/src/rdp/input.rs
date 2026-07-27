use anyhow::Context as _;
use ironrdp::input::{Database, MouseButton, MousePosition, Operation, Scancode, WheelRotations};
use ironrdp_client::rdp::RdpInputEvent;
use smallvec::SmallVec;
use tokio::sync::mpsc;

use crate::clipboard::TextClipboardController;
use crate::protocol::{HelperMouseButton, HelperRequest};

type FastPathEvents = SmallVec<[ironrdp::pdu::input::fast_path::FastPathInputEvent; 2]>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RdpInputAction {
    Continue,
    Close,
}

pub(crate) struct RdpInputContext<'a> {
    input_tx: &'a mpsc::UnboundedSender<RdpInputEvent>,
    database: &'a mut Database,
    clipboard: &'a TextClipboardController,
}

impl<'a> RdpInputContext<'a> {
    pub(crate) fn new(
        input_tx: &'a mpsc::UnboundedSender<RdpInputEvent>,
        database: &'a mut Database,
        clipboard: &'a TextClipboardController,
    ) -> Self {
        Self {
            input_tx,
            database,
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
        } => send_resize(context.input_tx, (width, height, scale_factor))?,
        HelperRequest::MouseMove { x, y } => {
            send_operations(context, [Operation::MouseMove(MousePosition { x, y })])?
        }
        HelperRequest::MouseButton { button, pressed } => {
            send_operations(context, [mouse_button_operation(button, pressed)])?
        }
        HelperRequest::Wheel { vertical, units } => send_operations(
            context,
            [Operation::WheelRotations(WheelRotations {
                is_vertical: vertical,
                rotation_units: units,
            })],
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
    input_tx: &mpsc::UnboundedSender<RdpInputEvent>,
) -> anyhow::Result<()> {
    let releases = database.release_all();
    let release_failed = send_fast_path(input_tx, releases).is_err();
    let close_failed = input_tx.send(RdpInputEvent::Close).is_err();
    anyhow::ensure!(
        !release_failed && !close_failed,
        "RDP input channel closed during shutdown"
    );
    Ok(())
}

fn send_resize(
    input_tx: &mpsc::UnboundedSender<RdpInputEvent>,
    (width, height, scale_factor): (u16, u16, u32),
) -> anyhow::Result<()> {
    input_tx
        .send(RdpInputEvent::Resize {
            width,
            height,
            scale_factor,
            physical_size: None,
        })
        .map_err(|_| anyhow::anyhow!("RDP input channel closed"))
}

fn send_operations<const N: usize>(
    context: &mut RdpInputContext<'_>,
    operations: [Operation; N],
) -> anyhow::Result<()> {
    let events = context.database.apply(operations);
    send_fast_path(context.input_tx, events)
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

fn send_fast_path(
    input_tx: &mpsc::UnboundedSender<RdpInputEvent>,
    events: FastPathEvents,
) -> anyhow::Result<()> {
    if events.is_empty() {
        return Ok(());
    }
    input_tx
        .send(RdpInputEvent::FastPath(events))
        .map_err(|_| anyhow::anyhow!("RDP input channel closed"))
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
