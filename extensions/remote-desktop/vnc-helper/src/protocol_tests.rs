use super::*;

#[test]
fn decodes_connect_request_shape_from_main_process() {
    let line = r#"{"type":"Connect","destination":"10.2.178.12:3389","username":"administrator","password":"secret","domain":null,"width":1280,"height":720}"#;

    let request = connect_request(decode_request_line(line).expect("request decodes"))
        .expect("connect request");

    assert_eq!(request.destination, "10.2.178.12:3389");
    assert_eq!(request.username.as_deref(), Some("administrator"));
    assert_eq!(request.password.as_deref(), Some("secret"));
    assert_eq!(request.width, 1280);
    assert_eq!(request.height, 720);
}

#[test]
fn rejects_binary_frame_event_as_json_line() {
    let event = HelperEvent::frame(1, 1, vec![0x11, 0x22, 0x33, 0xff]);

    let error = encode_event_line(&event).expect_err("binary frame is not a JSON line");

    assert!(error.to_string().contains("write_event"));
}

#[test]
fn writes_binary_frame_event_shape_for_main_process() {
    let event = HelperEvent::frame(2, 1, vec![0x33, 0x22, 0x11, 0xff, 0xef, 0xcd, 0xab, 0xff]);
    let mut output = Vec::new();

    write_event(&mut output, &event).expect("event writes");

    assert_eq!(
        output,
        b"{\"type\":\"FrameBgraBytes\",\"width\":2,\"height\":1,\"bgra_len\":8}\n\
          \x33\x22\x11\xff\xef\xcd\xab\xff"
            .to_vec()
    );
}

#[test]
fn decodes_clipboard_text_request_shape_from_main_process() {
    let line = r#"{"type":"ClipboardText","text":"local 中文"}"#;

    let request = decode_request_line(line).expect("request decodes");

    assert_eq!(
        request,
        HelperRequest::ClipboardText {
            text: "local 中文".to_string()
        }
    );
}

#[test]
fn decodes_clipboard_files_request_shape_from_main_process() {
    let line = r#"{"type":"ClipboardFiles","paths":["C:\\Users\\Rachel\\notes.txt"]}"#;

    let request = decode_request_line(line).expect("request decodes");

    assert_eq!(
        request,
        HelperRequest::ClipboardFiles {
            paths: vec![r"C:\Users\Rachel\notes.txt".to_string()],
        }
    );
}

#[test]
fn decodes_keysym_request_shape_from_main_process() {
    let line = r#"{"type":"KeySym","keysym":58,"pressed":true}"#;

    let request = decode_request_line(line).expect("request decodes");

    assert_eq!(
        request,
        HelperRequest::KeySym {
            keysym: b':' as u32,
            pressed: true,
        }
    );
}

#[test]
fn encodes_clipboard_text_event_shape_for_main_process() {
    let event = HelperEvent::ClipboardText {
        text: "remote 中文".to_string(),
    };

    let line = encode_event_line(&event).expect("event encodes");

    assert_eq!(
        line,
        "{\"type\":\"ClipboardText\",\"text\":\"remote 中文\"}\n"
    );
}

#[test]
fn reconnect_event_round_trips_with_snake_case_reason() {
    let event = HelperEvent::Reconnecting {
        reason: HelperReconnectReason::ConnectionLost,
        delay_secs: Some(2),
    };

    let line = encode_event_line(&event).expect("reconnect event encodes");
    let decoded: HelperEvent = serde_json::from_str(line.trim_end()).expect("event decodes");

    assert_eq!(
        line,
        "{\"type\":\"Reconnecting\",\"reason\":\"connection_lost\",\"delay_secs\":2}\n"
    );
    assert_eq!(decoded, event);
}

#[test]
fn reconnect_event_supports_no_delay() {
    let event = HelperEvent::Reconnecting {
        reason: HelperReconnectReason::Manual,
        delay_secs: None,
    };

    assert_eq!(
        encode_event_line(&event).expect("manual reconnect event encodes"),
        "{\"type\":\"Reconnecting\",\"reason\":\"manual\",\"delay_secs\":null}\n"
    );
}

#[test]
fn request_debug_redacts_credentials_text_and_paths() {
    let connect = HelperRequest::Connect {
        destination: "host:5900".to_string(),
        username: Some("debug-user-secret".to_string()),
        password: Some("debug-password-secret".to_string()),
        domain: Some("debug-domain-secret".to_string()),
        width: 800,
        height: 600,
    };
    assert_redacted(
        &format!("{connect:?}"),
        &[
            "debug-user-secret",
            "debug-password-secret",
            "debug-domain-secret",
        ],
    );

    let text = HelperRequest::ClipboardText {
        text: "debug-clipboard-secret".to_string(),
    };
    assert_redacted(&format!("{text:?}"), &["debug-clipboard-secret"]);

    let files = HelperRequest::ClipboardFiles {
        paths: vec!["C:\\debug\\path-secret.txt".to_string()],
    };
    assert_redacted(&format!("{files:?}"), &["path-secret.txt"]);
}

#[test]
fn event_debug_redacts_messages_text_and_frame_bytes() {
    let events = [
        HelperEvent::Status {
            message: "debug-status-secret".to_string(),
        },
        HelperEvent::ClipboardText {
            text: "debug-remote-secret".to_string(),
        },
        HelperEvent::FrameBgraBytes {
            width: 1,
            height: 1,
            bgra: vec![17, 34, 51, 68],
        },
    ];

    for event in events {
        let debug = format!("{event:?}");
        assert_redacted(
            &debug,
            &[
                "debug-status-secret",
                "debug-remote-secret",
                "[17, 34, 51, 68]",
            ],
        );
    }
}

fn assert_redacted(debug: &str, secrets: &[&str]) {
    for secret in secrets {
        assert!(
            !debug.contains(secret),
            "Debug output leaked {secret:?}: {debug}"
        );
    }
}
