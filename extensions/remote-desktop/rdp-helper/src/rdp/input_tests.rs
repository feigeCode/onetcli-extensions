use super::*;

use crate::clipboard::text_clipboard;
use crate::output_mailbox::output_mailbox;

#[test]
fn key_operation_builds_plain_scancode_events() {
    let operation = key_operation(0x39, false, true).expect("space key operation");
    assert_key_operation(operation, (true, false, 0x39));
}

#[test]
fn key_operation_builds_extended_scancode_events() {
    let operation = key_operation(0x48, true, false).expect("arrow key operation");
    assert_key_operation(operation, (false, true, 0x48));
}

#[test]
fn key_operation_rejects_out_of_range_scancode() {
    assert!(key_operation(0x100, false, true).is_err());
}

#[test]
fn apply_clipboard_text_request_advertises_local_text() {
    let (input_tx, mut input_rx) = RdpInputEvent::create_channel();
    let (output_tx, _output_rx) = output_mailbox();
    let (clipboard, _factory) = text_clipboard(input_tx.clone(), output_tx);
    let mut input_database = Database::new();
    let mut context = RdpInputContext::new(&input_tx, &mut input_database, &clipboard);

    let action = apply_input_request(
        HelperRequest::ClipboardText {
            text: "local 中文".to_string(),
        },
        &mut context,
    )
    .expect("clipboard request applies");

    assert_eq!(RdpInputAction::Continue, action);
    match input_rx.try_recv().expect("clipboard advertise") {
        RdpInputEvent::Clipboard(
            ironrdp::cliprdr::backend::ClipboardMessage::SendInitiateCopy(formats),
        ) => assert!(formats.iter().any(|format| {
            format.id() == ironrdp::cliprdr::pdu::ClipboardFormatId::CF_UNICODETEXT
        })),
        other => panic!("expected clipboard advertise, got {other:?}"),
    }
}

#[test]
fn close_request_defers_transport_close_to_session_shutdown() {
    let (input_tx, mut input_rx) = RdpInputEvent::create_channel();
    let (output_tx, _output_rx) = output_mailbox();
    let (clipboard, _factory) = text_clipboard(input_tx.clone(), output_tx);
    let mut input_database = Database::new();
    let mut context = RdpInputContext::new(&input_tx, &mut input_database, &clipboard);

    let action = apply_input_request(HelperRequest::Close, &mut context).expect("close applies");

    assert_eq!(RdpInputAction::Close, action);
    assert!(input_rx.try_recv().is_err());
}

#[test]
fn shutdown_releases_held_inputs_before_transport_close() {
    let (input_tx, mut input_rx) = RdpInputEvent::create_channel();
    let (output_tx, _output_rx) = output_mailbox();
    let (clipboard, _factory) = text_clipboard(input_tx.clone(), output_tx);
    let mut input_database = Database::new();
    let mut context = RdpInputContext::new(&input_tx, &mut input_database, &clipboard);
    send_operations(
        &mut context,
        [Operation::KeyPressed(Scancode::from_u8(false, 0x39))],
    )
    .expect("key press");
    send_operations(
        &mut context,
        [Operation::MouseButtonPressed(MouseButton::Left)],
    )
    .expect("mouse press");
    input_rx.try_recv().expect("key press event");
    input_rx.try_recv().expect("mouse press event");

    shutdown_inputs(&mut input_database, &input_tx).expect("input shutdown");

    match input_rx.try_recv().expect("release event") {
        RdpInputEvent::FastPath(events) => assert_eq!(2, events.len()),
        other => panic!("expected releases, got {other:?}"),
    }
    assert!(matches!(
        input_rx.try_recv().expect("close event"),
        RdpInputEvent::Close
    ));
    assert!(input_database.release_all().is_empty());
}

fn assert_key_operation(
    operation: Operation,
    (expected_pressed, expected_extended, expected_code): (bool, bool, u8),
) {
    let (pressed, scancode) = match operation {
        Operation::KeyPressed(scancode) => (true, scancode),
        Operation::KeyReleased(scancode) => (false, scancode),
        other => panic!("expected key operation, got {other:?}"),
    };
    assert_eq!(expected_pressed, pressed);
    assert_eq!((expected_extended, expected_code), scancode.as_u8());
}
