use std::io::{self, BufRead, Write};
use std::thread::JoinHandle;

use runtime::{
    RemoteDesktopConnectionOptions, RemoteDesktopInput, RemoteDesktopOutput, RemoteKey,
    RemoteMouseButton,
};
use tracing::error;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

use crate::protocol::{HelperEvent, HelperMouseButton, HelperRequest};

mod framebuffer;
mod protocol;
mod runtime;
mod vnc_encoding;
mod vnc_input;
mod vnc_keyboard;
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
    let (input_tx, mut input_rx) = tokio::sync::mpsc::unbounded_channel();
    let (output_tx, output_rx) = std::sync::mpsc::channel();
    let output_thread = spawn_output_writer(output_rx);
    let vnc_thread = spawn_vnc_thread(connect_options(connect), &mut input_rx, output_tx)?;

    for line in lines {
        let request = protocol::decode_request_line(&line?)?;
        let stop = matches!(request, HelperRequest::Close);
        if let Some(input) = request_to_input(request) {
            let _ = input_tx.send(input);
        }
        if stop {
            break;
        }
    }

    let _ = input_tx.send(RemoteDesktopInput::Close);
    let _ = vnc_thread.join();
    let _ = output_thread.join();
    Ok(())
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
    input_rx: &mut tokio::sync::mpsc::UnboundedReceiver<RemoteDesktopInput>,
    output_tx: std::sync::mpsc::Sender<RemoteDesktopOutput>,
) -> anyhow::Result<JoinHandle<()>> {
    let mut input_rx = std::mem::replace(input_rx, tokio::sync::mpsc::unbounded_channel().1);
    Ok(std::thread::Builder::new()
        .name("onetcli-vnc-helper-session".to_string())
        .spawn(move || vnc_rfb::run_vnc_thread(options, &mut input_rx, output_tx))?)
}

fn spawn_output_writer(
    output_rx: std::sync::mpsc::Receiver<RemoteDesktopOutput>,
) -> JoinHandle<anyhow::Result<()>> {
    std::thread::Builder::new()
        .name("onetcli-vnc-helper-output".to_string())
        .spawn(move || {
            for output in output_rx {
                write_event(&output_to_event(output))?;
            }
            Ok(())
        })
        .expect("spawn output writer")
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
        RemoteDesktopOutput::CursorDefault => HelperEvent::CursorDefault,
        RemoteDesktopOutput::CursorHidden => HelperEvent::CursorHidden,
        RemoteDesktopOutput::CursorPosition { x, y } => HelperEvent::CursorPosition { x, y },
        RemoteDesktopOutput::ClipboardText { text } => HelperEvent::ClipboardText { text },
        RemoteDesktopOutput::Status(message) => HelperEvent::Status { message },
        RemoteDesktopOutput::ConnectionFailure(message) => {
            HelperEvent::ConnectionFailure { message }
        }
        RemoteDesktopOutput::Terminated(message) => HelperEvent::Terminated { message },
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
mod tests {
    use super::*;

    #[test]
    fn reads_first_stdin_line_as_connect_request() {
        let input = vec![Ok(
            r#"{"type":"Connect","destination":"host:5900","username":null,"password":"secret","domain":null,"width":800,"height":600}"#
                .to_string(),
        )];
        let mut lines = input.into_iter();

        let request = read_connect_request(&mut lines).expect("connect request");

        assert_eq!(request.destination, "host:5900");
        assert_eq!(request.password.as_deref(), Some("secret"));
    }

    #[test]
    fn converts_extended_key_request_to_prefixed_scancode() {
        let input = request_to_input(HelperRequest::Key {
            code: 0x48,
            extended: true,
            pressed: true,
        });

        assert_eq!(
            input,
            Some(RemoteDesktopInput::Key {
                key: RemoteKey::Scancode(0xe048),
                pressed: true
            })
        );
    }

    #[test]
    fn converts_keysym_request_to_remote_keysym() {
        let input = request_to_input(HelperRequest::KeySym {
            keysym: b':' as u32,
            pressed: true,
        });

        assert_eq!(
            input,
            Some(RemoteDesktopInput::Key {
                key: RemoteKey::KeySym(b':' as u32),
                pressed: true,
            })
        );
    }
}
