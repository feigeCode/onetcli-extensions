use std::fmt;

use super::{RemoteDesktopConnectionOptions, RemoteDesktopInput, RemoteDesktopOutput};

impl fmt::Debug for RemoteDesktopConnectionOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RemoteDesktopConnectionOptions")
            .field("destination", &self.destination)
            .field("username_present", &self.username.is_some())
            .field("password_present", &self.password.is_some())
            .field("domain_present", &self.domain.is_some())
            .finish()
    }
}

impl fmt::Debug for RemoteDesktopInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text { text } | Self::ClipboardText { text } => formatter
                .debug_struct(input_name(self))
                .field("text_len", &text.len())
                .finish(),
            Self::ClipboardFiles { paths } => formatter
                .debug_struct("ClipboardFiles")
                .field("path_count", &paths.len())
                .finish(),
            Self::Resize { width, height } => formatter
                .debug_struct("Resize")
                .field("width", width)
                .field("height", height)
                .finish(),
            Self::MouseMove { x, y } => formatter
                .debug_struct("MouseMove")
                .field("x", x)
                .field("y", y)
                .finish(),
            Self::MouseButton { button, pressed } => formatter
                .debug_struct("MouseButton")
                .field("button", button)
                .field("pressed", pressed)
                .finish(),
            Self::Wheel { vertical, units } => formatter
                .debug_struct("Wheel")
                .field("vertical", vertical)
                .field("units", units)
                .finish(),
            Self::Key { key, pressed } => formatter
                .debug_struct("Key")
                .field("key", key)
                .field("pressed", pressed)
                .finish(),
            Self::Reconnect => formatter.write_str("Reconnect"),
            Self::Close => formatter.write_str("Close"),
        }
    }
}

fn input_name(input: &RemoteDesktopInput) -> &'static str {
    match input {
        RemoteDesktopInput::Text { .. } => "Text",
        RemoteDesktopInput::ClipboardText { .. } => "ClipboardText",
        _ => unreachable!(),
    }
}

impl fmt::Debug for RemoteDesktopOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Status(message)
            | Self::ConnectionFailure(message)
            | Self::Terminated(message) => formatter
                .debug_struct(output_name(self))
                .field("message_len", &message.len())
                .finish(),
            Self::ClipboardText { text } => formatter
                .debug_struct("ClipboardText")
                .field("text_len", &text.len())
                .finish(),
            Self::Frame {
                width,
                height,
                rgba,
            } => formatter
                .debug_struct("Frame")
                .field("width", width)
                .field("height", height)
                .field("byte_len", &rgba.len())
                .finish(),
            Self::FrameBgraRects {
                width,
                height,
                rects,
                bgra,
            } => formatter
                .debug_struct("FrameBgraRects")
                .field("width", width)
                .field("height", height)
                .field("rect_count", &rects.len())
                .field("byte_len", &bgra.len())
                .finish(),
            Self::Connected {
                width,
                height,
                capabilities,
            } => formatter
                .debug_struct("Connected")
                .field("width", width)
                .field("height", height)
                .field("capabilities", capabilities)
                .finish(),
            Self::Reconnecting(reconnect) => formatter
                .debug_tuple("Reconnecting")
                .field(reconnect)
                .finish(),
            Self::CursorDefault | Self::CursorHidden => formatter.write_str(output_name(self)),
            Self::CursorPosition { x, y } => formatter
                .debug_struct("CursorPosition")
                .field("x", x)
                .field("y", y)
                .finish(),
            Self::CursorBitmap(cursor) => formatter
                .debug_struct("CursorBitmap")
                .field("width", &cursor.width)
                .field("height", &cursor.height)
                .field("hotspot_x", &cursor.hotspot_x)
                .field("hotspot_y", &cursor.hotspot_y)
                .field("byte_len", &cursor.rgba.len())
                .finish(),
        }
    }
}

fn output_name(output: &RemoteDesktopOutput) -> &'static str {
    match output {
        RemoteDesktopOutput::Status(_) => "Status",
        RemoteDesktopOutput::ConnectionFailure(_) => "ConnectionFailure",
        RemoteDesktopOutput::Terminated(_) => "Terminated",
        RemoteDesktopOutput::CursorDefault => "CursorDefault",
        RemoteDesktopOutput::CursorHidden => "CursorHidden",
        _ => unreachable!(),
    }
}
