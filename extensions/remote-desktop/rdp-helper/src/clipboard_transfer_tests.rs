use super::*;
use ironrdp::cliprdr::backend::ClipboardMessage;
use ironrdp::cliprdr::pdu::{
    ClipboardFileAttributes, ClipboardFormat, ClipboardFormatId, ClipboardFormatName,
    FileContentsFlags, FileContentsRequest, FileContentsResponse, FileDescriptor,
};
use ironrdp_client::rdp::RdpInputEvent;
use tokio::sync::mpsc;

use crate::output_mailbox::output_mailbox;
use crate::protocol::HelperEvent;
use crate::rdp::HelperInputSender;

const LOCAL_TRANSFER_ID: u64 = 41;
const FIRST_REMOTE_TRANSFER_ID: u64 = (1_u64 << 63) | 1;

#[test]
fn local_directory_copy_preserves_hierarchy_and_descriptor_indexes() {
    let source = tempfile::tempdir().expect("source directory");
    let top = source.path().join("project");
    let nested = top.join("src");
    std::fs::create_dir_all(&nested).expect("nested directory");
    std::fs::write(top.join("README.md"), b"readme").expect("top-level file");
    std::fs::write(nested.join("main.rs"), b"fn main() {}").expect("nested file");

    let (input_tx, mut input_rx) = HelperInputSender::test_channel();
    let (output_tx, _output_rx) = output_mailbox();
    let staging = tempfile::tempdir().expect("staging root");
    let (controller, factory) =
        text_clipboard_at(input_tx, output_tx, staging.path().to_path_buf());

    controller
        .set_local_files(LOCAL_TRANSFER_ID, vec![top.to_string_lossy().into_owned()])
        .expect("directory clipboard starts");

    let descriptors = match input_rx.try_recv().expect("file copy event") {
        RdpInputEvent::Clipboard(ClipboardMessage::SendInitiateFileCopy(files)) => files,
        other => panic!("expected file copy event, got {other:?}"),
    };
    assert_eq!(4, descriptors.len());
    assert_descriptor(&descriptors[0], "project", None, true);
    assert_descriptor(&descriptors[1], "README.md", Some("project"), false);
    assert_descriptor(&descriptors[2], "src", Some("project"), true);
    assert_descriptor(&descriptors[3], "main.rs", Some("project\\src"), false);

    let mut backend = factory.build_cliprdr_backend();
    backend.on_file_contents_request(file_request(8, 3, 3, 4));
    match input_rx.try_recv().expect("file response") {
        RdpInputEvent::Clipboard(ClipboardMessage::SendFileContentsResponse(response)) => {
            assert!(!response.is_error());
            assert_eq!(b"main", response.data());
        }
        other => panic!("expected file response, got {other:?}"),
    }

    backend.on_file_contents_request(file_request(9, 0, 0, 1));
    match input_rx.try_recv().expect("directory error response") {
        RdpInputEvent::Clipboard(ClipboardMessage::SendFileContentsResponse(response)) => {
            assert!(response.is_error());
        }
        other => panic!("expected directory error response, got {other:?}"),
    }
}

#[test]
fn cancelling_local_transfer_stops_serving_its_snapshot() {
    let file = tempfile::NamedTempFile::new().expect("source file");
    std::fs::write(file.path(), b"secret").expect("file contents");
    let (input_tx, mut input_rx) = HelperInputSender::test_channel();
    let (output_tx, _output_rx) = output_mailbox();
    let staging = tempfile::tempdir().expect("staging root");
    let (controller, factory) =
        text_clipboard_at(input_tx, output_tx, staging.path().to_path_buf());
    controller
        .set_local_files(
            LOCAL_TRANSFER_ID,
            vec![file.path().to_string_lossy().into_owned()],
        )
        .expect("file clipboard starts");
    let _ = input_rx.try_recv();

    assert!(controller.cancel_transfer(LOCAL_TRANSFER_ID));
    let mut backend = factory.build_cliprdr_backend();
    backend.on_file_contents_request(file_request(7, 0, 0, 6));

    match input_rx.try_recv().expect("cancelled response") {
        RdpInputEvent::Clipboard(ClipboardMessage::SendFileContentsResponse(response)) => {
            assert!(response.is_error());
        }
        other => panic!("expected cancelled file response, got {other:?}"),
    }
}

#[test]
fn remote_directory_download_streams_files_and_emits_staged_top_level_path() {
    let (input_tx, mut input_rx) = HelperInputSender::test_channel();
    let (output_tx, output_rx) = output_mailbox();
    let staging_parent = tempfile::tempdir().expect("staging parent");
    let staging_root = staging_parent.path().join("navop-rdp-clipboard");
    let (_controller, factory) = text_clipboard_at(input_tx, output_tx, staging_root.clone());
    let mut backend = factory.build_cliprdr_backend();

    backend.on_remote_copy(&[remote_file_format()]);
    assert_initiate_file_paste(&mut input_rx);

    backend.on_remote_file_list(
        &[
            directory_descriptor("project", None),
            directory_descriptor("src", Some("project")),
            file_descriptor("main.rs", Some("project\\src"), 4),
        ],
        None,
    );

    let size_request = next_file_request(&mut input_rx);
    assert_eq!(2, size_request.index);
    assert_eq!(FileContentsFlags::SIZE, size_request.flags);
    backend.on_file_contents_response(FileContentsResponse::new_size_response(
        size_request.stream_id,
        4,
    ));

    let range_request = next_file_request(&mut input_rx);
    assert_eq!(2, range_request.index);
    assert_eq!(FileContentsFlags::RANGE, range_request.flags);
    backend.on_file_contents_response(FileContentsResponse::new_data_response(
        range_request.stream_id,
        b"main".as_slice(),
    ));

    let ready_paths = match output_rx.recv().expect("clipboard ready event") {
        HelperEvent::ClipboardFilesReady { transfer_id, paths } => {
            assert_eq!(FIRST_REMOTE_TRANSFER_ID, transfer_id);
            paths
        }
        other => panic!("expected clipboard ready event, got {other:?}"),
    };
    assert_eq!(1, ready_paths.len());
    let top = PathBuf::from(&ready_paths[0]);
    assert!(top.is_absolute());
    assert!(top.starts_with(&staging_root));
    assert_eq!(
        b"main".to_vec(),
        std::fs::read(top.join("src").join("main.rs")).expect("staged file")
    );
}

#[test]
fn invalid_remote_hierarchy_fails_without_leaving_partial_staging() {
    let (input_tx, mut input_rx) = HelperInputSender::test_channel();
    let (output_tx, output_rx) = output_mailbox();
    let staging_parent = tempfile::tempdir().expect("staging parent");
    let staging_root = staging_parent.path().join("navop-rdp-clipboard");
    let (_controller, factory) = text_clipboard_at(input_tx, output_tx, staging_root.clone());
    let mut backend = factory.build_cliprdr_backend();
    backend.on_remote_copy(&[remote_file_format()]);
    assert_initiate_file_paste(&mut input_rx);

    backend.on_remote_file_list(&[file_descriptor("escape.txt", Some("missing"), 1)], None);

    match output_rx.recv().expect("clipboard failure event") {
        HelperEvent::ClipboardTransferFailed { transfer_id, .. } => {
            assert_eq!(FIRST_REMOTE_TRANSFER_ID, transfer_id);
        }
        other => panic!("expected clipboard failure event, got {other:?}"),
    }
    assert!(
        !staging_root.exists()
            || std::fs::read_dir(staging_root)
                .expect("staging root reads")
                .next()
                .is_none()
    );
}

#[test]
fn remote_file_transfer_reports_when_clipboard_channel_closes() {
    let (input_tx, mut input_rx) = HelperInputSender::test_channel();
    let (output_tx, output_rx) = output_mailbox();
    let staging_parent = tempfile::tempdir().expect("staging parent");
    let staging_root = staging_parent.path().join("navop-rdp-clipboard");
    let (controller, factory) = text_clipboard_at(input_tx, output_tx, staging_root);
    let mut backend = factory.build_cliprdr_backend();

    backend.on_remote_copy(&[remote_file_format()]);
    assert_initiate_file_paste(&mut input_rx);
    drop(input_rx);

    backend.on_remote_file_list(&[file_descriptor("remote.txt", None, 4)], Some(7));

    match output_rx.recv().expect("clipboard failure event") {
        HelperEvent::ClipboardTransferFailed {
            transfer_id,
            message,
        } => {
            assert_eq!(FIRST_REMOTE_TRANSFER_ID, transfer_id);
            assert!(message.contains("clipboard channel closed"));
        }
        other => panic!("expected clipboard failure event, got {other:?}"),
    }
    let state = controller::lock_state(&controller.shared);
    assert!(state.pending_remote.is_none());
    assert!(state.remote_transfer.is_none());
}

fn assert_descriptor(
    descriptor: &FileDescriptor,
    name: &str,
    relative_path: Option<&str>,
    directory: bool,
) {
    assert_eq!(name, descriptor.name);
    assert_eq!(relative_path, descriptor.relative_path.as_deref());
    let attributes = descriptor.attributes.expect("descriptor attributes");
    assert_eq!(
        directory,
        attributes.contains(ClipboardFileAttributes::DIRECTORY)
    );
}

fn file_request(
    stream_id: u32,
    index: i32,
    position: u64,
    requested_size: u32,
) -> FileContentsRequest {
    FileContentsRequest {
        stream_id,
        index,
        flags: FileContentsFlags::RANGE,
        position,
        requested_size,
        data_id: None,
    }
}

fn remote_file_format() -> ClipboardFormat {
    ClipboardFormat::new(ClipboardFormatId::new(0xC001)).with_name(ClipboardFormatName::FILE_LIST)
}

fn directory_descriptor(name: &str, relative_path: Option<&str>) -> FileDescriptor {
    let descriptor = FileDescriptor::new(name).with_attributes(ClipboardFileAttributes::DIRECTORY);
    match relative_path {
        Some(path) => descriptor.with_relative_path(path),
        None => descriptor,
    }
}

fn file_descriptor(name: &str, relative_path: Option<&str>, size: u64) -> FileDescriptor {
    let descriptor = FileDescriptor::new(name)
        .with_attributes(ClipboardFileAttributes::ARCHIVE)
        .with_file_size(size);
    match relative_path {
        Some(path) => descriptor.with_relative_path(path),
        None => descriptor,
    }
}

fn assert_initiate_file_paste(input_rx: &mut mpsc::UnboundedReceiver<RdpInputEvent>) {
    match input_rx.try_recv().expect("file paste request") {
        RdpInputEvent::Clipboard(ClipboardMessage::SendInitiatePaste(format)) => {
            assert_eq!(ClipboardFormatId::new(0xC001), format);
        }
        other => panic!("expected file paste request, got {other:?}"),
    }
}

fn next_file_request(input_rx: &mut mpsc::UnboundedReceiver<RdpInputEvent>) -> FileContentsRequest {
    match input_rx.try_recv().expect("file contents request") {
        RdpInputEvent::Clipboard(ClipboardMessage::SendFileContentsRequest(request)) => request,
        other => panic!("expected file contents request, got {other:?}"),
    }
}
