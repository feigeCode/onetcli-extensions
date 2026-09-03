use extension_protocol::error::{ErrorCode, ProtocolError, error_codes};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

pub(crate) type ProviderResult = Result<Value, Box<ProtocolError>>;

pub(crate) fn boxed_error(code: ErrorCode, message: impl Into<String>) -> Box<ProtocolError> {
    Box::new(ProtocolError::new(code, message))
}

pub(crate) fn invalid_params(message: impl Into<String>) -> Box<ProtocolError> {
    boxed_error(error_codes::INVALID_PARAMS, message)
}

pub(crate) fn resource_error() -> Box<ProtocolError> {
    boxed_error(
        error_codes::RESOURCE_CLOSED,
        "Elasticsearch resource is not open",
    )
}

pub(crate) fn parse_params<T: DeserializeOwned>(value: Value) -> Result<T, Box<ProtocolError>> {
    serde_json::from_value(value).map_err(|error| invalid_params(error.to_string()))
}

pub(crate) fn serialize<T: Serialize>(value: T) -> ProviderResult {
    serde_json::to_value(value).map_err(|error| invalid_params(error.to_string()))
}
