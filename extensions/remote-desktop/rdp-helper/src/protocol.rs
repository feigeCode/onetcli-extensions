use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum HelperRequest {
    Connect {
        destination: String,
        username: Option<String>,
        password: Option<String>,
        domain: Option<String>,
        width: u16,
        height: u16,
        #[serde(default = "default_scale_factor")]
        scale_factor: u32,
        #[serde(default)]
        audio_playback: bool,
        #[serde(default)]
        audio_capture: bool,
        #[serde(default)]
        shared_folders: Vec<RemoteDesktopSharedFolder>,
    },
    Resize {
        width: u16,
        height: u16,
        #[serde(default = "default_scale_factor")]
        scale_factor: u32,
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
    ClipboardFiles {
        #[serde(default)]
        transfer_id: u64,
        paths: Vec<String>,
    },
    CancelClipboardTransfer {
        transfer_id: u64,
    },
    Close,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteDesktopSharedFolder {
    pub name: String,
    pub path: PathBuf,
    pub read_only: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HelperMouseButton {
    Left,
    Middle,
    Right,
    X1,
    X2,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectRequest {
    pub destination: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub domain: Option<String>,
    pub width: u16,
    pub height: u16,
    pub scale_factor: u32,
    pub audio_playback: bool,
    pub audio_capture: bool,
    pub shared_folders: Vec<RemoteDesktopSharedFolder>,
}

fn default_scale_factor() -> u32 {
    100
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    FrameBgraRects {
        width: u16,
        height: u16,
        rects: Vec<HelperFrameRect>,
        bgra: Vec<u8>,
    },
    CursorDefault,
    CursorHidden,
    CursorPosition {
        x: u16,
        y: u16,
    },
    CursorRgbaBytes {
        width: u16,
        height: u16,
        hotspot_x: u16,
        hotspot_y: u16,
        rgba: Vec<u8>,
    },
    ClipboardText {
        text: String,
    },
    ClipboardFilesReady {
        transfer_id: u64,
        paths: Vec<String>,
    },
    ClipboardTransferFailed {
        transfer_id: u64,
        message: String,
    },
    Reconnecting {
        reason: HelperReconnectReason,
        delay_secs: Option<u64>,
    },
    ConnectionFailure {
        message: String,
    },
    Terminated {
        message: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelperReconnectReason {
    DisplayUpdate,
    SessionError,
    ConnectionLost,
    Manual,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelperFrameRect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub byte_len: usize,
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
            scale_factor,
            audio_playback,
            audio_capture,
            shared_folders,
        } => Ok(ConnectRequest {
            destination,
            username,
            password,
            domain,
            width,
            height,
            scale_factor,
            audio_playback,
            audio_capture,
            shared_folders,
        }),
        _ => anyhow::bail!("first helper request must be Connect"),
    }
}

pub fn decode_request_line(line: &str) -> anyhow::Result<HelperRequest> {
    Ok(serde_json::from_str(line.trim_end())?)
}

pub fn encode_event_line(event: &HelperEvent) -> anyhow::Result<String> {
    if matches!(
        event,
        HelperEvent::FrameBgraBytes { .. }
            | HelperEvent::FrameBgraRects { .. }
            | HelperEvent::CursorRgbaBytes { .. }
    ) {
        anyhow::bail!("binary image events must be written with write_event");
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
        HelperEvent::FrameBgraRects {
            width,
            height,
            rects,
            bgra,
        } => {
            let header = HelperFrameBgraRectsHeader {
                width: *width,
                height: *height,
                rects,
                bgra_len: bgra.len(),
            };
            let mut line = serde_json::to_string(&header)?;
            line.push('\n');
            writer.write_all(line.as_bytes())?;
            writer.write_all(bgra)?;
        }
        HelperEvent::CursorRgbaBytes {
            width,
            height,
            hotspot_x,
            hotspot_y,
            rgba,
        } => {
            let header = HelperCursorRgbaBytesHeader {
                width: *width,
                height: *height,
                hotspot_x: *hotspot_x,
                hotspot_y: *hotspot_y,
                rgba_len: rgba.len(),
            };
            let mut line = serde_json::to_string(&header)?;
            line.push('\n');
            writer.write_all(line.as_bytes())?;
            writer.write_all(rgba)?;
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

#[derive(Serialize)]
#[serde(tag = "type", rename = "FrameBgraRects")]
struct HelperFrameBgraRectsHeader<'a> {
    width: u16,
    height: u16,
    rects: &'a [HelperFrameRect],
    bgra_len: usize,
}

#[derive(Serialize)]
#[serde(tag = "type", rename = "CursorRgbaBytes")]
struct HelperCursorRgbaBytesHeader {
    width: u16,
    height: u16,
    hotspot_x: u16,
    hotspot_y: u16,
    rgba_len: usize,
}

#[path = "protocol_debug.rs"]
mod debug;

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "protocol_clipboard_tests.rs"]
mod clipboard_tests;
