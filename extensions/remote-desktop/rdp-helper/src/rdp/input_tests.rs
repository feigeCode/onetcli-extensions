use super::*;

use crate::clipboard::text_clipboard;
use crate::output_mailbox::output_mailbox;
use ironrdp_client::rdp::RdpInputSender;

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
    let (input_tx, mut input_rx) = HelperInputSender::test_channel();
    let (output_tx, _output_rx) = output_mailbox();
    let (clipboard, _factory) = text_clipboard(input_tx.clone(), output_tx);
    let mut input_database = Database::new();
    let mut pending_mouse_position = None;
    let mut context = RdpInputContext::new(
        &input_tx,
        &mut input_database,
        &mut pending_mouse_position,
        &clipboard,
    );

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
    let (input_tx, mut input_rx) = HelperInputSender::test_channel();
    let (output_tx, _output_rx) = output_mailbox();
    let (clipboard, _factory) = text_clipboard(input_tx.clone(), output_tx);
    let mut input_database = Database::new();
    let mut pending_mouse_position = None;
    let mut context = RdpInputContext::new(
        &input_tx,
        &mut input_database,
        &mut pending_mouse_position,
        &clipboard,
    );

    let action = apply_input_request(HelperRequest::Close, &mut context).expect("close applies");

    assert_eq!(RdpInputAction::Close, action);
    assert!(input_rx.try_recv().is_err());
}

#[test]
fn shutdown_releases_held_inputs_before_transport_close() {
    let (input_tx, mut input_rx) = HelperInputSender::test_channel();
    let (output_tx, _output_rx) = output_mailbox();
    let (clipboard, _factory) = text_clipboard(input_tx.clone(), output_tx);
    let mut input_database = Database::new();
    let mut pending_mouse_position = None;
    let mut context = RdpInputContext::new(
        &input_tx,
        &mut input_database,
        &mut pending_mouse_position,
        &clipboard,
    );
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

#[test]
fn shutdown_does_not_wait_for_a_full_input_queue() {
    let (sender, _receiver) = RdpInputSender::channel(1);
    sender
        .try_send(RdpInputEvent::Resize {
            width: 800,
            height: 600,
            scale_factor: 100,
            physical_size: None,
        })
        .expect("fill input queue");
    let input_tx = HelperInputSender::production(sender);
    let mut input_database = Database::new();

    shutdown_inputs(&mut input_database, &input_tx)
        .expect("graceful close bypasses the full ordinary-input queue");
}

#[test]
fn full_input_queue_coalesces_mouse_move_until_next_pointer_action() {
    let (sender, mut receiver) = RdpInputSender::channel(1);
    sender
        .try_send(RdpInputEvent::Resize {
            width: 800,
            height: 600,
            scale_factor: 100,
            physical_size: None,
        })
        .expect("fill input queue");
    let input_tx = HelperInputSender::production(sender);
    let (output_tx, _output_rx) = output_mailbox();
    let (clipboard, _factory) = text_clipboard(input_tx.clone(), output_tx);
    let mut input_database = Database::new();
    let mut pending_mouse_position = None;
    let mut context = RdpInputContext::new(
        &input_tx,
        &mut input_database,
        &mut pending_mouse_position,
        &clipboard,
    );

    let action = apply_input_request(HelperRequest::MouseMove { x: 640, y: 480 }, &mut context)
        .expect("mouse move is best effort");
    apply_input_request(HelperRequest::MouseMove { x: 700, y: 500 }, &mut context)
        .expect("newer mouse move replaces the pending position");

    assert_eq!(RdpInputAction::Continue, action);
    assert_eq!(
        MousePosition { x: 0, y: 0 },
        context.database.mouse_position()
    );
    assert_eq!(
        Some(MousePosition { x: 700, y: 500 }),
        *context.pending_mouse_position
    );
    assert!(matches!(
        receiver.try_recv().expect("queue filler"),
        RdpInputEvent::Resize { .. }
    ));

    apply_input_request(
        HelperRequest::MouseButton {
            button: crate::protocol::HelperMouseButton::Left,
            pressed: true,
        },
        &mut context,
    )
    .expect("pointer action flushes the latest pending mouse move");

    assert_eq!(
        MousePosition { x: 700, y: 500 },
        context.database.mouse_position()
    );
    assert!(context.database.is_mouse_button_pressed(MouseButton::Left));
    assert!(context.pending_mouse_position.is_none());
    match receiver.try_recv().expect("coalesced pointer input") {
        RdpInputEvent::FastPath(events) => assert_eq!(2, events.len()),
        other => panic!("expected coalesced pointer input, got {other:?}"),
    }
}

#[test]
fn resize_discards_a_mouse_move_coalesced_for_the_previous_desktop_size() {
    let (sender, mut receiver) = RdpInputSender::channel(1);
    sender
        .try_send(RdpInputEvent::Resize {
            width: 800,
            height: 600,
            scale_factor: 100,
            physical_size: None,
        })
        .expect("fill input queue");
    let input_tx = HelperInputSender::production(sender);
    let (output_tx, _output_rx) = output_mailbox();
    let (clipboard, _factory) = text_clipboard(input_tx.clone(), output_tx);
    let mut input_database = Database::new();
    let mut pending_mouse_position = None;
    let mut context = RdpInputContext::new(
        &input_tx,
        &mut input_database,
        &mut pending_mouse_position,
        &clipboard,
    );

    apply_input_request(HelperRequest::MouseMove { x: 700, y: 500 }, &mut context)
        .expect("mouse move is coalesced");
    assert_eq!(
        Some(MousePosition { x: 700, y: 500 }),
        *context.pending_mouse_position
    );
    assert!(matches!(
        receiver.try_recv().expect("queue filler"),
        RdpInputEvent::Resize { .. }
    ));

    apply_input_request(
        HelperRequest::Resize {
            width: 1_600,
            height: 1_200,
            scale_factor: 100,
        },
        &mut context,
    )
    .expect("resize applies");

    assert!(context.pending_mouse_position.is_none());
    assert_eq!(
        MousePosition { x: 0, y: 0 },
        context.database.mouse_position()
    );
    assert!(matches!(
        receiver.try_recv().expect("new resize"),
        RdpInputEvent::Resize {
            width: 1_600,
            height: 1_200,
            ..
        }
    ));
}

#[test]
fn stateful_input_waits_for_capacity_before_mutating_local_state() {
    let (sender, mut receiver) = RdpInputSender::channel(1);
    sender
        .try_send(RdpInputEvent::Resize {
            width: 800,
            height: 600,
            scale_factor: 100,
            physical_size: None,
        })
        .expect("fill input queue");
    let input_tx = HelperInputSender::production(sender);
    let (output_tx, _output_rx) = output_mailbox();
    let (clipboard, _factory) = text_clipboard(input_tx.clone(), output_tx);
    let mut input_database = Database::new();
    let mut pending_mouse_position = None;
    let scancode = Scancode::from_u8(false, 0x39);
    let drain = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(5));
        let filler = receiver.blocking_recv().expect("queue filler");
        let key_press = receiver.blocking_recv().expect("queued key press");
        (filler, key_press)
    });
    let mut context = RdpInputContext::new(
        &input_tx,
        &mut input_database,
        &mut pending_mouse_position,
        &clipboard,
    );

    send_operations(&mut context, [Operation::KeyPressed(scancode)])
        .expect("key press waits for queue capacity");

    assert!(input_database.is_key_pressed(scancode));
    let (filler, key_press) = drain.join().expect("queue drain thread");
    assert!(matches!(filler, RdpInputEvent::Resize { .. }));
    assert!(matches!(key_press, RdpInputEvent::FastPath(_)));
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
