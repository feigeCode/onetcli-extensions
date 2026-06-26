use std::io::Write;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum HelperRequest {
    Connect {
        destination: String,
        username: Option<String>,
        password: Option<String>,
        domain: Option<String>,
        width: u16,
        height: u16,
    },
    Resize {
        width: u16,
        height: u16,
    },
    MouseMove {
        x: u16,
        y: u16,
    },
    MouseButton {
        button: HelperMouseButton,
        pressed: bool,
    },
    Wheel {
        vertical: bool,
        units: i16,
    },
    Key {
        code: u16,
        extended: bool,
        pressed: bool,
    },
    Text {
        text: String,
    },
    ClipboardText {
        text: String,
    },
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HelperMouseButton {
    Left,
    Middle,
    Right,
    X1,
    X2,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectRequest {
    pub destination: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub domain: Option<String>,
    pub width: u16,
    pub height: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum HelperEvent {
    Status {
        message: String,
    },
    Connected {
        width: u16,
        height: u16,
    },
    FrameBgraBytes {
        width: u16,
        height: u16,
        bgra: Vec<u8>,
    },
    CursorDefault,
    CursorHidden,
    CursorPosition {
        x: u16,
        y: u16,
    },
    ClipboardText {
        text: String,
    },
    ConnectionFailure {
        message: String,
    },
    Terminated {
        message: String,
    },
}

impl HelperEvent {
    pub fn frame(width: u16, height: u16, bgra: Vec<u8>) -> Self {
        Self::FrameBgraBytes {
            width,
            height,
            bgra,
        }
    }
}

pub fn connect_request(request: HelperRequest) -> anyhow::Result<ConnectRequest> {
    match request {
        HelperRequest::Connect {
            destination,
            username,
            password,
            domain,
            width,
            height,
        } => Ok(ConnectRequest {
            destination,
            username,
            password,
            domain,
            width,
            height,
        }),
        _ => anyhow::bail!("first helper request must be Connect"),
    }
}

pub fn decode_request_line(line: &str) -> anyhow::Result<HelperRequest> {
    Ok(serde_json::from_str(line.trim_end())?)
}

pub fn encode_event_line(event: &HelperEvent) -> anyhow::Result<String> {
    if matches!(event, HelperEvent::FrameBgraBytes { .. }) {
        anyhow::bail!("binary frame events must be written with write_event");
    }
    let mut line = serde_json::to_string(event)?;
    line.push('\n');
    Ok(line)
}

pub fn write_event<W>(writer: &mut W, event: &HelperEvent) -> anyhow::Result<()>
where
    W: Write,
{
    match event {
        HelperEvent::FrameBgraBytes {
            width,
            height,
            bgra,
        } => {
            let header = HelperFrameBgraBytesHeader {
                width: *width,
                height: *height,
                bgra_len: bgra.len(),
            };
            let mut line = serde_json::to_string(&header)?;
            line.push('\n');
            writer.write_all(line.as_bytes())?;
            writer.write_all(bgra)?;
        }
        event => writer.write_all(encode_event_line(event)?.as_bytes())?,
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(tag = "type", rename = "FrameBgraBytes")]
struct HelperFrameBgraBytesHeader {
    width: u16,
    height: u16,
    bgra_len: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_connect_request_shape_from_main_process() {
        let line = r#"{"type":"Connect","destination":"10.2.178.12:3389","username":"administrator","password":"secret","domain":null,"width":1280,"height":720}"#;

        let request = connect_request(decode_request_line(line).expect("request decodes"))
            .expect("connect request");

        assert_eq!(request.destination, "10.2.178.12:3389");
        assert_eq!(request.username.as_deref(), Some("administrator"));
        assert_eq!(request.password.as_deref(), Some("secret"));
        assert_eq!(request.width, 1280);
        assert_eq!(request.height, 720);
    }

    #[test]
    fn rejects_binary_frame_event_as_json_line() {
        let event = HelperEvent::frame(1, 1, vec![0x11, 0x22, 0x33, 0xff]);

        let error = encode_event_line(&event).expect_err("binary frame is not a JSON line");

        assert!(error.to_string().contains("write_event"));
    }

    #[test]
    fn writes_binary_frame_event_shape_for_main_process() {
        let event = HelperEvent::frame(2, 1, vec![1, 2, 3, 255, 4, 5, 6, 255]);
        let mut output = Vec::new();

        write_event(&mut output, &event).expect("event writes");

        assert_eq!(
            output,
            b"{\"type\":\"FrameBgraBytes\",\"width\":2,\"height\":1,\"bgra_len\":8}\n\
              \x01\x02\x03\xff\x04\x05\x06\xff"
                .to_vec()
        );
    }

    #[test]
    fn decodes_clipboard_text_request_shape_from_main_process() {
        let line = r#"{"type":"ClipboardText","text":"local 中文"}"#;

        let request = decode_request_line(line).expect("request decodes");

        assert_eq!(
            request,
            HelperRequest::ClipboardText {
                text: "local 中文".to_string()
            }
        );
    }

    #[test]
    fn encodes_clipboard_text_event_shape_for_main_process() {
        let event = HelperEvent::ClipboardText {
            text: "remote 中文".to_string(),
        };

        let line = encode_event_line(&event).expect("event encodes");

        assert_eq!(
            line,
            "{\"type\":\"ClipboardText\",\"text\":\"remote 中文\"}\n"
        );
    }
}
