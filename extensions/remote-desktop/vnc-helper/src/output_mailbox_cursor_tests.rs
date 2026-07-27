use super::*;
use crate::runtime::{
    RemoteDesktopCapabilities, RemoteDesktopCursor, RemoteDesktopOutput, RemoteDesktopReconnect,
    RemoteDesktopReconnectReason,
};

#[test]
fn coalesces_adjacent_cursor_positions() {
    let outputs = collect_outputs([
        RemoteDesktopOutput::CursorPosition { x: 1, y: 2 },
        RemoteDesktopOutput::CursorPosition { x: 3, y: 4 },
    ]);

    assert_eq!(
        vec![RemoteDesktopOutput::CursorPosition { x: 3, y: 4 }],
        outputs
    );
}

#[test]
fn coalesces_adjacent_cursor_bitmaps() {
    let outputs = collect_outputs([cursor(1), cursor(2)]);

    assert_eq!(vec![cursor(2)], outputs);
}

#[test]
fn cursor_visibility_state_is_a_coalescing_barrier() {
    let outputs = collect_outputs([cursor(1), RemoteDesktopOutput::CursorHidden, cursor(2)]);

    assert_eq!(
        vec![cursor(1), RemoteDesktopOutput::CursorHidden, cursor(2)],
        outputs
    );
}

#[test]
fn reconnect_discards_pending_cursor_state() {
    let (tx, rx) = output_mailbox();
    tx.send(RemoteDesktopOutput::CursorPosition { x: 1, y: 2 })
        .unwrap();
    tx.send(cursor(1)).unwrap();
    tx.send(reconnecting()).unwrap();
    drop(tx);

    assert_eq!(vec![reconnecting()], receive_all(rx));
}

#[test]
fn terminal_event_discards_pending_cursor_state() {
    let outputs = collect_outputs([
        RemoteDesktopOutput::CursorPosition { x: 1, y: 2 },
        cursor(1),
        RemoteDesktopOutput::Terminated("closed".to_string()),
    ]);

    assert_eq!(
        vec![RemoteDesktopOutput::Terminated("closed".to_string())],
        outputs
    );
}

#[test]
fn begin_generation_discards_stale_cursor_state() {
    let (tx, rx) = output_mailbox();
    tx.send(RemoteDesktopOutput::Status("old".to_string()))
        .unwrap();
    tx.send(cursor(1)).unwrap();
    tx.begin_generation();
    tx.send(connected()).unwrap();
    drop(tx);

    assert_eq!(
        vec![RemoteDesktopOutput::Status("old".to_string()), connected()],
        receive_all(rx)
    );
}

#[test]
fn connected_cursor_and_frame_keep_wire_order() {
    let outputs = collect_outputs([connected(), cursor(1), frame(7)]);

    assert_eq!(vec![connected(), cursor(1), frame(7)], outputs);
}

fn collect_outputs<const N: usize>(outputs: [RemoteDesktopOutput; N]) -> Vec<RemoteDesktopOutput> {
    let (tx, rx) = output_mailbox();
    for output in outputs {
        tx.send(output).unwrap();
    }
    drop(tx);
    receive_all(rx)
}

fn receive_all(rx: OutputReceiver) -> Vec<RemoteDesktopOutput> {
    let mut outputs = Vec::new();
    while let Some(output) = rx.recv() {
        outputs.push(output);
    }
    outputs
}

fn cursor(value: u8) -> RemoteDesktopOutput {
    RemoteDesktopOutput::CursorBitmap(RemoteDesktopCursor {
        width: 1,
        height: 1,
        hotspot_x: 0,
        hotspot_y: 0,
        rgba: vec![value, 0, 0, 255],
    })
}

fn reconnecting() -> RemoteDesktopOutput {
    RemoteDesktopOutput::Reconnecting(RemoteDesktopReconnect {
        reason: RemoteDesktopReconnectReason::ConnectionLost,
        delay_secs: Some(1),
    })
}

fn connected() -> RemoteDesktopOutput {
    RemoteDesktopOutput::Connected {
        width: 2,
        height: 2,
        capabilities: RemoteDesktopCapabilities::vnc_mvp(),
    }
}

fn frame(value: u8) -> RemoteDesktopOutput {
    RemoteDesktopOutput::Frame {
        width: 1,
        height: 1,
        rgba: vec![value, 0, 0, 255],
    }
}
