use vnc_client::Rect;

use crate::runtime::{RemoteDesktopCursor, RemoteDesktopOutput};

pub(crate) const MAX_CURSOR_DIMENSION: u16 = 1024;
pub(crate) const CURSOR_PIXEL_BYTES: usize = 4;

pub(crate) fn map_vnc_cursor(rect: Rect, rgba: Vec<u8>) -> Result<RemoteDesktopOutput, String> {
    if rect.width == 0 || rect.height == 0 {
        return Ok(RemoteDesktopOutput::CursorHidden);
    }
    validate_dimensions(rect)?;
    validate_hotspot(rect)?;
    validate_payload_length(rect, rgba.len())?;
    Ok(RemoteDesktopOutput::CursorBitmap(RemoteDesktopCursor {
        width: rect.width,
        height: rect.height,
        hotspot_x: rect.x,
        hotspot_y: rect.y,
        rgba,
    }))
}

fn validate_dimensions(rect: Rect) -> Result<(), String> {
    if rect.width > MAX_CURSOR_DIMENSION || rect.height > MAX_CURSOR_DIMENSION {
        return Err(format!(
            "VNC cursor dimensions {}x{} exceed the {}x{} limit",
            rect.width, rect.height, MAX_CURSOR_DIMENSION, MAX_CURSOR_DIMENSION
        ));
    }
    Ok(())
}

fn validate_hotspot(rect: Rect) -> Result<(), String> {
    if rect.x >= rect.width || rect.y >= rect.height {
        return Err(format!(
            "VNC cursor hotspot ({}, {}) is outside the {}x{} bitmap",
            rect.x, rect.y, rect.width, rect.height
        ));
    }
    Ok(())
}

fn validate_payload_length(rect: Rect, actual: usize) -> Result<(), String> {
    let expected = usize::from(rect.width)
        .checked_mul(usize::from(rect.height))
        .and_then(|pixels| pixels.checked_mul(CURSOR_PIXEL_BYTES))
        .ok_or_else(|| "VNC cursor dimensions overflow payload length".to_string())?;
    if actual != expected {
        return Err(format!(
            "VNC cursor payload length is {actual}, expected {expected}"
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "vnc_cursor_tests.rs"]
mod tests;
