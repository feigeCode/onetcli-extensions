use std::sync::atomic::{AtomicI64, Ordering};

use extension_protocol::{
    conn::SecretRef,
    envelope::{Request, RequestId, RpcMessage},
    error::{ProtocolError, error_codes},
    framing::{recv_msg_async, send_msg_async},
    host::{ResolveSecretParams, ResolveSecretResult},
    method,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::{boxed_error, invalid_params};

pub(crate) struct IpcParts<R, W>
where
    R: AsyncReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    pub(crate) reader: R,
    pub(crate) writer: W,
}

impl<R, W> IpcParts<R, W>
where
    R: AsyncReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    pub(crate) fn new(reader: R, writer: W) -> Self {
        Self { reader, writer }
    }
}

pub(crate) async fn resolve_secret<R, W>(
    ipc: &mut IpcParts<R, W>,
    secret_ref: &SecretRef,
    next_id: &AtomicI64,
) -> Result<Vec<u8>, Box<ProtocolError>>
where
    R: AsyncReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    let id = RequestId::Number(next_id.fetch_add(1, Ordering::SeqCst));
    let request = Request::new(
        id,
        method::HOST_RESOLVE_SECRET,
        serde_json::to_value(ResolveSecretParams {
            secret_ref: secret_ref.clone(),
        })
        .map_err(|error| invalid_params(error.to_string()))?,
    );
    send_msg_async(&mut ipc.writer, &RpcMessage::Request(request))
        .await
        .map_err(|error| {
            boxed_error(
                error_codes::INTERNAL_ERROR,
                format!("failed to request secret resolution: {error}"),
            )
        })?;

    let message = recv_msg_async::<_, RpcMessage>(&mut ipc.reader)
        .await
        .map_err(|error| {
            boxed_error(
                error_codes::INTERNAL_ERROR,
                format!("failed to receive secret resolution response: {error}"),
            )
        })?;
    let RpcMessage::Response(response) = message else {
        return Err(boxed_error(
            error_codes::INTERNAL_ERROR,
            "secret resolution returned an invalid RPC response",
        ));
    };
    if let Some(error) = response.error() {
        return Err(Box::new(error.clone()));
    }
    let result_value = response.result().ok_or_else(|| {
        boxed_error(
            error_codes::INTERNAL_ERROR,
            "secret resolution returned neither a result nor an error",
        )
    })?;
    let result: ResolveSecretResult = serde_json::from_value(result_value.clone())
        .map_err(|error| invalid_params(error.to_string()))?;
    Ok(result.value)
}
