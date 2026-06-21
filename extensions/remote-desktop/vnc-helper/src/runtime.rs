#[derive(Clone)]
pub struct RemoteDesktopConnectionOptions {
    pub destination: String,
    pub password: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq, Eq)]
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
    CursorDefault,
    CursorHidden,
    CursorPosition {
        x: u16,
        y: u16,
    },
    ClipboardText {
        text: String,
    },
    Status(String),
    ConnectionFailure(String),
    Terminated(String),
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
            clipboard_text: false,
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
