use async_trait::async_trait;
use base64::Engine as _;
use bson_legacy::{Bson, Document, decode_document, encode_document};
use extension_driver::{
    AsyncDriverConnection, AsyncNativeDriver, AsyncOpenedConnection, serve_async_from_env,
};
use extension_protocol::blob::{BlobReadParams, BlobReadResult, WireBytes};
use extension_protocol::conn::{ConnOpenParams, ConnOpenResult};
use extension_protocol::error::{ErrorData, ProtocolError, error_codes};
use extension_protocol::lifecycle::{Capability, InitResult};
use extension_protocol::method;
use extension_protocol::mongodb::{
    MongoBsonDocument, MongoCommandParams, MongoConnectionConfig, MongoFindParams, MongoFindResult,
};
use mongodb_legacy32::coll::options::FindOptions;
use mongodb_legacy32::db::ThreadedDatabase;
use mongodb_legacy32::{Client, CommandType, ThreadedClient};
use percent_encoding::percent_decode_str;
use serde_json::Value;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::atomic::{AtomicU64, Ordering};

const SOCKET_ENV: &str = "ONETCLI_EXT_SOCKET";

type BlobStore = HashMap<String, (Vec<u8>, usize)>;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    serve_async_from_env(
        MongoDriver {
            next_conn_id: AtomicU64::new(1),
        },
        SOCKET_ENV,
    )
    .await
}

struct MongoDriver {
    next_conn_id: AtomicU64,
}

struct MongoConnection {
    client: Client,
    blobs: BlobStore,
}

#[async_trait]
impl AsyncNativeDriver for MongoDriver {
    async fn init(&self, _params: &Value) -> Result<Value, ProtocolError> {
        let result = InitResult::new(env!("CARGO_PKG_VERSION"))
            .with_api("mongodb", extension_protocol::WIRE_PROTOCOL_VERSION)
            .with_feature(Capability::CANCEL_REQUEST)
            .with_method(method::MONGODB_COMMAND)
            .with_method(method::MONGODB_FIND)
            .with_method(method::BLOB_READ)
            .with_method(method::BLOB_CLOSE)
            .with_driver("mongodb-legacy-3-2");
        serde_json::to_value(result).map_err(internal_error)
    }

    async fn open_connection(
        &self,
        params: &Value,
    ) -> Result<AsyncOpenedConnection, ProtocolError> {
        let open: ConnOpenParams =
            serde_json::from_value(params.clone()).map_err(invalid_params)?;
        let config: MongoConnectionConfig =
            serde_json::from_value(open.config).map_err(invalid_params)?;
        let connection_string = config.connection_string;
        let client = tokio::task::spawn_blocking(move || connect(&connection_string))
            .await
            .map_err(internal_error)??;
        let conn_id = self.next_conn_id.fetch_add(1, Ordering::Relaxed);
        Ok(AsyncOpenedConnection {
            conn_id,
            open_result: serde_json::to_value(ConnOpenResult {
                conn_id,
                server_info: None,
            })
            .map_err(internal_error)?,
            connection: Box::new(MongoConnection {
                client,
                blobs: BlobStore::new(),
            }),
        })
    }

    async fn call_connless(
        &self,
        method_name: &str,
        _params: &Value,
    ) -> Result<Value, ProtocolError> {
        Err(unsupported_method(method_name))
    }
}

fn connect(connection_string: &str) -> Result<Client, ProtocolError> {
    if connection_string.starts_with("mongodb+srv://") {
        return Err(connection_error(
            "MongoDB 3.2 driver does not support mongodb+srv connection strings",
        ));
    }
    if uri_option_is_true(connection_string, "tls") || uri_option_is_true(connection_string, "ssl")
    {
        return Err(connection_error(
            "MongoDB 3.2 driver does not support TLS connection options",
        ));
    }

    let parsed =
        mongodb_legacy32::connstring::parse(connection_string).map_err(mongo_connection_error)?;
    let username = parsed.user.as_deref().map(percent_decode);
    let password = parsed.password.as_deref().map(percent_decode);
    let auth_source = parsed
        .options
        .as_ref()
        .and_then(|options| {
            options
                .options
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case("authSource"))
                .map(|(_, value)| percent_decode(value))
        })
        .or_else(|| parsed.database.clone().filter(|value| !value.is_empty()))
        .unwrap_or_else(|| "admin".to_string());

    let client = Client::with_uri(connection_string).map_err(mongo_connection_error)?;
    if let Some(username) = username {
        let password = password.unwrap_or_default();
        client
            .db(&auth_source)
            .auth(&username, &password)
            .map_err(mongo_connection_error)?;
    }
    client
        .db("admin")
        .command(
            legacy_doc(&[("buildInfo", Bson::I32(1))]),
            CommandType::BuildInfo,
            None,
        )
        .map_err(mongo_connection_error)?;
    Ok(client)
}

#[async_trait]
impl AsyncDriverConnection for MongoConnection {
    async fn call(&mut self, method_name: &str, params: &Value) -> Result<Value, ProtocolError> {
        match method_name {
            method::MONGODB_COMMAND => {
                let params: MongoCommandParams =
                    serde_json::from_value(params.clone()).map_err(invalid_params)?;
                let command = decode_legacy_document(params.command.bson)?;
                let client = self.client.clone();
                let database = params.database;
                let result = tokio::task::spawn_blocking(move || {
                    client
                        .db(&database)
                        .command(command, CommandType::Suppressed, None)
                        .map_err(command_error)
                })
                .await
                .map_err(internal_error)??;
                serde_json::to_value(encode_legacy_document(&result)?).map_err(internal_error)
            }
            method::MONGODB_FIND => self.find(params).await,
            method::BLOB_READ => read_blob(&mut self.blobs, params),
            method::BLOB_CLOSE => close_blob(&mut self.blobs, params),
            _ => Err(unsupported_method(method_name)),
        }
    }

    async fn close(&mut self) {
        self.blobs.clear();
    }
}

impl MongoConnection {
    async fn find(&mut self, value: &Value) -> Result<Value, ProtocolError> {
        let params: MongoFindParams =
            serde_json::from_value(value.clone()).map_err(invalid_params)?;
        let filter = params
            .filter
            .map(|document| decode_legacy_document(document.bson))
            .transpose()?;
        let projection = params
            .options
            .projection
            .map(|document| decode_legacy_document(document.bson))
            .transpose()?;
        let sort = params
            .options
            .sort
            .map(|document| decode_legacy_document(document.bson))
            .transpose()?;
        let options = FindOptions {
            limit: params.options.limit,
            skip: params.options.skip,
            projection,
            sort,
            ..FindOptions::new()
        };
        let client = self.client.clone();
        let database = params.database;
        let collection = params.collection;
        let documents = tokio::task::spawn_blocking(move || {
            client
                .db(&database)
                .collection(&collection)
                .find(filter, Some(options))
                .map_err(command_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(command_error)
        })
        .await
        .map_err(internal_error)??;

        let mut encoded_documents = Vec::with_capacity(documents.len());
        let mut packed = Vec::new();
        for document in documents {
            let bytes = legacy_document_bytes(&document)?;
            packed.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            packed.extend_from_slice(&bytes);
            encoded_documents.push(MongoBsonDocument {
                bson: WireBytes::Base64(base64::engine::general_purpose::STANDARD.encode(bytes)),
            });
        }
        let document_count = encoded_documents.len() as u64;
        let documents_blob_id =
            if packed.len() as u64 > extension_protocol::blob::INLINE_BLOB_THRESHOLD_BYTES {
                let blob_id = next_blob_id();
                self.blobs.insert(blob_id.clone(), (packed, 0));
                encoded_documents.clear();
                Some(blob_id)
            } else {
                None
            };
        serde_json::to_value(MongoFindResult {
            documents: encoded_documents,
            documents_blob_id,
            document_count,
            cursor_id: None,
        })
        .map_err(internal_error)
    }
}

fn decode_legacy_document(value: WireBytes) -> Result<Document, ProtocolError> {
    let bytes = match value {
        WireBytes::Utf8(value) => value.into_bytes(),
        WireBytes::Base64(value) => base64::engine::general_purpose::STANDARD
            .decode(value)
            .map_err(invalid_params)?,
    };
    decode_document(&mut Cursor::new(bytes)).map_err(invalid_params)
}

fn encode_legacy_document(document: &Document) -> Result<MongoBsonDocument, ProtocolError> {
    let bytes = legacy_document_bytes(document)?;
    Ok(MongoBsonDocument {
        bson: WireBytes::Base64(base64::engine::general_purpose::STANDARD.encode(bytes)),
    })
}

fn legacy_document_bytes(document: &Document) -> Result<Vec<u8>, ProtocolError> {
    let mut bytes = Vec::new();
    encode_document(&mut bytes, document).map_err(internal_error)?;
    Ok(bytes)
}

fn legacy_doc(entries: &[(&str, Bson)]) -> Document {
    let mut document = Document::new();
    for (key, value) in entries {
        document.insert((*key).to_string(), value.clone());
    }
    document
}

fn percent_decode(value: &str) -> String {
    percent_decode_str(value).decode_utf8_lossy().into_owned()
}

fn uri_option_is_true(connection_string: &str, option: &str) -> bool {
    connection_string
        .split_once('?')
        .map(|(_, query)| query)
        .unwrap_or_default()
        .split(['&', ';'])
        .filter_map(|item| item.split_once('='))
        .any(|(key, value)| {
            key.eq_ignore_ascii_case(option)
                && matches!(value.to_ascii_lowercase().as_str(), "true" | "1")
        })
}

fn connection_error(error: impl std::fmt::Display + std::fmt::Debug) -> ProtocolError {
    legacy_protocol_error(error_codes::IO_CONNECTION_REFUSED, error)
}

fn mongo_connection_error(error: mongodb_legacy32::Error) -> ProtocolError {
    legacy_mongo_protocol_error(error_codes::IO_CONNECTION_REFUSED, error)
}

fn command_error(error: mongodb_legacy32::Error) -> ProtocolError {
    legacy_mongo_protocol_error(error_codes::EXTENSION_CUSTOM_START, error)
}

fn legacy_mongo_protocol_error(code: i32, error: mongodb_legacy32::Error) -> ProtocolError {
    use mongodb_legacy32::Error;

    let message = error.to_string();
    let mut extra = serde_json::Map::from_iter([
        ("kind".to_string(), Value::String(format!("{error:?}"))),
        ("source".to_string(), Value::String(message.clone())),
        (
            "chain".to_string(),
            Value::Array(
                legacy_mongo_error_chain(&error)
                    .into_iter()
                    .map(Value::String)
                    .collect(),
            ),
        ),
    ]);
    let mut vendor_code = None;

    match error {
        Error::CodedError(error_code) => {
            extra.insert(
                "code_name".to_string(),
                Value::String(format!("{error_code:?}")),
            );
            vendor_code = Some(error_code as i64);
        }
        Error::WriteError(exception) => {
            if let Some(write_error) = exception.write_error {
                extra.insert(
                    "server_message".to_string(),
                    Value::String(write_error.message),
                );
                vendor_code = Some(i64::from(write_error.code));
            } else if let Some(write_concern_error) = exception.write_concern_error {
                extra.insert(
                    "server_message".to_string(),
                    Value::String(write_concern_error.message),
                );
                vendor_code = Some(i64::from(write_concern_error.code));
            }
        }
        Error::BulkWriteError(exception) => {
            let write_errors = exception
                .write_errors
                .iter()
                .map(|write_error| {
                    serde_json::json!({
                        "index": write_error.index,
                        "code": write_error.code,
                        "message": write_error.message,
                    })
                })
                .collect::<Vec<_>>();
            if !write_errors.is_empty() {
                extra.insert("write_errors".to_string(), Value::Array(write_errors));
            }
            if let Some(write_error) = exception.write_errors.first() {
                vendor_code = Some(i64::from(write_error.code));
            } else if let Some(write_concern_error) = exception.write_concern_error {
                extra.insert(
                    "server_message".to_string(),
                    Value::String(write_concern_error.message),
                );
                vendor_code = Some(i64::from(write_concern_error.code));
            }
        }
        _ => {}
    }

    let mut data = ErrorData::new().with_extra(Value::Object(extra));
    if let Some(vendor_code) = vendor_code {
        data = data.with_vendor_code(vendor_code);
    }
    ProtocolError::new(code, message).with_data(data)
}

fn legacy_mongo_error_chain(error: &mongodb_legacy32::Error) -> Vec<String> {
    let mut chain = Vec::new();
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(source) = current {
        chain.push(source.to_string());
        if chain.len() >= 64 {
            break;
        }
        current = source.source();
    }
    chain
}

fn legacy_protocol_error(
    code: i32,
    error: impl std::fmt::Display + std::fmt::Debug,
) -> ProtocolError {
    let message = error.to_string();
    ProtocolError::new(code, message.clone()).with_data(ErrorData::new().with_extra(
        serde_json::json!({
            "kind": format!("{error:?}"),
            "source": message,
        }),
    ))
}

fn invalid_params(error: impl std::fmt::Display) -> ProtocolError {
    ProtocolError::new(error_codes::INVALID_PARAMS, error.to_string())
}

fn internal_error(error: impl std::fmt::Display) -> ProtocolError {
    ProtocolError::new(error_codes::INTERNAL_ERROR, error.to_string())
}

fn unsupported_method(method_name: &str) -> ProtocolError {
    ProtocolError::new(
        error_codes::METHOD_NOT_FOUND,
        format!("unsupported MongoDB method `{method_name}`"),
    )
}

fn read_blob(store: &mut BlobStore, params: &Value) -> Result<Value, ProtocolError> {
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

fn close_blob(store: &mut BlobStore, params: &Value) -> Result<Value, ProtocolError> {
    let blob_id = params
        .get("blob_id")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_params("blob_id is required"))?;
    store.remove(blob_id);
    Ok(Value::Null)
}

fn next_blob_id() -> String {
    static BLOB_ID: AtomicU64 = AtomicU64::new(1);
    format!(
        "mongo-legacy-3-2-docs-{}",
        BLOB_ID.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_bson_round_trip_preserves_binary_values() {
        let mut document = Document::new();
        document.insert("n".to_string(), Bson::I64(i64::MAX));
        document.insert("ok".to_string(), Bson::Boolean(true));

        let encoded = encode_legacy_document(&document).unwrap();
        let decoded = decode_legacy_document(encoded.bson).unwrap();

        assert_eq!(document, decoded);
    }

    #[test]
    fn detects_unsupported_tls_options_case_insensitively() {
        assert!(uri_option_is_true("mongodb://localhost/?TLS=true", "tls"));
        assert!(uri_option_is_true("mongodb://localhost/?ssl=1", "ssl"));
        assert!(!uri_option_is_true("mongodb://localhost/?tls=false", "tls"));
    }

    #[test]
    fn legacy_errors_preserve_original_message_and_kind() {
        let error = mongodb_legacy32::Error::OperationError(
            "server rejected the command with code 123".to_string(),
        );

        let protocol_error = command_error(error);
        let data = protocol_error.data.expect("legacy MongoDB error data");
        let extra = data.extra.expect("legacy MongoDB native details");

        assert!(
            protocol_error
                .message
                .contains("server rejected the command with code 123")
        );
        assert!(
            extra
                .get("kind")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.contains("OperationError"))
        );
    }

    #[test]
    fn legacy_coded_errors_preserve_vendor_code() {
        let error = mongodb_legacy32::Error::CodedError(mongodb_legacy32::ErrorCode::Unauthorized);

        let protocol_error = command_error(error);
        let data = protocol_error.data.expect("legacy MongoDB error data");
        let extra = data.extra.expect("legacy MongoDB native details");

        assert_eq!(data.vendor_code, Some(13));
        assert_eq!(
            extra.get("code_name").and_then(Value::as_str),
            Some("Unauthorized")
        );
        assert!(
            extra
                .get("chain")
                .and_then(Value::as_array)
                .is_some_and(|chain| !chain.is_empty())
        );
    }
}
