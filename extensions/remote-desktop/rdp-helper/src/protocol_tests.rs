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
