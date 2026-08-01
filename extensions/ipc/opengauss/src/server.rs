#![allow(clippy::result_large_err)]

use anyhow::{Context, Result};
use extension_protocol::error::{ErrorCode, ErrorData, ProtocolError, error_codes};
use interprocess::local_socket::{
    GenericNamespaced, ToNsName,
    tokio::{Stream, prelude::*},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::driver::OpenGaussDriver;

pub async fn run(socket_name: &str) -> Result<()> {
    let name = socket_name
        .to_ns_name::<GenericNamespaced>()
        .context("invalid local socket name")?;
    let stream = Stream::connect(name)
        .await
        .context("failed to connect to host listener")?;
    let (reader, writer) = tokio::io::split(stream);
    extension_driver::serve(OpenGaussDriver::new(), reader, writer).await
}

pub async fn handle_stream<R, W>(reader: R, writer: W) -> Result<()>
where
    R: AsyncReadExt + Unpin + Send,
    W: AsyncWriteExt + Unpin + Send + 'static,
{
    extension_driver::serve(OpenGaussDriver::new(), reader, writer).await
}

pub(crate) fn invalid_params(message: impl Into<String>) -> ProtocolError {
    ProtocolError::new(error_codes::INVALID_PARAMS, message)
}

pub(crate) fn params_deserialize_error(error: serde_json::Error) -> ProtocolError {
    ProtocolError::new(
        error_codes::INVALID_PARAMS,
        format!("failed to deserialize params: {error}"),
    )
}

pub(crate) fn protocol_error_from_anyhow(code: ErrorCode, error: anyhow::Error) -> ProtocolError {
    let chain = error
        .chain()
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    let mut data = ErrorData::new();
    let mut extra = serde_json::json!({ "chain": chain });

    if let Some(database_error) = error.chain().find_map(|source| {
        source
            .downcast_ref::<tokio_opengauss::Error>()
            .and_then(tokio_opengauss::Error::as_db_error)
    }) {
        data.sqlstate = Some(database_error.code().code().to_owned());
        data.schema = database_error.schema().map(ToOwned::to_owned);
        data.table = database_error.table().map(ToOwned::to_owned);
        data.column = database_error.column().map(ToOwned::to_owned);
        data.constraint = database_error.constraint().map(ToOwned::to_owned);
        extra["severity"] = serde_json::json!(database_error.severity());
        extra["server_message"] = serde_json::json!(database_error.message());
        extra["detail"] = serde_json::json!(database_error.detail());
        extra["hint"] = serde_json::json!(database_error.hint());
        extra["where"] = serde_json::json!(database_error.where_());
        extra["datatype"] = serde_json::json!(database_error.datatype());
        extra["file"] = serde_json::json!(database_error.file());
        extra["line"] = serde_json::json!(database_error.line());
        extra["routine"] = serde_json::json!(database_error.routine());
        match database_error.position() {
            Some(tokio_opengauss::error::ErrorPosition::Original(position)) => {
                extra["position"] = serde_json::json!(position);
            }
            Some(tokio_opengauss::error::ErrorPosition::Internal { position, query }) => {
                extra["internal_position"] = serde_json::json!(position);
                extra["internal_query"] = serde_json::json!(query);
            }
            None => {}
        }
    }

    ProtocolError::new(code, format!("{error:#}")).with_data(data.with_extra(extra))
}

pub(crate) fn not_supported(message: impl Into<String>) -> ProtocolError {
    ProtocolError::new(error_codes::METHOD_NOT_FOUND, message)
}
