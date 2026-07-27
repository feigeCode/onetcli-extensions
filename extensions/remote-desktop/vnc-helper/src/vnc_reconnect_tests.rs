use super::*;

#[test]
fn cleanup_failure_preserves_closed_result() {
    let result = merge_cleanup_error(VncSessionResult::Closed, anyhow::anyhow!("cleanup-secret"));

    assert!(matches!(result, VncSessionResult::Closed));
}

#[test]
fn cleanup_failure_preserves_input_closed_result() {
    let result = merge_cleanup_error(
        VncSessionResult::InputClosed,
        anyhow::anyhow!("cleanup-secret"),
    );

    assert!(matches!(result, VncSessionResult::InputClosed));
}

#[test]
fn cleanup_failure_preserves_reconnect_metadata() {
    let result = merge_cleanup_error(
        reconnect_result(
            RemoteDesktopReconnectReason::Manual,
            "reconnect-secret".to_string(),
            true,
            true,
        ),
        anyhow::anyhow!("cleanup-secret"),
    );

    let VncSessionResult::Reconnect {
        reason,
        diagnostic,
        manual,
        was_connected,
    } = result
    else {
        panic!("cleanup failure must preserve reconnect result");
    };
    assert_eq!(reason, RemoteDesktopReconnectReason::Manual);
    assert!(diagnostic.contains("reconnect-secret"));
    assert!(diagnostic.contains("cleanup-secret"));
    assert!(manual);
    assert!(was_connected);
}

#[test]
fn reconnect_wait_keeps_encoded_text_snapshot_and_ignores_files() {
    let (input_tx, mut input_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut snapshot =
        Some(VncClipboardSnapshot::encode("previous").expect("initial clipboard encodes"));

    input_tx
        .send(RemoteDesktopInput::ClipboardFiles {
            paths: vec![r"C:\Users\Rachel\notes.txt".to_string()],
        })
        .expect("file clipboard input sends");
    assert!(matches!(
        handle_wait_input(&mut input_rx, &mut snapshot),
        WaitAction::Continue
    ));
    assert_eq!(
        snapshot.as_ref().map(VncClipboardSnapshot::wire_bytes),
        Some(b"previous".as_slice())
    );

    input_tx
        .send(RemoteDesktopInput::ClipboardText {
            text: "café".to_string(),
        })
        .expect("text clipboard input sends");
    assert!(matches!(
        handle_wait_input(&mut input_rx, &mut snapshot),
        WaitAction::Continue
    ));
    assert_eq!(
        snapshot.as_ref().map(VncClipboardSnapshot::wire_bytes),
        Some(b"caf\xe9".as_slice())
    );

    input_tx
        .send(RemoteDesktopInput::ClipboardText {
            text: "中文".to_string(),
        })
        .expect("Unicode clipboard input sends");
    assert!(matches!(
        handle_wait_input(&mut input_rx, &mut snapshot),
        WaitAction::Continue
    ));
    assert_eq!(
        snapshot.as_ref().map(VncClipboardSnapshot::wire_bytes),
        Some(b"??".as_slice())
    );
}

#[test]
fn reconnect_wait_finds_close_behind_queued_pointer_noise() {
    let (input_tx, mut input_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut snapshot = None;
    for coordinate in 0..200 {
        input_tx
            .send(RemoteDesktopInput::MouseMove {
                x: coordinate,
                y: coordinate,
            })
            .expect("pointer input sends");
    }
    input_tx
        .send(RemoteDesktopInput::Close)
        .expect("close input sends");

    assert!(matches!(
        handle_wait_input(&mut input_rx, &mut snapshot),
        WaitAction::Stop
    ));
}
