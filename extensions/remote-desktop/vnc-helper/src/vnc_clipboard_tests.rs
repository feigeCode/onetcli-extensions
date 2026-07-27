use super::*;

#[test]
fn encodes_ascii_without_changes() {
    let snapshot = VncClipboardSnapshot::encode("plain text").expect("ASCII text is supported");

    assert_eq!(snapshot.wire_bytes(), b"plain text");
}

#[test]
fn encodes_latin1_as_single_byte_codepoints() {
    let snapshot = VncClipboardSnapshot::encode("café").expect("Latin-1 text is supported");

    assert_eq!(snapshot.wire_bytes(), b"caf\xe9");
}

#[test]
fn replaces_non_latin1_codepoints_deterministically() {
    let snapshot = VncClipboardSnapshot::encode("中文 🖥").expect("text is safely downgraded");

    assert_eq!(snapshot.wire_bytes(), b"?? ?");
}

#[test]
fn decodes_strict_utf8_before_trying_legacy_latin1() {
    let utf8 = "中文".as_bytes();

    assert_eq!(decode_clipboard_text(utf8).as_deref(), Some("中文"));
}

#[test]
fn decodes_invalid_utf8_as_legacy_latin1() {
    assert_eq!(decode_clipboard_text(b"caf\xe9").as_deref(), Some("café"));
}

#[test]
fn rejects_payloads_above_the_wire_limit() {
    let oversized = "a".repeat(MAX_CUT_TEXT_BYTES + 1);

    assert_eq!(VncClipboardSnapshot::encode(&oversized), None);
    assert_eq!(
        decode_clipboard_text(&vec![b'a'; MAX_CUT_TEXT_BYTES + 1]),
        None
    );
}
