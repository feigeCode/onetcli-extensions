use extension_protocol::{lifecycle::InitResult, method};
use serde_json::Value;

use crate::{
    error::{ProviderResult, serialize},
    state::ProviderState,
};

const PROVIDER_VERSION: &str = env!("CARGO_PKG_VERSION");

pub(super) fn init() -> ProviderResult {
    serialize(
        InitResult::new(PROVIDER_VERSION)
            .with_api("extension", "1.0")
            .with_method(method::RESOURCE_OPEN)
            .with_method(method::RESOURCE_PING)
            .with_method(method::RESOURCE_INVOKE)
            .with_method(method::RESOURCE_CLOSE)
            .with_method(method::BLOB_READ)
            .with_method(method::BLOB_CLOSE)
            .with_method(method::JOB_START)
            .with_method(method::JOB_STATUS)
            .with_method(method::JOB_CANCEL)
            .with_method(method::JOB_RESULT)
            .with_method(method::JOB_CLOSE)
            .with_method(method::EVENT_OPEN)
            .with_method(method::EVENT_READ)
            .with_method(method::EVENT_CLOSE),
    )
}

pub(super) fn shutdown(state: &mut ProviderState) -> ProviderResult {
    state.clear();
    Ok(Value::Null)
}
