use extension_protocol::{
    blob::{BlobCloseParams, BlobReadParams},
    event_stream::{EventCloseParams, EventOpenParams, EventReadParams},
};
use serde_json::Value;

use crate::{
    error::{ProviderResult, parse_params, serialize},
    state::ProviderState,
};

pub(super) fn read_blob(state: &mut ProviderState, params: Value) -> ProviderResult {
    let params: BlobReadParams = parse_params(params)?;
    serialize(state.read_blob(params)?)
}

pub(super) fn close_blob(state: &mut ProviderState, params: Value) -> ProviderResult {
    let params: BlobCloseParams = parse_params(params)?;
    state.close_blob(params);
    Ok(Value::Null)
}

pub(super) fn open_event(state: &mut ProviderState, params: Value) -> ProviderResult {
    let params: EventOpenParams = parse_params(params)?;
    serialize(state.open_event_stream(params)?)
}

pub(super) fn read_event(state: &mut ProviderState, params: Value) -> ProviderResult {
    let params: EventReadParams = parse_params(params)?;
    serialize(state.read_event_stream(params))
}

pub(super) fn close_event(state: &mut ProviderState, params: Value) -> ProviderResult {
    let params: EventCloseParams = parse_params(params)?;
    state.close_event_stream(&params.stream_id);
    Ok(Value::Null)
}
