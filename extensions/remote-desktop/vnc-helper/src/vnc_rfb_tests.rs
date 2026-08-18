use std::io;

use super::*;
use crate::output_mailbox::output_mailbox;

#[test]
fn missing_password_emits_terminal_authentication_failure() {
    assert_terminal_authentication_failure(VncError::NoPassword, "VNC authentication failed");
}

#[test]
fn wrong_password_emits_terminal_authentication_failure() {
    assert_terminal_authentication_failure(VncError::WrongPassword, "wrong password");
}

#[test]
fn rfb_38_authentication_failure_reason_is_terminal() {
    assert_terminal_authentication_failure(
        VncError::General("password check failed!".to_string()),
        "password check failed!",
    );
}

#[test]
fn transient_io_error_still_requests_reconnect() {
    let error = anyhow::Error::new(VncError::IoError(io::Error::new(
        io::ErrorKind::ConnectionReset,
        "temporary reset",
    )));
    let (output_tx, output_rx) = output_mailbox();

    let result = connect_error_result(error, &output_tx);
    drop(output_tx);

    assert!(matches!(
        result,
        VncSessionResult::Reconnect {
            reason: RemoteDesktopReconnectReason::SessionError,
            manual: false,
            was_connected: false,
            ..
        }
    ));
    assert!(
        output_rx.recv().is_none(),
        "transient errors must not emit a terminal failure"
    );
}

fn assert_terminal_authentication_failure(error: VncError, expected_message: &str) {
    let (output_tx, output_rx) = output_mailbox();

    let result = connect_error_result(anyhow::Error::new(error), &output_tx);
    drop(output_tx);

    assert!(matches!(result, VncSessionResult::Closed));
    let Some(RemoteDesktopOutput::ConnectionFailure(message)) = output_rx.recv() else {
        panic!("authentication errors must emit ConnectionFailure");
    };
    assert!(
        message
            .to_ascii_lowercase()
            .contains(&expected_message.to_ascii_lowercase())
    );
    assert!(
        output_rx.recv().is_none(),
        "authentication errors must not emit Reconnecting"
    );
}
