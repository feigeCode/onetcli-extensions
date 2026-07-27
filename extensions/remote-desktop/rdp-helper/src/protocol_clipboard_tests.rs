use super::*;

#[test]
fn decodes_clipboard_files_with_transfer_id() {
    let request = decode_request_line(
        r#"{"type":"ClipboardFiles","transfer_id":17,"paths":["/tmp/report.txt"]}"#,
    )
    .expect("request decodes");

    assert_eq!(
        request,
        HelperRequest::ClipboardFiles {
            transfer_id: 17,
            paths: vec!["/tmp/report.txt".to_string()],
        }
    );
}

#[test]
fn legacy_clipboard_files_defaults_transfer_id_to_zero() {
    let request = decode_request_line(r#"{"type":"ClipboardFiles","paths":["/tmp/report.txt"]}"#)
        .expect("legacy request decodes");

    assert_eq!(
        request,
        HelperRequest::ClipboardFiles {
            transfer_id: 0,
            paths: vec!["/tmp/report.txt".to_string()],
        }
    );
}

#[test]
fn cancel_clipboard_transfer_round_trips() {
    let request = decode_request_line(r#"{"type":"CancelClipboardTransfer","transfer_id":17}"#)
        .expect("request decodes");

    assert_eq!(
        request,
        HelperRequest::CancelClipboardTransfer { transfer_id: 17 }
    );
    assert_eq!(
        serde_json::to_string(&request).expect("request encodes"),
        r#"{"type":"CancelClipboardTransfer","transfer_id":17}"#
    );
}

#[test]
fn encodes_clipboard_file_completion_events_for_main_process() {
    assert_eq!(
        encode_event_line(&HelperEvent::ClipboardFilesReady {
            transfer_id: (1_u64 << 63) | 5,
            paths: vec!["/tmp/navop-rdp-clipboard/transfer/report.txt".to_string()],
        })
        .expect("ready event encodes"),
        "{\"type\":\"ClipboardFilesReady\",\"transfer_id\":9223372036854775813,\"paths\":[\"/tmp/navop-rdp-clipboard/transfer/report.txt\"]}\n"
    );
    assert_eq!(
        encode_event_line(&HelperEvent::ClipboardTransferFailed {
            transfer_id: (1_u64 << 63) | 5,
            message: "transfer failed".to_string(),
        })
        .expect("failure event encodes"),
        "{\"type\":\"ClipboardTransferFailed\",\"transfer_id\":9223372036854775813,\"message\":\"transfer failed\"}\n"
    );
}

#[test]
fn clipboard_file_request_debug_does_not_expose_paths() {
    let request = HelperRequest::ClipboardFiles {
        transfer_id: 19,
        paths: vec!["/private/customer/report.txt".to_string()],
    };

    let debug = format!("{request:?}");

    assert!(debug.contains("transfer_id"));
    assert!(debug.contains("path_count"));
    assert!(!debug.contains("customer"));
    assert!(!debug.contains("report.txt"));
}

#[test]
fn helper_event_debug_redacts_payloads_paths_and_messages() {
    let events = [
        HelperEvent::Status {
            message: "sensitive-status".to_string(),
        },
        HelperEvent::frame(1, 1, b"frame-secret".to_vec()),
        HelperEvent::ClipboardText {
            text: "clipboard-secret".to_string(),
        },
        HelperEvent::ClipboardFilesReady {
            transfer_id: 7,
            paths: vec!["/private/customer/report.txt".to_string()],
        },
        HelperEvent::ClipboardTransferFailed {
            transfer_id: 7,
            message: "sensitive-transfer-error".to_string(),
        },
    ];

    for event in events {
        let debug = format!("{event:?}");
        assert!(!debug.contains("secret"));
        assert!(!debug.contains("customer"));
        assert!(!debug.contains("report.txt"));
        assert!(!debug.contains("sensitive"));
    }
}
