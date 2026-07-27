use std::path::PathBuf;

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
    assert_eq!(request.scale_factor, 100);
    assert!(!request.audio_playback);
    assert!(!request.audio_capture);
    assert!(request.shared_folders.is_empty());
}

#[test]
fn decodes_enabled_audio_playback_from_main_process() {
    let line = r#"{"type":"Connect","destination":"host:3389","username":null,"password":null,"domain":null,"width":1280,"height":720,"scale_factor":150,"audio_playback":true}"#;

    let request = connect_request(decode_request_line(line).expect("request decodes"))
        .expect("connect request");

    assert!(request.audio_playback);
    assert_eq!(request.scale_factor, 150);
}

#[test]
fn decodes_detailed_connect_options_from_main_process() {
    let line = r#"{"type":"Connect","destination":"host:3389","username":null,"password":null,"domain":null,"width":1280,"height":720,"scale_factor":150,"audio_playback":true,"audio_capture":false,"shared_folders":[{"name":"workspace","path":"/private/workspace","read_only":true}]}"#;

    let request = connect_request(decode_request_line(line).expect("request decodes"))
        .expect("connect request");

    assert!(!request.audio_capture);
    assert_eq!(
        request.shared_folders,
        vec![RemoteDesktopSharedFolder {
            name: "workspace".to_string(),
            path: PathBuf::from("/private/workspace"),
            read_only: true,
        }]
    );
}

#[test]
fn helper_request_debug_redacts_credentials_and_local_paths() {
    let request = HelperRequest::Connect {
        destination: "host:3389".to_string(),
        username: Some("administrator".to_string()),
        password: Some("top-secret".to_string()),
        domain: Some("customer-domain".to_string()),
        width: 1280,
        height: 720,
        scale_factor: 100,
        audio_playback: true,
        audio_capture: false,
        shared_folders: vec![RemoteDesktopSharedFolder {
            name: "workspace".to_string(),
            path: PathBuf::from("/private/customer-workspace"),
            read_only: true,
        }],
    };

    let debug = format!("{request:?}");

    assert!(debug.contains("username_present"));
    assert!(debug.contains("shared_folder_count"));
    assert!(!debug.contains("administrator"));
    assert!(!debug.contains("top-secret"));
    assert!(!debug.contains("customer-domain"));
    assert!(!debug.contains("workspace"));
    assert!(!debug.contains("customer-workspace"));
}

#[test]
fn connect_request_debug_redacts_credentials_and_local_paths() {
    let request = connect_request(HelperRequest::Connect {
        destination: "host:3389".to_string(),
        username: Some("administrator".to_string()),
        password: Some("top-secret".to_string()),
        domain: Some("customer-domain".to_string()),
        width: 1280,
        height: 720,
        scale_factor: 100,
        audio_playback: true,
        audio_capture: false,
        shared_folders: vec![RemoteDesktopSharedFolder {
            name: "workspace".to_string(),
            path: PathBuf::from("/private/customer-workspace"),
            read_only: true,
        }],
    })
    .expect("connect request");

    let debug = format!("{request:?}");

    assert!(debug.contains("username_present"));
    assert!(debug.contains("shared_folder_count"));
    assert!(!debug.contains("administrator"));
    assert!(!debug.contains("top-secret"));
    assert!(!debug.contains("customer-domain"));
    assert!(!debug.contains("workspace"));
    assert!(!debug.contains("customer-workspace"));
}

#[test]
fn request_debug_redacts_text_and_shared_folder_names() {
    let requests = [
        HelperRequest::Text {
            text: "typed-secret".to_string(),
        },
        HelperRequest::ClipboardText {
            text: "clipboard-secret".to_string(),
        },
    ];
    for request in requests {
        let debug = format!("{request:?}");
        assert!(debug.contains("text_len"));
        assert!(!debug.contains("secret"));
    }

    let folder = RemoteDesktopSharedFolder {
        name: "customer-workspace".to_string(),
        path: PathBuf::from("/private/customer-workspace"),
        read_only: false,
    };
    let debug = format!("{folder:?}");
    assert!(debug.contains("name_len"));
    assert!(!debug.contains("customer-workspace"));
}

#[test]
fn rejects_binary_frame_event_as_json_line() {
    let event = HelperEvent::frame(1, 1, vec![0x11, 0x22, 0x33, 0xff]);

    let error = encode_event_line(&event).expect_err("binary frame is not a JSON line");

    assert!(error.to_string().contains("write_event"));
}

#[test]
fn writes_binary_frame_event_shape_for_main_process() {
    let event = HelperEvent::frame(2, 1, vec![1, 2, 3, 255, 4, 5, 6, 255]);
    let mut output = Vec::new();

    write_event(&mut output, &event).expect("event writes");

    assert_eq!(
        output,
        b"{\"type\":\"FrameBgraBytes\",\"width\":2,\"height\":1,\"bgra_len\":8}\n\
          \x01\x02\x03\xff\x04\x05\x06\xff"
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
