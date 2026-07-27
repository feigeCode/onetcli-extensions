use vnc_client::{ClientMouseEvent, VncClient, X11Event};

use crate::runtime::{RemoteDesktopInput, RemoteMouseButton};
use crate::vnc_keyboard::{VncKeyboardState, remote_key_to_keysym};

const MAX_INPUTS_PER_POLL: usize = 256;

pub(crate) enum VncInputAction {
    Continue,
    Closed,
    InputClosed,
    Reconnect,
    Failed(String),
}

pub(crate) async fn handle_pending_inputs(
    client: &VncClient,
    latest_clipboard_text: &mut Option<String>,
    input_rx: &mut tokio::sync::mpsc::UnboundedReceiver<RemoteDesktopInput>,
    keyboard: &mut VncKeyboardState,
    pointer: &mut VncPointerState,
) -> VncInputAction {
    let inputs = match drain_remote_inputs(input_rx) {
        VncInputBatch::Inputs(inputs) => inputs,
        VncInputBatch::Disconnected => return VncInputAction::InputClosed,
    };
    for input in inputs {
        let action =
            handle_vnc_input(client, latest_clipboard_text, keyboard, pointer, input).await;
        if !matches!(action, VncInputAction::Continue) {
            return action;
        }
    }
    VncInputAction::Continue
}

async fn handle_vnc_input(
    client: &VncClient,
    latest_clipboard_text: &mut Option<String>,
    keyboard: &mut VncKeyboardState,
    pointer: &mut VncPointerState,
    input: RemoteDesktopInput,
) -> VncInputAction {
    match send_vnc_input(client, latest_clipboard_text, keyboard, pointer, input).await {
        Ok(action) => action,
        Err(error) => VncInputAction::Failed(error.to_string()),
    }
}

async fn send_vnc_input(
    client: &VncClient,
    latest_clipboard_text: &mut Option<String>,
    keyboard: &mut VncKeyboardState,
    pointer: &mut VncPointerState,
    input: RemoteDesktopInput,
) -> anyhow::Result<VncInputAction> {
    match input {
        RemoteDesktopInput::Close => Ok(VncInputAction::Closed),
        RemoteDesktopInput::Reconnect => Ok(VncInputAction::Reconnect),
        RemoteDesktopInput::MouseMove { x, y } => move_pointer(client, pointer, x, y).await,
        RemoteDesktopInput::MouseButton { button, pressed } => {
            update_button(client, pointer, button, pressed).await
        }
        RemoteDesktopInput::Wheel { vertical, units } => {
            send_wheel_events(client, pointer, vertical, units).await
        }
        RemoteDesktopInput::Key { key, pressed } => {
            if let Some(keysym) = remote_key_to_keysym(&key) {
                keyboard.send(client, keysym, pressed).await?;
            }
            Ok(VncInputAction::Continue)
        }
        RemoteDesktopInput::ClipboardText { text } | RemoteDesktopInput::Text { text } => {
            send_clipboard_text(client, latest_clipboard_text, text).await
        }
        RemoteDesktopInput::ClipboardFiles { .. } => Ok(VncInputAction::Continue),
        RemoteDesktopInput::Resize { .. } => Ok(VncInputAction::Continue),
    }
}

async fn move_pointer(
    client: &VncClient,
    pointer: &mut VncPointerState,
    x: u16,
    y: u16,
) -> anyhow::Result<VncInputAction> {
    let mut next = *pointer;
    next.move_to(x, y);
    send_pointer_event(client, &next).await?;
    *pointer = next;
    Ok(VncInputAction::Continue)
}

async fn update_button(
    client: &VncClient,
    pointer: &mut VncPointerState,
    button: RemoteMouseButton,
    pressed: bool,
) -> anyhow::Result<VncInputAction> {
    let mut next = *pointer;
    next.set_button(button, pressed);
    send_pointer_event(client, &next).await?;
    *pointer = next;
    Ok(VncInputAction::Continue)
}

async fn send_pointer_event(client: &VncClient, pointer: &VncPointerState) -> anyhow::Result<()> {
    let (x, y, mask) = pointer.snapshot();
    client
        .input(X11Event::PointerEvent(ClientMouseEvent::from((x, y, mask))))
        .await?;
    Ok(())
}

async fn send_wheel_events(
    client: &VncClient,
    pointer: &VncPointerState,
    vertical: bool,
    units: i16,
) -> anyhow::Result<VncInputAction> {
    let (x, y, _) = pointer.snapshot();
    for mask in pointer.wheel_masks(vertical, units) {
        client
            .input(X11Event::PointerEvent(ClientMouseEvent::from((x, y, mask))))
            .await?;
    }
    Ok(VncInputAction::Continue)
}

async fn send_clipboard_text(
    client: &VncClient,
    latest_clipboard_text: &mut Option<String>,
    text: String,
) -> anyhow::Result<VncInputAction> {
    let Some(text) = supported_clipboard_text(text) else {
        return Ok(VncInputAction::Continue);
    };
    *latest_clipboard_text = Some(text.clone());
    client.input(X11Event::CopyText(text)).await?;
    Ok(VncInputAction::Continue)
}

pub(crate) fn supported_clipboard_text(text: String) -> Option<String> {
    if text.is_ascii() {
        Some(text)
    } else {
        tracing::debug!("ignoring non-ASCII VNC clipboard text");
        None
    }
}

enum VncInputBatch {
    Inputs(Vec<RemoteDesktopInput>),
    Disconnected,
}

fn drain_remote_inputs(
    input_rx: &mut tokio::sync::mpsc::UnboundedReceiver<RemoteDesktopInput>,
) -> VncInputBatch {
    let mut inputs = Vec::new();
    for _ in 0..MAX_INPUTS_PER_POLL {
        match input_rx.try_recv() {
            Ok(input) => inputs.push(input),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                return VncInputBatch::Disconnected;
            }
        }
    }
    VncInputBatch::Inputs(coalesce_remote_inputs(inputs))
}

fn coalesce_remote_inputs<I>(inputs: I) -> Vec<RemoteDesktopInput>
where
    I: IntoIterator<Item = RemoteDesktopInput>,
{
    let mut coalesced = Vec::new();
    let mut pending_mouse_move = None;
    for input in inputs {
        match input {
            RemoteDesktopInput::MouseMove { .. } => pending_mouse_move = Some(input),
            input => {
                if let Some(mouse_move) = pending_mouse_move.take() {
                    coalesced.push(mouse_move);
                }
                coalesced.push(input);
            }
        }
    }
    if let Some(mouse_move) = pending_mouse_move {
        coalesced.push(mouse_move);
    }
    coalesced
}

#[derive(Default, Clone, Copy)]
pub(crate) struct VncPointerState {
    x: u16,
    y: u16,
    buttons: u8,
}

impl VncPointerState {
    fn move_to(&mut self, x: u16, y: u16) -> u8 {
        self.x = x;
        self.y = y;
        self.buttons
    }

    fn set_button(&mut self, button: RemoteMouseButton, pressed: bool) -> u8 {
        let Some(bit) = vnc_button_bit(button) else {
            return self.buttons;
        };
        if pressed {
            self.buttons |= bit;
        } else {
            self.buttons &= !bit;
        }
        self.buttons
    }

    fn wheel_masks(&self, vertical: bool, units: i16) -> Vec<u8> {
        let Some(bit) = vnc_wheel_bit(vertical, units) else {
            return Vec::new();
        };
        vec![self.buttons | bit, self.buttons]
    }

    fn snapshot(&self) -> (u16, u16, u8) {
        (self.x, self.y, self.buttons)
    }

    pub(crate) async fn release_buttons(&mut self, client: &VncClient) -> anyhow::Result<()> {
        if self.buttons == 0 {
            return Ok(());
        }
        let snapshot = (self.x, self.y, 0);
        let result = client
            .input(X11Event::PointerEvent(ClientMouseEvent::from(snapshot)))
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()));
        self.buttons = 0;
        result.map_err(|error| anyhow::anyhow!("VNC mouse release failed: {error}"))
    }
}

pub(crate) async fn shutdown_inputs(
    client: &VncClient,
    keyboard: &mut VncKeyboardState,
    pointer: &mut VncPointerState,
) -> anyhow::Result<()> {
    let mut first_error = keyboard.release_all(client).await.err();
    if let Err(error) = pointer.release_buttons(client).await {
        if first_error.is_none() {
            first_error = Some(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn vnc_button_bit(button: RemoteMouseButton) -> Option<u8> {
    match button {
        RemoteMouseButton::Left => Some(1),
        RemoteMouseButton::Middle => Some(2),
        RemoteMouseButton::Right => Some(4),
        RemoteMouseButton::X1 => Some(128),
        RemoteMouseButton::X2 => None,
    }
}

fn vnc_wheel_bit(vertical: bool, units: i16) -> Option<u8> {
    match (vertical, units.signum()) {
        (true, -1) => Some(8),
        (true, 1) => Some(16),
        (false, -1) => Some(32),
        (false, 1) => Some(64),
        _ => None,
    }
}

#[cfg(test)]
#[path = "vnc_input_tests.rs"]
mod tests;
