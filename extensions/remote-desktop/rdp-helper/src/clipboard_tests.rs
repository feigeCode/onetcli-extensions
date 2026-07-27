use super::*;
use ironrdp::cliprdr::backend::ClipboardMessage;
use ironrdp::cliprdr::pdu::{ClipboardFormatId, FileContentsFlags, FileContentsRequest};
use ironrdp::cliprdr::pdu::{FormatDataRequest, FormatDataResponse};
use ironrdp_client::rdp::RdpInputEvent;

use crate::output_mailbox::output_mailbox;
use crate::protocol::HelperEvent;

#[test]
fn local_text_advertises_unicode_clipboard_format() {
    let (input_tx, mut input_rx) = RdpInputEvent::create_channel();
    let (output_tx, _output_rx) = output_mailbox();
    let (controller, _factory) = text_clipboard(input_tx, output_tx);

    controller
        .set_local_text("hello 中文".to_string())
        .expect("local clipboard advertises");

    match input_rx.try_recv().expect("clipboard message") {
        RdpInputEvent::Clipboard(ClipboardMessage::SendInitiateCopy(formats)) => {
            assert_eq!(
                vec![ClipboardFormatId::CF_UNICODETEXT],
                format_ids(&formats)
            );
        }
        other => panic!("expected clipboard advertise, got {other:?}"),
    }
}

#[test]
fn empty_local_clipboard_advertises_empty_format_list_during_initialization() {
    let (input_tx, mut input_rx) = RdpInputEvent::create_channel();
    let (output_tx, _output_rx) = output_mailbox();
    let (_controller, factory) = text_clipboard(input_tx, output_tx);
    let mut backend = factory.build_cliprdr_backend();

    backend.on_request_format_list();

    match input_rx.try_recv().expect("initial clipboard format list") {
        RdpInputEvent::Clipboard(ClipboardMessage::SendInitiateCopy(formats)) => {
            assert!(formats.is_empty());
        }
        other => panic!("expected empty clipboard advertise, got {other:?}"),
    }
}

#[test]
fn local_files_start_streaming_clipboard_copy() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), b"navop-file").unwrap();
    let (input_tx, mut input_rx) = RdpInputEvent::create_channel();
    let (output_tx, _output_rx) = output_mailbox();
    let (controller, _factory) = text_clipboard(input_tx, output_tx);

    controller
        .set_local_files(1, vec![file.path().to_string_lossy().into_owned()])
        .expect("local file copy starts");

    match input_rx.try_recv().expect("file copy event") {
        RdpInputEvent::ClipboardFileCopy(files) => {
            assert_eq!(1, files.len());
            assert_eq!(Some(10), files[0].file_size);
        }
        other => panic!("expected file copy event, got {other:?}"),
    }
}

#[test]
fn reads_requested_file_range_without_loading_whole_file() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), b"0123456789").unwrap();
    let request = FileContentsRequest {
        stream_id: 7,
        index: 0,
        flags: FileContentsFlags::RANGE,
        position: 3,
        requested_size: 4,
        data_id: None,
    };

    let response = read_file_contents(&file.path().to_path_buf(), &request).unwrap();

    assert_eq!(7, response.stream_id());
    assert_eq!(b"3456", response.data());
}

#[test]
fn backend_replies_with_local_text_when_remote_requests_unicode_data() {
    let (input_tx, mut input_rx) = RdpInputEvent::create_channel();
    let (output_tx, _output_rx) = output_mailbox();
    let (controller, factory) = text_clipboard(input_tx, output_tx);
    controller
        .set_local_text("hello 中文".to_string())
        .expect("local clipboard advertises");
    let _ = input_rx.try_recv();
    let mut backend = factory.build_cliprdr_backend();

    backend.on_format_data_request(FormatDataRequest {
        format: ClipboardFormatId::CF_UNICODETEXT,
    });

    match input_rx.try_recv().expect("format data response") {
        RdpInputEvent::Clipboard(ClipboardMessage::SendFormatData(response)) => {
            assert_eq!(
                "hello 中文",
                response.to_unicode_string().expect("unicode text decodes")
            );
        }
        other => panic!("expected clipboard data response, got {other:?}"),
    }
}

#[test]
fn backend_fetches_and_emits_remote_unicode_clipboard_text() {
    let (input_tx, mut input_rx) = RdpInputEvent::create_channel();
    let (output_tx, output_rx) = output_mailbox();
    let (_controller, factory) = text_clipboard(input_tx, output_tx);
    let mut backend = factory.build_cliprdr_backend();

    backend.on_remote_copy(&[ironrdp::cliprdr::pdu::ClipboardFormat::new(
        ClipboardFormatId::CF_UNICODETEXT,
    )]);

    match input_rx.try_recv().expect("paste request") {
        RdpInputEvent::Clipboard(ClipboardMessage::SendInitiatePaste(format)) => {
            assert_eq!(ClipboardFormatId::CF_UNICODETEXT, format);
        }
        other => panic!("expected paste request, got {other:?}"),
    }

    backend.on_format_data_response(FormatDataResponse::new_unicode_string("remote 中文"));

    assert_eq!(
        HelperEvent::ClipboardText {
            text: "remote 中文".to_string()
        },
        output_rx.recv().expect("clipboard event")
    );
}

#[test]
fn shutdown_clears_all_clipboard_snapshots_and_pending_state() {
    let (input_tx, _input_rx) = RdpInputEvent::create_channel();
    let (output_tx, _output_rx) = output_mailbox();
    let (controller, _factory) = text_clipboard(input_tx, output_tx);
    {
        let mut state = controller::lock_state(&controller.shared);
        state.local_text = Some("sensitive text".to_string());
        state.waiting_remote_text = true;
        state.pending_remote = Some(REMOTE_CLIPBOARD_TRANSFER_BIT | 7);
    }

    controller.shutdown();

    let state = controller::lock_state(&controller.shared);
    assert!(state.local_text.is_none());
    assert!(state.local_files.is_none());
    assert!(state.locked_local_files.is_empty());
    assert!(state.pending_remote.is_none());
    assert!(state.remote_transfer.is_none());
    assert!(!state.waiting_remote_text);
}

fn format_ids(formats: &[ironrdp::cliprdr::pdu::ClipboardFormat]) -> Vec<ClipboardFormatId> {
    formats.iter().map(|format| format.id()).collect()
}
