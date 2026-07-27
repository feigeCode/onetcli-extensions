use std::fmt;

use super::{ConnectRequest, HelperEvent, HelperRequest};

impl fmt::Debug for HelperRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect { .. } => debug_connect(self, formatter),
            Self::Text { text } | Self::ClipboardText { text } => formatter
                .debug_struct(request_name(self))
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
            Self::Key {
                code,
                extended,
                pressed,
            } => formatter
                .debug_struct("Key")
                .field("code", code)
                .field("extended", extended)
                .field("pressed", pressed)
                .finish(),
            Self::KeySym { keysym, pressed } => formatter
                .debug_struct("KeySym")
                .field("keysym", keysym)
                .field("pressed", pressed)
                .finish(),
            Self::Close => formatter.write_str("Close"),
        }
    }
}

fn debug_connect(request: &HelperRequest, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    let HelperRequest::Connect {
        destination,
        username,
        password,
        domain,
        width,
        height,
    } = request
    else {
        unreachable!();
    };
    formatter
        .debug_struct("Connect")
        .field("destination", destination)
        .field("username_present", &username.is_some())
        .field("username_len", &option_len(username))
        .field("password_present", &password.is_some())
        .field("domain_present", &domain.is_some())
        .field("domain_len", &option_len(domain))
        .field("width", width)
        .field("height", height)
        .finish()
}

fn request_name(request: &HelperRequest) -> &'static str {
    match request {
        HelperRequest::Text { .. } => "Text",
        HelperRequest::ClipboardText { .. } => "ClipboardText",
        _ => unreachable!(),
    }
}

impl fmt::Debug for ConnectRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConnectRequest")
            .field("destination", &self.destination)
            .field("username_present", &self.username.is_some())
            .field("username_len", &option_len(&self.username))
            .field("password_present", &self.password.is_some())
            .field("domain_present", &self.domain.is_some())
            .field("domain_len", &option_len(&self.domain))
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

impl fmt::Debug for HelperEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Status { message }
            | Self::ConnectionFailure { message }
            | Self::Terminated { message } => formatter
                .debug_struct(event_name(self))
                .field("message_len", &message.len())
                .finish(),
            Self::ClipboardText { text } => formatter
                .debug_struct("ClipboardText")
                .field("text_len", &text.len())
                .finish(),
            Self::FrameBgraBytes {
                width,
                height,
                bgra,
            } => formatter
                .debug_struct("FrameBgraBytes")
                .field("width", width)
                .field("height", height)
                .field("byte_len", &bgra.len())
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
            Self::Connected { width, height } => formatter
                .debug_struct("Connected")
                .field("width", width)
                .field("height", height)
                .finish(),
            Self::Reconnecting { reason, delay_secs } => formatter
                .debug_struct("Reconnecting")
                .field("reason", reason)
                .field("delay_secs", delay_secs)
                .finish(),
            Self::CursorDefault | Self::CursorHidden => formatter.write_str(event_name(self)),
            Self::CursorPosition { x, y } => formatter
                .debug_struct("CursorPosition")
                .field("x", x)
                .field("y", y)
                .finish(),
        }
    }
}

fn event_name(event: &HelperEvent) -> &'static str {
    match event {
        HelperEvent::Status { .. } => "Status",
        HelperEvent::ConnectionFailure { .. } => "ConnectionFailure",
        HelperEvent::Terminated { .. } => "Terminated",
        HelperEvent::CursorDefault => "CursorDefault",
        HelperEvent::CursorHidden => "CursorHidden",
        _ => unreachable!(),
    }
}

fn option_len(value: &Option<String>) -> Option<usize> {
    value.as_ref().map(String::len)
}
