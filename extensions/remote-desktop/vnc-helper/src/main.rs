use std::io::{self, BufRead, Write};
use std::thread::JoinHandle;

use anyhow::Context as _;
use runtime::{
    RemoteDesktopConnectionOptions, RemoteDesktopInput, RemoteDesktopOutput,
    RemoteDesktopReconnectReason, RemoteKey, RemoteMouseButton,
};
use tracing::error;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

use crate::output_mailbox::{OutputReceiver, OutputSender, output_mailbox};
use crate::protocol::{HelperEvent, HelperMouseButton, HelperReconnectReason, HelperRequest};

mod framebuffer;
mod output_mailbox;
mod protocol;
mod runtime;
mod vnc_clipboard;
mod vnc_encoding;
mod vnc_input;
mod vnc_keyboard;
mod vnc_reconnect;
mod vnc_rfb;

fn main() {
    if let Err(error) = run() {
        error!(?error, "VNC helper failed");
        let _ = write_event(&HelperEvent::ConnectionFailure {
            message: format!("{error:#}"),
        });
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    setup_logging()?;
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let connect = read_connect_request(&mut lines)?;
    let (input_tx, input_rx) = tokio::sync::mpsc::unbounded_channel();
    let (output_tx, output_rx) = output_mailbox();
    let output_thread = spawn_output_writer(output_rx)?;
    let vnc_thread = spawn_vnc_thread(connect_options(connect), input_rx, output_tx)?;

    let mut request_error = None;
    for line in lines {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                request_error = Some(anyhow::Error::from(error));
                break;
            }
        };
        let request = match protocol::decode_request_line(&line) {
            Ok(request) => request,
            Err(error) => {
                request_error = Some(error);
                break;
            }
        };
        let stop = matches!(request, HelperRequest::Close);
        if let Some(input) = request_to_input(request)
            && input_tx.send(input).is_err()
        {
            request_error = Some(anyhow::anyhow!("VNC input channel closed"));
            break;
        }
        if stop {
            break;
        }
    }

    drop(input_tx);
    let vnc_result = join_worker(vnc_thread, "VNC session thread");
    let output_result = join_worker(output_thread, "VNC output writer");
    combine_results(request_error, vnc_result, output_result)
}

fn read_connect_request(
    lines: &mut impl Iterator<Item = io::Result<String>>,
) -> anyhow::Result<protocol::ConnectRequest> {
    let line = lines
        .next()
        .transpose()?
        .ok_or_else(|| anyhow::anyhow!("missing Connect request"))?;
    protocol::connect_request(protocol::decode_request_line(&line)?)
}

fn connect_options(connect: protocol::ConnectRequest) -> RemoteDesktopConnectionOptions {
    RemoteDesktopConnectionOptions {
        destination: connect.destination,
        password: connect.password,
    }
}

fn request_to_input(request: HelperRequest) -> Option<RemoteDesktopInput> {
    Some(match request {
        HelperRequest::Connect { .. } => return None,
        HelperRequest::Resize { width, height } => RemoteDesktopInput::Resize { width, height },
        HelperRequest::MouseMove { x, y } => RemoteDesktopInput::MouseMove { x, y },
        HelperRequest::MouseButton { button, pressed } => RemoteDesktopInput::MouseButton {
            button: mouse_button(button),
            pressed,
        },
        HelperRequest::Wheel { vertical, units } => RemoteDesktopInput::Wheel { vertical, units },
        HelperRequest::Key {
            code,
            extended,
            pressed,
        } => RemoteDesktopInput::Key {
            key: RemoteKey::Scancode(scancode_value(code, extended)),
            pressed,
        },
        HelperRequest::KeySym { keysym, pressed } => RemoteDesktopInput::Key {
            key: RemoteKey::KeySym(keysym),
            pressed,
        },
        HelperRequest::Text { text } => RemoteDesktopInput::Text { text },
        HelperRequest::ClipboardText { text } => RemoteDesktopInput::ClipboardText { text },
        HelperRequest::ClipboardFiles { paths } => RemoteDesktopInput::ClipboardFiles { paths },
        HelperRequest::Close => RemoteDesktopInput::Close,
    })
}

fn mouse_button(button: HelperMouseButton) -> RemoteMouseButton {
    match button {
        HelperMouseButton::Left => RemoteMouseButton::Left,
        HelperMouseButton::Middle => RemoteMouseButton::Middle,
        HelperMouseButton::Right => RemoteMouseButton::Right,
        HelperMouseButton::X1 => RemoteMouseButton::X1,
        HelperMouseButton::X2 => RemoteMouseButton::X2,
    }
}

fn scancode_value(code: u16, extended: bool) -> u16 {
    if extended { 0xe000 | code } else { code }
}

fn spawn_vnc_thread(
    options: RemoteDesktopConnectionOptions,
    input_rx: tokio::sync::mpsc::UnboundedReceiver<RemoteDesktopInput>,
    output_tx: OutputSender,
) -> anyhow::Result<JoinHandle<anyhow::Result<()>>> {
    Ok(std::thread::Builder::new()
        .name("navop-vnc-helper-session".to_string())
        .spawn(move || vnc_rfb::run_vnc_thread(options, input_rx, output_tx))?)
}

fn spawn_output_writer(
    output_rx: OutputReceiver,
) -> anyhow::Result<JoinHandle<anyhow::Result<()>>> {
    std::thread::Builder::new()
        .name("navop-vnc-helper-output".to_string())
        .spawn(move || {
            while let Some(output) = output_rx.recv() {
                write_event(&output_to_event(output))?;
            }
            Ok(())
        })
        .map_err(Into::into)
}

fn join_worker(handle: JoinHandle<anyhow::Result<()>>, worker_name: &str) -> anyhow::Result<()> {
    match handle.join() {
        Ok(result) => result.with_context(|| format!("{worker_name} returned an error")),
        Err(payload) => Err(anyhow::anyhow!(
            "{worker_name} panicked: {}",
            panic_message(payload)
        )),
    }
}

fn combine_results(
    request_error: Option<anyhow::Error>,
    vnc_result: anyhow::Result<()>,
    output_result: anyhow::Result<()>,
) -> anyhow::Result<()> {
    if let Some(error) = request_error {
        return Err(error.context("VNC helper request loop failed"));
    }
    vnc_result?;
    output_result?;
    Ok(())
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    "unknown panic payload".to_string()
}

fn output_to_event(output: RemoteDesktopOutput) -> HelperEvent {
    match output {
        RemoteDesktopOutput::Connected { width, height, .. } => {
            HelperEvent::Connected { width, height }
        }
        RemoteDesktopOutput::Frame {
            width,
            height,
            rgba,
        } => HelperEvent::frame(width, height, rgba),
        RemoteDesktopOutput::FrameBgraRects {
            width,
            height,
            rects,
            bgra,
        } => HelperEvent::FrameBgraRects {
            width,
            height,
            rects: rects
                .into_iter()
                .map(|rect| crate::protocol::HelperFrameRect {
                    x: rect.x,
                    y: rect.y,
                    width: rect.width,
                    height: rect.height,
                    byte_len: rect.byte_len,
                })
                .collect(),
            bgra,
        },
        RemoteDesktopOutput::CursorDefault => HelperEvent::CursorDefault,
        RemoteDesktopOutput::CursorHidden => HelperEvent::CursorHidden,
        RemoteDesktopOutput::CursorPosition { x, y } => HelperEvent::CursorPosition { x, y },
        RemoteDesktopOutput::CursorBitmap(cursor) => HelperEvent::CursorRgbaBytes {
            width: cursor.width,
            height: cursor.height,
            hotspot_x: cursor.hotspot_x,
            hotspot_y: cursor.hotspot_y,
            rgba: cursor.rgba,
        },
        RemoteDesktopOutput::ClipboardText { text } => HelperEvent::ClipboardText { text },
        RemoteDesktopOutput::Reconnecting(reconnect) => HelperEvent::Reconnecting {
            reason: helper_reconnect_reason(reconnect.reason),
            delay_secs: reconnect.delay_secs,
        },
        RemoteDesktopOutput::Status(message) => HelperEvent::Status { message },
        RemoteDesktopOutput::ConnectionFailure(message) => {
            HelperEvent::ConnectionFailure { message }
        }
        RemoteDesktopOutput::Terminated(message) => HelperEvent::Terminated { message },
    }
}

fn helper_reconnect_reason(reason: RemoteDesktopReconnectReason) -> HelperReconnectReason {
    match reason {
        RemoteDesktopReconnectReason::DisplayUpdate => HelperReconnectReason::DisplayUpdate,
        RemoteDesktopReconnectReason::SessionError => HelperReconnectReason::SessionError,
        RemoteDesktopReconnectReason::ConnectionLost => HelperReconnectReason::ConnectionLost,
        RemoteDesktopReconnectReason::Manual => HelperReconnectReason::Manual,
    }
}

fn write_event(event: &HelperEvent) -> anyhow::Result<()> {
    let mut stdout = io::stdout().lock();
    protocol::write_event(&mut stdout, event)?;
    stdout.flush()?;
    Ok(())
}

fn setup_logging() -> anyhow::Result<()> {
    let env_filter = EnvFilter::builder()
        .with_env_var("ONETCLI_VNC_HELPER_LOG")
        .from_env_lossy();
    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer().with_writer(io::stderr))
        .try_init()?;
    Ok(())
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
