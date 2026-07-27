use std::fmt;

pub(crate) const MAX_CUT_TEXT_BYTES: usize = vnc_client::MAX_CUT_TEXT_BYTES;

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct VncClipboardSnapshot {
    wire_bytes: Vec<u8>,
}

impl VncClipboardSnapshot {
    pub(crate) fn encode(text: &str) -> Option<Self> {
        let mut wire_bytes = Vec::new();
        let mut replacement_count = 0usize;
        for character in text.chars() {
            if wire_bytes.len() == MAX_CUT_TEXT_BYTES {
                trace_oversized_payload(text.len());
                return None;
            }
            match u8::try_from(u32::from(character)) {
                Ok(byte) => wire_bytes.push(byte),
                Err(_) => {
                    wire_bytes.push(b'?');
                    replacement_count += 1;
                }
            }
        }
        if replacement_count > 0 {
            tracing::debug!(
                replacement_count,
                "replaced non-Latin-1 VNC clipboard codepoints"
            );
        }
        Some(Self { wire_bytes })
    }

    pub(crate) fn wire_bytes(&self) -> &[u8] {
        &self.wire_bytes
    }
}

pub(crate) fn decode_clipboard_text(bytes: &[u8]) -> Option<String> {
    if bytes.len() > MAX_CUT_TEXT_BYTES {
        trace_oversized_payload(bytes.len());
        return None;
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => Some(text.to_owned()),
        Err(_) => Some(bytes.iter().copied().map(char::from).collect()),
    }
}

fn trace_oversized_payload(actual: usize) {
    tracing::warn!(
        actual,
        maximum = MAX_CUT_TEXT_BYTES,
        "ignored oversized VNC clipboard payload"
    );
}

impl fmt::Debug for VncClipboardSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VncClipboardSnapshot")
            .field("byte_len", &self.wire_bytes.len())
            .finish()
    }
}

#[cfg(test)]
#[path = "vnc_clipboard_tests.rs"]
mod tests;
