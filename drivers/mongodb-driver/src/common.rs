use base64::Engine as _;
use extension_protocol::blob::{BlobReadParams, BlobReadResult, WireBytes};
use extension_protocol::error::{ErrorData, ProtocolError, error_codes};
use extension_protocol::mongodb::MongoBsonDocument;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

pub type BlobStore = HashMap<String, (Vec<u8>, usize)>;

pub fn decode_document(value: WireBytes) -> Result<bson::Document, ProtocolError> {
    let bytes = match value {
        WireBytes::Utf8(value) => value.into_bytes(),
        WireBytes::Base64(value) => base64::engine::general_purpose::STANDARD
            .decode(value)
            .map_err(invalid_params)?,
    };
    bson::from_slice(&bytes).map_err(invalid_params)
}

pub fn encode_document(document: &bson::Document) -> Result<MongoBsonDocument, ProtocolError> {
    let bytes = bson::to_vec(document).map_err(internal_error)?;
    Ok(MongoBsonDocument {
        bson: WireBytes::Base64(base64::engine::general_purpose::STANDARD.encode(bytes)),
    })
}

pub fn read_blob(store: &mut BlobStore, params: &Value) -> Result<Value, ProtocolError> {
    let params: BlobReadParams = serde_json::from_value(params.clone()).map_err(invalid_params)?;
    let Some((bytes, offset)) = store.get_mut(&params.blob_id) else {
        return Err(ProtocolError::new(
            error_codes::RESOURCE_CLOSED,
            "blob is closed",
        ));
    };
    let end = (*offset + params.effective_max_bytes() as usize).min(bytes.len());
    let chunk = &bytes[*offset..end];
    *offset = end;
    serde_json::to_value(BlobReadResult {
        data: base64::engine::general_purpose::STANDARD.encode(chunk),
        bytes_read: chunk.len() as u32,
        done: end == bytes.len(),
    })
    .map_err(internal_error)
}

pub fn close_blob(store: &mut BlobStore, params: &Value) -> Result<Value, ProtocolError> {
    let blob_id = params
        .get("blob_id")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_params("blob_id is required"))?;
    store.remove(blob_id);
    Ok(Value::Null)
}

pub fn next_blob_id() -> String {
    static BLOB_ID: AtomicU64 = AtomicU64::new(1);
    format!("mongo-docs-{}", BLOB_ID.fetch_add(1, Ordering::Relaxed))
}

pub fn should_retry_with_admin_auth_source(connection_string: &str, error: &str) -> bool {
    connection_string.contains('@')
        && !connection_string
            .to_ascii_lowercase()
            .contains("authsource=")
        && (error.contains("SCRAM failure")
            || error.contains("AuthenticationFailed")
            || error.contains("code 18"))
}

pub fn append_admin_auth_source(connection_string: &str) -> String {
    if connection_string.contains('?') {
        let separator = if connection_string.ends_with(['?', '&']) {
            ""
        } else {
            "&"
        };
        return format!("{connection_string}{separator}authSource=admin");
    }
    let separator = if connection_string
        .split_once("://")
        .is_some_and(|(_, remaining)| remaining.contains('/'))
    {
        "?"
    } else {
        "/?"
    };
    format!("{connection_string}{separator}authSource=admin")
}

pub fn invalid_params(error: impl std::fmt::Display) -> ProtocolError {
    ProtocolError::new(error_codes::INVALID_PARAMS, error.to_string())
}

pub fn mongo_error(
    protocol_code: i32,
    message: String,
    kind: String,
    mut labels: Vec<String>,
    vendor_code: Option<i64>,
    code_name: Option<String>,
    server_message: Option<String>,
) -> ProtocolError {
    labels.sort();
    let retryable = labels.iter().any(|label| {
        matches!(
            label.as_str(),
            "RetryableWriteError" | "TransientTransactionError" | "UnknownTransactionCommitResult"
        )
    });
    let data = ErrorData {
        vendor_code,
        retryable: retryable.then_some(true),
        extra: Some(json!({
            "kind": kind,
            "labels": labels,
            "code_name": code_name,
            "server_message": server_message,
            "source": message,
        })),
        ..ErrorData::default()
    };
    ProtocolError::new(protocol_code, message).with_data(data)
}

pub fn internal_error(error: impl std::fmt::Display) -> ProtocolError {
    ProtocolError::new(error_codes::INTERNAL_ERROR, error.to_string())
}

pub fn unsupported_method(method_name: &str) -> ProtocolError {
    ProtocolError::new(
        error_codes::METHOD_NOT_FOUND,
        format!("unsupported MongoDB method `{method_name}`"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bson_document_round_trip_is_binary_safe() {
        let document = bson::doc! { "n": bson::Bson::Int64(i64::MAX), "ok": true };
        let wire = encode_document(&document).unwrap();
        let decoded = decode_document(wire.bson).unwrap();

        assert_eq!(document, decoded);
    }

    #[test]
    fn admin_auth_source_retry_preserves_existing_uri_parts() {
        let uri = "mongodb://user:p%40ss@mongo.internal:27017/app?replicaSet=rs0";

        assert!(should_retry_with_admin_auth_source(
            uri,
            "AuthenticationFailed code 18"
        ));
        assert_eq!(
            "mongodb://user:p%40ss@mongo.internal:27017/app?replicaSet=rs0&authSource=admin",
            append_admin_auth_source(uri)
        );
    }

    #[test]
    fn admin_auth_source_retry_requires_credentials_and_missing_option() {
        assert!(!should_retry_with_admin_auth_source(
            "mongodb://mongo.internal:27017/app",
            "AuthenticationFailed code 18"
        ));
        assert!(!should_retry_with_admin_auth_source(
            "mongodb://user:pass@mongo.internal/app?authSource=users",
            "AuthenticationFailed code 18"
        ));
    }

    #[test]
    fn mongo_error_preserves_native_details_and_retryable_label() {
        let error = mongo_error(
            error_codes::EXTENSION_CUSTOM_START,
            "command failed: duplicate key".to_string(),
            "Command".to_string(),
            vec!["RetryableWriteError".to_string()],
            Some(11000),
            Some("DuplicateKey".to_string()),
            Some("duplicate key".to_string()),
        );
        let data = error.data.expect("MongoDB error data");
        let extra = data.extra.expect("MongoDB native details");

        assert_eq!(Some(11000), data.vendor_code);
        assert_eq!(Some(true), data.retryable);
        assert_eq!(
            Some("DuplicateKey"),
            extra.get("code_name").and_then(Value::as_str)
        );
        assert_eq!(
            Some("duplicate key"),
            extra.get("server_message").and_then(Value::as_str)
        );
    }
}
