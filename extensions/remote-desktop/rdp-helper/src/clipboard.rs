#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ironrdp::cliprdr::backend::CliprdrBackendFactory;
use ironrdp::cliprdr::pdu::{ClipboardFormat, ClipboardFormatId};

use crate::output_mailbox::OutputSender;
use crate::rdp::HelperInputSender;

use self::backend::TextClipboardBackendFactory;
use self::controller::TextClipboardState;
#[cfg(test)]
use self::local::LocalClipboardEntry;

mod backend;
mod backend_remote;
mod controller;
mod local;
mod path;
mod remote;
mod remote_layout;
mod staging;

pub(crate) const REMOTE_CLIPBOARD_TRANSFER_BIT: u64 = 1 << 63;
pub(crate) const LOCAL_CLIPBOARD_TRANSFER_MASK: u64 = REMOTE_CLIPBOARD_TRANSFER_BIT - 1;
pub(crate) const FIRST_SEQUENCE_ID: u64 = 1;

pub use controller::TextClipboardController;

pub fn text_clipboard(
    input_tx: HelperInputSender,
    output_tx: OutputSender,
) -> (
    TextClipboardController,
    Box<dyn CliprdrBackendFactory + Send>,
) {
    let staging_root = std::env::temp_dir().join("navop-rdp-clipboard");
    build_text_clipboard(input_tx, output_tx, staging_root)
}

fn build_text_clipboard(
    input_tx: HelperInputSender,
    output_tx: OutputSender,
    staging_root: PathBuf,
) -> (
    TextClipboardController,
    Box<dyn CliprdrBackendFactory + Send>,
) {
    let shared = Arc::new(Mutex::new(TextClipboardState::new()));
    let controller = TextClipboardController {
        shared: shared.clone(),
        input_tx: input_tx.clone(),
        output_tx: output_tx.clone(),
    };
    let factory = TextClipboardBackendFactory::new(shared, input_tx, output_tx, staging_root);
    (controller, Box::new(factory))
}

fn text_formats() -> Vec<ClipboardFormat> {
    vec![ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]
}

#[cfg(test)]
fn text_clipboard_at(
    input_tx: HelperInputSender,
    output_tx: OutputSender,
    staging_root: PathBuf,
) -> (
    TextClipboardController,
    Box<dyn CliprdrBackendFactory + Send>,
) {
    build_text_clipboard(input_tx, output_tx, staging_root)
}

#[cfg(test)]
fn read_file_contents(
    path: &Path,
    request: &ironrdp::cliprdr::pdu::FileContentsRequest,
) -> anyhow::Result<ironrdp::cliprdr::pdu::FileContentsResponse<'static>> {
    LocalClipboardEntry::from_file(path)?.read(request)
}

#[cfg(test)]
#[path = "clipboard_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "clipboard_transfer_tests.rs"]
mod transfer_tests;
