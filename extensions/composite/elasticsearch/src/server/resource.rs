use elasticsearch::auth::Credentials;
use extension_protocol::{
    error::ProtocolError,
    resource::{ResourceCloseParams, ResourceInvokeParams, ResourceOpenResult, ResourcePingParams},
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{
    client::{build_client, execute, normalize_result, validate_connection},
    config::{PendingCredentials, parse_open_params, stored_credentials},
    error::{ProviderResult, invalid_params, parse_params, resource_error, serialize},
    ipc::{IpcParts, resolve_secret},
    state::ProviderState,
};

pub(super) async fn open<R, W>(
    ipc: &mut IpcParts<R, W>,
    state: &mut ProviderState,
    params: Value,
) -> ProviderResult
where
    R: AsyncReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    let (url, pending_credentials) = parse_open_params(params)?;
    let credentials = resolve_credentials(ipc, state, pending_credentials).await?;
    let resource = build_client(&url, credentials)?;
    validate_connection(&resource).await?;
    let resource_id = state.insert_resource(resource);
    serialize(ResourceOpenResult {
        resource_id,
        capabilities: vec![
            "elasticsearch/cluster/info".to_owned(),
            "elasticsearch/cluster/health".to_owned(),
            "elasticsearch/index/list".to_owned(),
            "elasticsearch/index/get".to_owned(),
            "elasticsearch/index/mapping".to_owned(),
            "elasticsearch/search".to_owned(),
            "elasticsearch/search/async".to_owned(),
            "elasticsearch/search/events".to_owned(),
        ],
        metadata: Some(json!({
            "client": "elasticsearch-rs",
            "client_version": "9.1.0-alpha.1",
            "server_major": 9,
            "network": true,
            "operations": "read-only"
        })),
    })
}

async fn resolve_credentials<R, W>(
    ipc: &mut IpcParts<R, W>,
    state: &ProviderState,
    credentials: PendingCredentials,
) -> Result<Option<Credentials>, Box<ProtocolError>>
where
    R: AsyncReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    let PendingCredentials::Reference {
        kind,
        reference,
        username,
    } = credentials
    else {
        return Ok(match credentials {
            PendingCredentials::None => None,
            PendingCredentials::Direct(credentials) => Some(credentials),
            PendingCredentials::Reference { .. } => unreachable!(),
        });
    };
    let secret = resolve_secret(ipc, &reference, state.reverse_request_id()).await?;
    let secret = String::from_utf8(secret)
        .map_err(|_| invalid_params("Elasticsearch credential is not UTF-8"))?;
    Ok(Some(
        stored_credentials(&kind, username.as_deref(), &secret)
            .unwrap_or_else(|| Credentials::EncodedApiKey(secret)),
    ))
}

pub(super) fn ping(state: &ProviderState, params: Value) -> ProviderResult {
    let params: ResourcePingParams = parse_params(params)?;
    if state.resource(&params.resource_id).is_none() {
        return Err(resource_error());
    }
    Ok(Value::Null)
}

pub(super) async fn invoke(state: &mut ProviderState, params: Value) -> ProviderResult {
    let params: ResourceInvokeParams = parse_params(params)?;
    let resource = state
        .cloned_resource(&params.resource_id)
        .ok_or_else(resource_error)?;
    let value = execute(&resource, &params.method, &params.params).await?;
    let value = normalize_result(&params.method, value);
    serialize(state.blob_result(&params.resource_id, value)?)
}

pub(super) fn close(state: &mut ProviderState, params: Value) -> ProviderResult {
    let params: ResourceCloseParams = parse_params(params)?;
    if !state.close_resource(&params.resource_id) {
        return Err(resource_error());
    }
    Ok(Value::Null)
}
