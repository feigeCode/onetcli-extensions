use super::*;

#[test]
fn keeps_only_latest_pending_frame() {
    let (tx, rx) = output_mailbox();
    tx.send(frame(1)).unwrap();
    tx.send(frame(2)).unwrap();
    tx.send(frame(3)).unwrap();

    assert_eq!(Some(frame(3)), rx.recv());
}

#[test]
fn preserves_control_order_while_replacing_frames() {
    let (tx, rx) = output_mailbox();
    tx.send(HelperEvent::Status {
        message: "one".into(),
    })
    .unwrap();
    tx.send(frame(1)).unwrap();
    tx.send(HelperEvent::ClipboardText { text: "two".into() })
        .unwrap();
    tx.send(frame(2)).unwrap();

    assert_eq!(
        Some(HelperEvent::Status {
            message: "one".into()
        }),
        rx.recv()
    );
    assert_eq!(
        Some(HelperEvent::ClipboardText { text: "two".into() }),
        rx.recv()
    );
    assert_eq!(Some(frame(2)), rx.recv());
}

#[test]
fn terminal_event_discards_pending_frame() {
    let (tx, rx) = output_mailbox();
    tx.send(frame(7)).unwrap();
    tx.send(HelperEvent::Terminated {
        message: "closed".into(),
    })
    .unwrap();

    assert_eq!(
        Some(HelperEvent::Terminated {
            message: "closed".into()
        }),
        rx.recv()
    );
    drop(tx);
    assert_eq!(None, rx.recv());
}

#[test]
fn keeps_keyframe_when_coalescing_dirty_rectangles() {
    let (tx, rx) = output_mailbox();
    tx.send(HelperEvent::frame(128, 128, vec![0; 128 * 128 * 4]))
        .unwrap();
    tx.send(HelperEvent::FrameBgraRects {
        width: 128,
        height: 128,
        rects: vec![crate::protocol::HelperFrameRect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            byte_len: 4,
        }],
        bgra: vec![1, 2, 3, 255],
    })
    .unwrap();

    assert!(matches!(
        rx.recv(),
        Some(HelperEvent::FrameBgraBytes { .. })
    ));
    assert!(matches!(
        rx.recv(),
        Some(HelperEvent::FrameBgraRects { .. })
    ));
}

#[test]
fn last_sender_drop_wakes_receiver() {
    let (tx, rx) = output_mailbox();
    let waiter = std::thread::spawn(move || rx.recv());

    drop(tx);

    assert_eq!(None, waiter.join().unwrap());
}

#[test]
fn send_fails_after_receiver_is_dropped() {
    let (tx, rx) = output_mailbox();
    drop(rx);

    assert!(tx.send(frame(1)).is_err());
}

#[test]
fn coalesces_adjacent_cursor_positions() {
    let (tx, rx) = output_mailbox();
    tx.send(HelperEvent::CursorPosition { x: 1, y: 2 }).unwrap();
    tx.send(HelperEvent::CursorPosition { x: 3, y: 4 }).unwrap();

    assert_eq!(Some(HelperEvent::CursorPosition { x: 3, y: 4 }), rx.recv());
}

#[test]
fn coalesces_adjacent_cursor_bitmaps_without_crossing_state_boundaries() {
    let (tx, rx) = output_mailbox();
    tx.send(cursor(1)).unwrap();
    tx.send(cursor(2)).unwrap();
    tx.send(HelperEvent::CursorHidden).unwrap();
    tx.send(cursor(3)).unwrap();

    assert_eq!(Some(cursor(2)), rx.recv());
    assert_eq!(Some(HelperEvent::CursorHidden), rx.recv());
    assert_eq!(Some(cursor(3)), rx.recv());
}

#[test]
fn reconnect_barrier_discards_pending_cursor_state() {
    let (tx, rx) = output_mailbox();
    tx.send(HelperEvent::CursorPosition { x: 1, y: 2 }).unwrap();
    tx.send(cursor(1)).unwrap();
    tx.send(HelperEvent::Reconnecting {
        reason: crate::protocol::HelperReconnectReason::ConnectionLost,
        delay_secs: Some(1),
    })
    .unwrap();

    assert_eq!(
        Some(HelperEvent::Reconnecting {
            reason: crate::protocol::HelperReconnectReason::ConnectionLost,
            delay_secs: Some(1),
        }),
        rx.recv()
    );
}

fn frame(value: u8) -> HelperEvent {
    HelperEvent::frame(1, 1, vec![value, 0, 0, 255])
}

fn cursor(value: u8) -> HelperEvent {
    HelperEvent::CursorRgbaBytes {
        width: 1,
        height: 1,
        hotspot_x: 0,
        hotspot_y: 0,
        rgba: vec![value, 0, 0, 255],
    }
}
