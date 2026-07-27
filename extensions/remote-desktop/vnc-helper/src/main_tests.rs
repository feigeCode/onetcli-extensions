use super::*;

#[test]
fn reads_first_stdin_line_as_connect_request() {
    let input = vec![Ok(
        r#"{"type":"Connect","destination":"host:5900","username":null,"password":"secret","domain":null,"width":800,"height":600}"#
            .to_string(),
    )];
    let mut lines = input.into_iter();

    let request = read_connect_request(&mut lines).expect("connect request");

    assert_eq!(request.destination, "host:5900");
    assert_eq!(request.password.as_deref(), Some("secret"));
}

#[test]
fn converts_extended_key_request_to_prefixed_scancode() {
    let input = request_to_input(HelperRequest::Key {
        code: 0x48,
        extended: true,
        pressed: true,
    });

    assert_eq!(
        input,
        Some(RemoteDesktopInput::Key {
            key: RemoteKey::Scancode(0xe048),
            pressed: true
        })
    );
}

#[test]
fn converts_keysym_request_to_remote_keysym() {
    let input = request_to_input(HelperRequest::KeySym {
        keysym: b':' as u32,
        pressed: true,
    });

    assert_eq!(
        input,
        Some(RemoteDesktopInput::Key {
            key: RemoteKey::KeySym(b':' as u32),
            pressed: true,
        })
    );
}

#[test]
fn converts_clipboard_files_request_without_losing_paths() {
    let input = request_to_input(HelperRequest::ClipboardFiles {
        paths: vec![r"C:\Users\Rachel\notes.txt".to_string()],
    });

    assert_eq!(
        input,
        Some(RemoteDesktopInput::ClipboardFiles {
            paths: vec![r"C:\Users\Rachel\notes.txt".to_string()],
        })
    );
}
