use super::*;
use crate::runtime::{RemoteDesktopCursor, RemoteDesktopOutput};

#[test]
fn maps_rgba_cursor_and_hotspot() {
    let output = map_vnc_cursor(
        vnc_client::Rect {
            x: 1,
            y: 0,
            width: 2,
            height: 1,
        },
        vec![1, 2, 3, 255, 4, 5, 6, 128],
    )
    .expect("valid cursor");

    assert_eq!(
        RemoteDesktopOutput::CursorBitmap(RemoteDesktopCursor {
            width: 2,
            height: 1,
            hotspot_x: 1,
            hotspot_y: 0,
            rgba: vec![1, 2, 3, 255, 4, 5, 6, 128],
        }),
        output
    );
}

#[test]
fn maps_zero_width_or_height_to_hidden_cursor() {
    for rect in [
        vnc_client::Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 1,
        },
        vnc_client::Rect {
            x: 0,
            y: 0,
            width: 1,
            height: 0,
        },
    ] {
        assert_eq!(
            RemoteDesktopOutput::CursorHidden,
            map_vnc_cursor(rect, Vec::new()).expect("empty cursor hides")
        );
    }
}

#[test]
fn rejects_mismatched_cursor_payload_length() {
    let error = map_vnc_cursor(rect(2, 1), vec![0; 7]).expect_err("payload must match dimensions");

    assert!(error.contains("payload length"));
}

#[test]
fn rejects_out_of_bounds_cursor_hotspot() {
    for rect in [
        vnc_client::Rect {
            x: 2,
            y: 0,
            width: 2,
            height: 1,
        },
        vnc_client::Rect {
            x: 0,
            y: 1,
            width: 2,
            height: 1,
        },
    ] {
        let error = map_vnc_cursor(rect, vec![0; 8]).expect_err("hotspot must be in bitmap");
        assert!(error.contains("hotspot"));
    }
}

#[test]
fn accepts_maximum_cursor_dimensions() {
    let edge = MAX_CURSOR_DIMENSION;
    let rgba_len = usize::from(edge) * usize::from(edge) * CURSOR_PIXEL_BYTES;

    let output =
        map_vnc_cursor(rect(edge, edge), vec![0; rgba_len]).expect("maximum cursor is valid");

    assert!(matches!(output, RemoteDesktopOutput::CursorBitmap(_)));
}

#[test]
fn rejects_cursor_dimensions_above_limit() {
    let error = map_vnc_cursor(rect(MAX_CURSOR_DIMENSION + 1, 1), Vec::new())
        .expect_err("oversized cursor must fail");

    assert!(error.contains("dimensions"));
}

fn rect(width: u16, height: u16) -> vnc_client::Rect {
    vnc_client::Rect {
        x: 0,
        y: 0,
        width,
        height,
    }
}
