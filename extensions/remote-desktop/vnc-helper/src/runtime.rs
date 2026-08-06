#[derive(Clone)]
pub struct RemoteDesktopConnectionOptions {
    pub destination: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub domain: Option<String>,
}

#[derive(Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum RemoteDesktopInput {
    Resize {
        width: u16,
        height: u16,
    },
    MouseMove {
        x: u16,
        y: u16,
    },
    MouseButton {
        button: RemoteMouseButton,
        pressed: bool,
    },
    Wheel {
        vertical: bool,
        units: i16,
    },
    Key {
        key: RemoteKey,
        pressed: bool,
    },
    Text {
        text: String,
    },
    ClipboardText {
        text: String,
    },
    ClipboardFiles {
        paths: Vec<String>,
    },
    Reconnect,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteMouseButton {
    Left,
    Middle,
    Right,
    X1,
    X2,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum RemoteKey {
    Scancode(u16),
    KeySym(u32),
}

#[derive(Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum RemoteDesktopOutput {
    Connected {
        width: u16,
        height: u16,
        capabilities: RemoteDesktopCapabilities,
    },
    Frame {
        width: u16,
        height: u16,
        rgba: Vec<u8>,
    },
    FrameBgraRects {
        width: u16,
        height: u16,
        rects: Vec<RemoteDesktopFrameRect>,
        bgra: Vec<u8>,
    },
    CursorDefault,
    CursorHidden,
    CursorPosition {
        x: u16,
        y: u16,
    },
    CursorBitmap(RemoteDesktopCursor),
    ClipboardText {
        text: String,
    },
    Reconnecting(RemoteDesktopReconnect),
    Status(String),
    ConnectionFailure(String),
    Terminated(String),
}

#[derive(Clone, PartialEq, Eq)]
pub struct RemoteDesktopCursor {
    pub width: u16,
    pub height: u16,
    pub hotspot_x: u16,
    pub hotspot_y: u16,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemoteDesktopReconnect {
    pub reason: RemoteDesktopReconnectReason,
    pub delay_secs: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteDesktopReconnectReason {
    DisplayUpdate,
    SessionError,
    ConnectionLost,
    Manual,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteDesktopFrameRect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub byte_len: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemoteDesktopCapabilities {
    pub resize: ResizeSupport,
    pub clipboard_text: bool,
    pub cursor_shape: bool,
    pub audio: bool,
    pub file_transfer: bool,
}

impl RemoteDesktopCapabilities {
    pub fn vnc_mvp() -> Self {
        Self {
            resize: ResizeSupport::LocalScaleOnly,
            clipboard_text: true,
            cursor_shape: false,
            audio: false,
            file_transfer: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeSupport {
    LocalScaleOnly,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vnc_mvp_reports_text_clipboard_without_remote_cursor_support() {
        let capabilities = RemoteDesktopCapabilities::vnc_mvp();

        assert!(capabilities.clipboard_text);
        assert!(!capabilities.cursor_shape);
    }

    #[test]
    fn connection_options_debug_redacts_structured_credentials() {
        let options = RemoteDesktopConnectionOptions {
            destination: "host:5900".to_string(),
            username: Some("alice-secret".to_string()),
            password: Some("password-secret".to_string()),
            domain: Some("domain-secret".to_string()),
        };

        let debug = format!("{options:?}");

        assert!(debug.contains("username_present: true"));
        assert!(debug.contains("password_present: true"));
        assert!(debug.contains("domain_present: true"));
        assert!(!debug.contains("alice-secret"));
        assert!(!debug.contains("password-secret"));
        assert!(!debug.contains("domain-secret"));
    }

    #[test]
    fn runtime_debug_redacts_clipboard_paths_text_messages_and_pixels() {
        let inputs = [
            RemoteDesktopInput::ClipboardText {
                text: "runtime-input-secret".to_string(),
            },
            RemoteDesktopInput::ClipboardFiles {
                paths: vec!["C:\\runtime\\path-secret.txt".to_string()],
            },
        ];
        for input in inputs {
            let debug = format!("{input:?}");
            assert!(!debug.contains("runtime-input-secret"));
            assert!(!debug.contains("path-secret.txt"));
        }

        let outputs = [
            RemoteDesktopOutput::Status("runtime-status-secret".to_string()),
            RemoteDesktopOutput::Frame {
                width: 1,
                height: 1,
                rgba: vec![17, 34, 51, 68],
            },
            RemoteDesktopOutput::CursorBitmap(RemoteDesktopCursor {
                width: 1,
                height: 1,
                hotspot_x: 0,
                hotspot_y: 0,
                rgba: vec![17, 34, 51, 68],
            }),
        ];
        for output in outputs {
            let debug = format!("{output:?}");
            assert!(!debug.contains("runtime-status-secret"));
            assert!(!debug.contains("[17, 34, 51, 68]"));
        }
    }
}

#[path = "runtime_debug.rs"]
mod debug;
