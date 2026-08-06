use anyhow::Context as _;
use tokio::net::TcpStream;
use vnc_client::{
    PixelFormat, SecurityPolicy, VncClient, VncConnector, VncCredentials, VncEncoding, X11Event,
};

use crate::output_mailbox::OutputSender;
use crate::runtime::{
    RemoteDesktopConnectionOptions, RemoteDesktopInput, RemoteDesktopOutput,
    RemoteDesktopReconnectReason,
};
use crate::vnc_clipboard::VncClipboardSnapshot;
use crate::vnc_encoding::ConnectedVncSession;
use crate::vnc_input::{handle_pending_inputs, shutdown_inputs};
use crate::vnc_reconnect::{
    VNC_POLL_INTERVAL, VncSessionResult, merge_cleanup_error, reconnect_delay, reconnect_reason,
    reconnect_result, send_reconnecting, session_result_from_action, wait_before_reconnect,
};

pub fn run_vnc_thread(
    options: RemoteDesktopConnectionOptions,
    input_rx: tokio::sync::mpsc::UnboundedReceiver<RemoteDesktopInput>,
    output_tx: OutputSender,
) -> anyhow::Result<()> {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        send_failure(&output_tx, "failed to start VNC runtime");
        return Err(anyhow::anyhow!("failed to start VNC runtime"));
    };
    runtime.block_on(run_vnc_backend(options, input_rx, output_tx))
}

async fn run_vnc_backend(
    options: RemoteDesktopConnectionOptions,
    mut input_rx: tokio::sync::mpsc::UnboundedReceiver<RemoteDesktopInput>,
    output_tx: OutputSender,
) -> anyhow::Result<()> {
    let mut latest_clipboard = None;
    let mut reconnect_attempt = 0usize;
    loop {
        match run_vnc_session(&options, &mut latest_clipboard, &mut input_rx, &output_tx).await? {
            VncSessionResult::Closed | VncSessionResult::InputClosed => break,
            VncSessionResult::Reconnect {
                reason,
                diagnostic,
                manual,
                was_connected,
            } => {
                tracing::warn!(
                    ?reason,
                    manual,
                    was_connected,
                    %diagnostic,
                    "VNC session reconnecting"
                );
                if was_connected || manual {
                    reconnect_attempt = 0;
                }
                if manual {
                    send_reconnecting(&output_tx, reason, None);
                    continue;
                }
                let delay = reconnect_delay(reconnect_attempt);
                reconnect_attempt = reconnect_attempt.saturating_add(1);
                send_reconnecting(&output_tx, reason, Some(delay.as_secs()));
                if !wait_before_reconnect(&mut input_rx, &mut latest_clipboard, delay).await {
                    break;
                }
            }
        }
    }
    Ok(())
}

async fn run_vnc_session(
    options: &RemoteDesktopConnectionOptions,
    latest_clipboard: &mut Option<VncClipboardSnapshot>,
    input_rx: &mut tokio::sync::mpsc::UnboundedReceiver<RemoteDesktopInput>,
    output_tx: &OutputSender,
) -> anyhow::Result<VncSessionResult> {
    send_status(
        output_tx,
        &format!("connecting to VNC {}", options.destination),
    );
    let client = match connect_vnc(options).await {
        Ok(client) => client,
        Err(error) => {
            return Ok(reconnect_result(
                RemoteDesktopReconnectReason::SessionError,
                error.to_string(),
                false,
                false,
            ));
        }
    };
    output_tx.begin_generation();
    let mut session = ConnectedVncSession::new(client);
    if let Err(error) = session.request_initial_refresh().await {
        let reason = shutdown_with_reason(&mut session, error.to_string()).await;
        return Ok(reconnect_result(
            RemoteDesktopReconnectReason::DisplayUpdate,
            reason,
            false,
            session.was_connected,
        ));
    }
    if let Some(snapshot) = latest_clipboard.as_ref() {
        let event = X11Event::CopyTextBytes(snapshot.wire_bytes().to_vec());
        if let Err(error) = session.client.input(event).await {
            let reason = shutdown_with_reason(&mut session, error.to_string()).await;
            return Ok(reconnect_result(
                reconnect_reason(session.was_connected),
                reason,
                false,
                session.was_connected,
            ));
        }
    }
    Ok(run_connected_vnc_session(session, latest_clipboard, input_rx, output_tx).await)
}

async fn connect_vnc(options: &RemoteDesktopConnectionOptions) -> anyhow::Result<VncClient> {
    let tcp = TcpStream::connect(&options.destination)
        .await
        .with_context(|| format!("failed to connect VNC {}", options.destination))?;
    let credentials = VncCredentials {
        username: options.username.clone(),
        password: options.password.clone(),
        domain: options.domain.clone(),
    };
    let state = VncConnector::new(tcp)
        .set_credentials(credentials)
        .set_security_policy(SecurityPolicy::Auto)
        .add_encoding(VncEncoding::Zrle)
        .add_encoding(VncEncoding::CopyRect)
        .add_encoding(VncEncoding::CursorPseudo)
        .add_encoding(VncEncoding::Raw)
        .allow_shared(true)
        .set_pixel_format(PixelFormat::rgba())
        .build()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let client = state
        .try_start()
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?
        .finish()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(client)
}

async fn run_connected_vnc_session(
    mut session: ConnectedVncSession,
    latest_clipboard: &mut Option<VncClipboardSnapshot>,
    input_rx: &mut tokio::sync::mpsc::UnboundedReceiver<RemoteDesktopInput>,
    output_tx: &OutputSender,
) -> VncSessionResult {
    let result = loop {
        if let Err(reason) = session.poll_events(output_tx).await {
            break reconnect_result(
                reconnect_reason(session.was_connected),
                reason,
                false,
                session.was_connected,
            );
        }
        let action = handle_pending_inputs(
            &session.client,
            latest_clipboard,
            input_rx,
            &mut session.keyboard,
            &mut session.pointer,
        )
        .await;
        if let Some(result) = session_result_from_action(action, session.was_connected) {
            break result;
        }
        if let Err(reason) = session.refresh_if_needed().await {
            break reconnect_result(
                RemoteDesktopReconnectReason::DisplayUpdate,
                reason,
                false,
                session.was_connected,
            );
        }
        tokio::time::sleep(VNC_POLL_INTERVAL).await;
    };
    if let Err(error) = shutdown_session(&mut session).await {
        return merge_cleanup_error(result, error);
    }
    result
}

async fn shutdown_with_reason(session: &mut ConnectedVncSession, reason: String) -> String {
    match shutdown_session(session).await {
        Ok(()) => reason,
        Err(error) => format!("{reason}; cleanup failed: {error:#}"),
    }
}

async fn shutdown_session(session: &mut ConnectedVncSession) -> anyhow::Result<()> {
    let mut first_error =
        shutdown_inputs(&session.client, &mut session.keyboard, &mut session.pointer)
            .await
            .err();
    if let Err(error) = session.client.close().await
        && first_error.is_none()
    {
        first_error = Some(anyhow::anyhow!(error.to_string()));
    }
    first_error.map_or(Ok(()), Err)
}

fn send_status(output_tx: &OutputSender, message: &str) {
    let _ = output_tx.send(RemoteDesktopOutput::Status(message.to_string()));
}

fn send_failure(output_tx: &OutputSender, message: &str) {
    let _ = output_tx.send(RemoteDesktopOutput::ConnectionFailure(message.to_string()));
}
