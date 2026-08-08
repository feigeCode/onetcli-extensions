use anyhow::Context as _;
use ironrdp_client::rdp::{RdpClient, RdpOutputEvent};
use tokio::sync::mpsc;

use crate::clipboard::{TextClipboardController, text_clipboard};
use crate::output_mailbox::{OutputReceiver, OutputSender, output_mailbox};
use crate::protocol::{ConnectRequest, HelperEvent};
use crate::threading::join_worker;

mod config;
mod input;
mod input_sender;
mod output;

pub(crate) use input::{RdpInputAction, RdpInputContext, apply_input_request, shutdown_inputs};
pub(crate) use input_sender::{HelperInputSender, InputQueueStatus};
use output::RdpOutputMapper;

pub struct RdpRuntime {
    pub input_tx: HelperInputSender,
    output_rx: Option<OutputReceiver>,
    pub clipboard: TextClipboardController,
    client_thread: std::thread::JoinHandle<anyhow::Result<()>>,
}

pub fn start(connect: ConnectRequest) -> anyhow::Result<RdpRuntime> {
    let config = config::build_config(connect)?;
    let (output_tx, output_rx) = mpsc::channel::<RdpOutputEvent>(64);
    let (helper_output_tx, helper_output_rx) = output_mailbox();
    let client = RdpClient::new(config, output_tx);
    let input_tx = HelperInputSender::production(client.input_sender());
    let (clipboard, cliprdr_factory) = text_clipboard(input_tx.clone(), helper_output_tx.clone());
    let client = client.with_cliprdr_backend_factory(cliprdr_factory);

    let client_thread = spawn_client_thread(client, output_rx, helper_output_tx)?;
    Ok(RdpRuntime {
        input_tx,
        output_rx: Some(helper_output_rx),
        clipboard,
        client_thread,
    })
}

impl RdpRuntime {
    pub fn take_output_receiver(&mut self) -> anyhow::Result<OutputReceiver> {
        self.output_rx
            .take()
            .context("RDP output receiver was already taken")
    }

    pub fn shutdown(self, database: &mut ironrdp::input::Database) -> anyhow::Result<()> {
        let Self {
            input_tx,
            output_rx: _output_rx,
            clipboard,
            client_thread,
        } = self;
        if let Err(error) = shutdown_inputs(database, &input_tx) {
            tracing::debug!(?error, "RDP input channel was already closed");
        }
        clipboard.shutdown();
        drop(clipboard);
        drop(input_tx);
        join_worker(client_thread, "RDP client")
    }
}

fn spawn_client_thread(
    client: RdpClient,
    output_rx: mpsc::Receiver<RdpOutputEvent>,
    helper_output_tx: OutputSender,
) -> anyhow::Result<std::thread::JoinHandle<anyhow::Result<()>>> {
    let thread = std::thread::Builder::new()
        .name("navop-rdp-helper-runtime".to_string())
        .spawn(move || run_client_thread(client, output_rx, helper_output_tx))
        .context("spawn RDP client thread")?;
    Ok(thread)
}

fn run_client_thread(
    client: RdpClient,
    output_rx: mpsc::Receiver<RdpOutputEvent>,
    helper_output_tx: OutputSender,
) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build();
    let runtime = match runtime {
        Ok(runtime) => runtime,
        Err(error) => {
            report_runtime_failure(&helper_output_tx);
            return Err(error).context("create RDP tokio runtime");
        }
    };
    runtime.block_on(run_client(client, output_rx, helper_output_tx))
}

async fn run_client(
    client: RdpClient,
    output_rx: mpsc::Receiver<RdpOutputEvent>,
    helper_output_tx: OutputSender,
) -> anyhow::Result<()> {
    let output_task = tokio::spawn(map_output_events(output_rx, helper_output_tx));
    client.run().await;
    output_task
        .await
        .context("RDP output mapper task panicked")?
}

async fn map_output_events(
    mut output_rx: mpsc::Receiver<RdpOutputEvent>,
    helper_output_tx: OutputSender,
) -> anyhow::Result<()> {
    let mut output_mapper = RdpOutputMapper::default();
    while let Some(event) = output_rx.recv().await {
        for helper_event in output_mapper.map(event) {
            helper_output_tx
                .send(helper_event)
                .context("RDP helper output receiver closed")?;
        }
    }
    Ok(())
}

fn report_runtime_failure(helper_output_tx: &OutputSender) {
    let event = HelperEvent::ConnectionFailure {
        message: "failed to create RDP tokio runtime".to_string(),
    };
    if helper_output_tx.send(event).is_err() {
        tracing::debug!("RDP helper output receiver was already closed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_uses_coalescing_output_receiver() {
        fn assert_receiver(runtime: &RdpRuntime) {
            let _: &Option<crate::output_mailbox::OutputReceiver> = &runtime.output_rx;
        }
        let _contract: fn(&RdpRuntime) = assert_receiver;
    }
}
