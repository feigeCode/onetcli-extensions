use super::*;
use crate::runtime::{RemoteDesktopOutput, RemoteDesktopReconnect, RemoteDesktopReconnectReason};

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
    tx.send(RemoteDesktopOutput::Status("one".into())).unwrap();
    tx.send(frame(1)).unwrap();
    tx.send(RemoteDesktopOutput::ClipboardText { text: "two".into() })
        .unwrap();
    tx.send(frame(2)).unwrap();

    assert_eq!(Some(RemoteDesktopOutput::Status("one".into())), rx.recv());
    assert_eq!(
        Some(RemoteDesktopOutput::ClipboardText { text: "two".into() }),
        rx.recv()
    );
    assert_eq!(Some(frame(2)), rx.recv());
}

#[test]
fn terminal_event_discards_pending_frame() {
    let (tx, rx) = output_mailbox();
    tx.send(frame(7)).unwrap();
    tx.send(RemoteDesktopOutput::Terminated("closed".into()))
        .unwrap();

    assert_eq!(
        Some(RemoteDesktopOutput::Terminated("closed".into())),
        rx.recv()
    );
    drop(tx);
    assert_eq!(None, rx.recv());
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
fn begin_generation_discards_stale_frame_and_delta_but_keeps_controls() {
    let (tx, rx) = output_mailbox();
    tx.send(RemoteDesktopOutput::Status("old".into())).unwrap();
    tx.send(RemoteDesktopOutput::FrameBgraRects {
        width: 1,
        height: 1,
        rects: vec![crate::runtime::RemoteDesktopFrameRect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            byte_len: 4,
        }],
        bgra: vec![1, 2, 3, 4],
    })
    .unwrap();

    tx.begin_generation();
    tx.send(RemoteDesktopOutput::Connected {
        width: 2,
        height: 2,
        capabilities: crate::runtime::RemoteDesktopCapabilities::vnc_mvp(),
    })
    .unwrap();
    tx.send(frame(9)).unwrap();

    assert_eq!(Some(RemoteDesktopOutput::Status("old".into())), rx.recv());
    assert!(matches!(
        rx.recv(),
        Some(RemoteDesktopOutput::Connected {
            width: 2,
            height: 2,
            ..
        })
    ));
    assert_eq!(Some(frame(9)), rx.recv());
    drop(tx);
    assert_eq!(None, rx.recv());
}

#[test]
fn reconnect_barrier_discards_pending_visual_updates_and_late_frames() {
    let (tx, rx) = output_mailbox();
    tx.send(frame(1)).unwrap();
    tx.send(RemoteDesktopOutput::FrameBgraRects {
        width: 1,
        height: 1,
        rects: vec![crate::runtime::RemoteDesktopFrameRect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            byte_len: 4,
        }],
        bgra: vec![1, 2, 3, 4],
    })
    .unwrap();
    tx.send(reconnecting()).unwrap();
    tx.send(frame(2)).unwrap();

    assert_eq!(Some(reconnecting()), rx.recv());
    drop(tx);
    assert_eq!(None, rx.recv());
}

#[test]
fn connected_after_reconnect_resumes_frames_and_preserves_control_order() {
    let (tx, rx) = output_mailbox();
    tx.send(RemoteDesktopOutput::Status("before".into()))
        .unwrap();
    tx.send(reconnecting()).unwrap();
    tx.send(RemoteDesktopOutput::ClipboardText {
        text: "during".into(),
    })
    .unwrap();
    tx.send(RemoteDesktopOutput::Connected {
        width: 2,
        height: 2,
        capabilities: crate::runtime::RemoteDesktopCapabilities::vnc_mvp(),
    })
    .unwrap();
    tx.send(frame(3)).unwrap();

    assert_eq!(
        Some(RemoteDesktopOutput::Status("before".into())),
        rx.recv()
    );
    assert_eq!(Some(reconnecting()), rx.recv());
    assert_eq!(
        Some(RemoteDesktopOutput::ClipboardText {
            text: "during".into()
        }),
        rx.recv()
    );
    assert!(matches!(
        rx.recv(),
        Some(RemoteDesktopOutput::Connected {
            width: 2,
            height: 2,
            ..
        })
    ));
    assert_eq!(Some(frame(3)), rx.recv());
}

fn reconnecting() -> RemoteDesktopOutput {
    RemoteDesktopOutput::Reconnecting(RemoteDesktopReconnect {
        reason: RemoteDesktopReconnectReason::ConnectionLost,
        delay_secs: Some(1),
    })
}

fn frame(value: u8) -> RemoteDesktopOutput {
    RemoteDesktopOutput::Frame {
        width: 1,
        height: 1,
        rgba: vec![value, 0, 0, 255],
    }
}
