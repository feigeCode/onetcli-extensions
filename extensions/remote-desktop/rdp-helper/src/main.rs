use std::io::{self, BufRead, Write};
use std::thread::JoinHandle;

use anyhow::Context as _;
use ironrdp::input::Database;
use tracing::error;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

use crate::output_mailbox::OutputReceiver;
use crate::protocol::HelperEvent;
use crate::threading::join_worker;

mod clipboard;
mod output_mailbox;
mod pixels;
mod protocol;
mod rdp;
mod threading;

fn main() {
    if let Err(error) = run() {
        error!(?error, "RDP helper failed");
        let _ = write_event(&HelperEvent::ConnectionFailure {
            message: format!("{error:#}"),
        });
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    setup_logging()?;
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let connect = read_connect_request(&mut lines)?;
    write_event(&HelperEvent::Status {
        message: format!("connecting to RDP {}", connect.destination),
    })?;

    run_session(rdp::start(connect)?, lines)
}

fn run_session(
    mut runtime: rdp::RdpRuntime,
    lines: impl Iterator<Item = io::Result<String>>,
) -> anyhow::Result<()> {
    let mut database = Database::new();
    let output_rx = match runtime.take_output_receiver() {
        Ok(receiver) => receiver,
        Err(error) => return shutdown_without_output(runtime, &mut database, error),
    };
    let output_thread = match spawn_output_writer(output_rx) {
        Ok(thread) => thread,
        Err(error) => return shutdown_without_output(runtime, &mut database, error),
    };

    let input_result = process_input_requests(lines, &runtime, &mut database);
    let shutdown_result = runtime.shutdown(&mut database);
    let output_result = join_worker(output_thread, "RDP output writer");
    combine_results([input_result, shutdown_result, output_result])
}

fn process_input_requests(
    lines: impl Iterator<Item = io::Result<String>>,
    runtime: &rdp::RdpRuntime,
    database: &mut Database,
) -> anyhow::Result<()> {
    for line in lines {
        let request = protocol::decode_request_line(&line?)?;
        let mut context =
            rdp::RdpInputContext::new(&runtime.input_tx, database, &runtime.clipboard);
        if rdp::apply_input_request(request, &mut context)? == rdp::RdpInputAction::Close {
            break;
        }
    }
    Ok(())
}

fn shutdown_without_output(
    runtime: rdp::RdpRuntime,
    database: &mut Database,
    primary_error: anyhow::Error,
) -> anyhow::Result<()> {
    combine_results([Err(primary_error), runtime.shutdown(database), Ok(())])
}

fn read_connect_request(
    lines: &mut impl Iterator<Item = io::Result<String>>,
) -> anyhow::Result<protocol::ConnectRequest> {
    let line = lines
        .next()
        .transpose()?
        .ok_or_else(|| anyhow::anyhow!("missing Connect request"))?;
    protocol::connect_request(protocol::decode_request_line(&line)?)
}

fn spawn_output_writer(
    output_rx: OutputReceiver,
) -> anyhow::Result<JoinHandle<anyhow::Result<()>>> {
    std::thread::Builder::new()
        .name("navop-rdp-helper-output".to_string())
        .spawn(move || {
            while let Some(event) = output_rx.recv() {
                write_event(&event)?;
            }
            Ok(())
        })
        .map_err(anyhow::Error::from)
        .context("spawn RDP output writer")
}

fn write_event(event: &HelperEvent) -> anyhow::Result<()> {
    let mut stdout = io::stdout().lock();
    protocol::write_event(&mut stdout, event)?;
    stdout.flush()?;
    Ok(())
}

fn setup_logging() -> anyhow::Result<()> {
    let env_filter = EnvFilter::builder()
        .with_env_var("ONETCLI_RDP_HELPER_LOG")
        .from_env_lossy();
    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer().with_writer(io::stderr))
        .try_init()?;
    Ok(())
}

fn combine_results<const N: usize>(results: [anyhow::Result<()>; N]) -> anyhow::Result<()> {
    let mut first_error = None;
    for result in results {
        if let Err(error) = result {
            if first_error.is_none() {
                first_error = Some(error);
            } else {
                tracing::error!(?error, "additional RDP session cleanup failure");
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_first_stdin_line_as_connect_request() {
        let input = vec![Ok(
            r#"{"type":"Connect","destination":"host:3389","username":null,"password":null,"domain":null,"width":800,"height":600}"#
                .to_string(),
        )];
        let mut lines = input.into_iter();

        let request = read_connect_request(&mut lines).expect("connect request");

        assert_eq!(request.destination, "host:3389");
        assert_eq!(request.width, 800);
        assert_eq!(request.height, 600);
    }

    #[test]
    fn rejects_non_connect_first_line() {
        let input = vec![Ok(r#"{"type":"Close"}"#.to_string())];
        let mut lines = input.into_iter();

        let error = read_connect_request(&mut lines).expect_err("not a connect request");

        assert!(error.to_string().contains("first helper request"));
    }

    #[test]
    fn join_worker_reports_thread_panics() {
        let handle = std::thread::spawn(|| -> anyhow::Result<()> {
            panic!("worker exploded");
        });

        let error = join_worker(handle, "test").expect_err("panic is reported");

        assert!(error.to_string().contains("test thread panicked"));
        assert!(error.to_string().contains("worker exploded"));
    }

    #[test]
    fn join_worker_reports_inner_errors() {
        let handle = std::thread::spawn(|| anyhow::bail!("worker returned an error"));

        let error = join_worker(handle, "test").expect_err("error is reported");
        let report = format!("{error:#}");

        assert!(report.contains("test thread failed"));
        assert!(report.contains("worker returned an error"));
    }
}
