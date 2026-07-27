use std::any::Any;
use std::thread::JoinHandle;

use anyhow::Context as _;

pub(crate) fn join_worker(
    handle: JoinHandle<anyhow::Result<()>>,
    worker: &str,
) -> anyhow::Result<()> {
    match handle.join() {
        Ok(result) => result.with_context(|| format!("{worker} thread failed")),
        Err(payload) => {
            let message = panic_message(payload);
            anyhow::bail!("{worker} thread panicked: {message}")
        }
    }
}

fn panic_message(payload: Box<dyn Any + Send + 'static>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    "non-string panic payload".to_string()
}
