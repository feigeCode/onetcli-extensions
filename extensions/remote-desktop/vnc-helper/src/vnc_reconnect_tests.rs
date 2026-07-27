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
fn reconnect_wait_ignores_files_and_non_ascii_clipboard_text() {
    let (input_tx, mut input_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut latest_clipboard_text = Some("previous".to_string());

    input_tx
        .send(RemoteDesktopInput::ClipboardFiles {
            paths: vec![r"C:\Users\Rachel\notes.txt".to_string()],
        })
        .expect("file clipboard input sends");
    assert!(matches!(
        handle_wait_input(&mut input_rx, &mut latest_clipboard_text),
        WaitAction::Continue
    ));
    assert_eq!(latest_clipboard_text.as_deref(), Some("previous"));

    input_tx
        .send(RemoteDesktopInput::ClipboardText {
            text: "中文".to_string(),
        })
        .expect("text clipboard input sends");
    assert!(matches!(
        handle_wait_input(&mut input_rx, &mut latest_clipboard_text),
        WaitAction::Continue
    ));
    assert_eq!(latest_clipboard_text.as_deref(), Some("previous"));

    input_tx
        .send(RemoteDesktopInput::ClipboardText {
            text: "next".to_string(),
        })
        .expect("ASCII clipboard input sends");
    assert!(matches!(
        handle_wait_input(&mut input_rx, &mut latest_clipboard_text),
        WaitAction::Continue
    ));
    assert_eq!(latest_clipboard_text.as_deref(), Some("next"));
}
