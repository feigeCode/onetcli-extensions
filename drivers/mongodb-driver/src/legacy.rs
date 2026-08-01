use crate::common::{
    BlobStore, append_admin_auth_source, close_blob, decode_document, encode_document,
    internal_error, invalid_params, mongo_error, next_blob_id, read_blob,
    should_retry_with_admin_auth_source, unsupported_method,
};
use async_trait::async_trait;
use extension_driver::{
    AsyncDriverConnection, AsyncNativeDriver, AsyncOpenedConnection, serve_async_from_env,
};
use extension_protocol::conn::{ConnOpenParams, ConnOpenResult};
use extension_protocol::error::{ProtocolError, error_codes};
use extension_protocol::lifecycle::{Capability, InitResult};
use extension_protocol::method;
use extension_protocol::mongodb::MongoFindResult;
use extension_protocol::mongodb::{MongoCommandParams, MongoConnectionConfig, MongoFindParams};
use futures_util::TryStreamExt;
use mongodb_legacy::{Client, Database};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};

const SOCKET_ENV: &str = "ONETCLI_EXT_SOCKET";

pub async fn run() -> anyhow::Result<()> {
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
            .with_driver("mongodb-legacy");
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
        let client = match connect_and_read_build_info(&config.connection_string).await {
            Ok(client) => client,
            Err(error)
                if should_retry_with_admin_auth_source(
                    &config.connection_string,
                    &error.to_string(),
                ) =>
            {
                connect_and_read_build_info(&append_admin_auth_source(&config.connection_string))
                    .await
                    .map_err(legacy_connection_error)?
            }
            Err(error) => return Err(legacy_connection_error(error)),
        };
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

async fn connect_and_read_build_info(
    connection_string: &str,
) -> Result<Client, mongodb_legacy::error::Error> {
    let client = Client::with_uri_str(connection_string).await?;
    client
        .database("admin")
        .run_command(mongodb_legacy::bson::doc! { "buildInfo": 1 }, None)
        .await?;
    Ok(client)
}

#[async_trait]
impl AsyncDriverConnection for MongoConnection {
    async fn call(&mut self, method_name: &str, params: &Value) -> Result<Value, ProtocolError> {
        match method_name {
            method::MONGODB_COMMAND => {
                let params: MongoCommandParams =
                    serde_json::from_value(params.clone()).map_err(invalid_params)?;
                let result = self
                    .database(&params.database)
                    .run_command(decode_document(params.command.bson)?, None)
                    .await
                    .map_err(legacy_command_error)?;
                serde_json::to_value(encode_document(&result)?).map_err(internal_error)
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
    fn database(&self, name: &str) -> Database {
        self.client.database(name)
    }

    async fn find(&mut self, value: &Value) -> Result<Value, ProtocolError> {
        let params: MongoFindParams =
            serde_json::from_value(value.clone()).map_err(invalid_params)?;
        let filter = params
            .filter
            .map(|document| decode_document(document.bson))
            .transpose()?
            .unwrap_or_default();
        let options = mongodb_legacy::options::FindOptions::builder()
            .limit(params.options.limit)
            .skip(params.options.skip.map(|skip| skip.max(0) as u64))
            .sort(
                params
                    .options
                    .sort
                    .map(|document| decode_document(document.bson))
                    .transpose()?,
            )
            .projection(
                params
                    .options
                    .projection
                    .map(|document| decode_document(document.bson))
                    .transpose()?,
            )
            .build();
        let collection = self
            .client
            .database(&params.database)
            .collection::<mongodb_legacy::bson::Document>(&params.collection);
        let mut cursor = collection
            .find(filter, options)
            .await
            .map_err(legacy_command_error)?;
        let mut documents = Vec::new();
        let mut packed = Vec::new();
        while let Some(document) = cursor.try_next().await.map_err(legacy_command_error)? {
            let bytes = mongodb_legacy::bson::to_vec(&document).map_err(internal_error)?;
            packed.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            packed.extend_from_slice(&bytes);
            documents.push(encode_document(&document)?);
        }
        let document_count = documents.len() as u64;
        if packed.len() as u64 > extension_protocol::blob::INLINE_BLOB_THRESHOLD_BYTES {
            let blob_id = next_blob_id();
            self.blobs.insert(blob_id.clone(), (packed, 0));
            documents.clear();
            return serde_json::to_value(MongoFindResult {
                documents,
                documents_blob_id: Some(blob_id),
                document_count,
                cursor_id: None,
            })
            .map_err(internal_error);
        }
        serde_json::to_value(MongoFindResult {
            documents,
            documents_blob_id: None,
            document_count,
            cursor_id: None,
        })
        .map_err(internal_error)
    }
}

fn legacy_connection_error(error: mongodb_legacy::error::Error) -> ProtocolError {
    legacy_protocol_error(error_codes::IO_CONNECTION_REFUSED, error)
}

fn legacy_command_error(error: mongodb_legacy::error::Error) -> ProtocolError {
    legacy_protocol_error(error_codes::EXTENSION_CUSTOM_START, error)
}

fn legacy_protocol_error(protocol_code: i32, error: mongodb_legacy::error::Error) -> ProtocolError {
    let (vendor_code, code_name, server_message) = match error.kind.as_ref() {
        mongodb_legacy::error::ErrorKind::Command(command) => (
            Some(i64::from(command.code)),
            (!command.code_name.is_empty()).then(|| command.code_name.clone()),
            (!command.message.is_empty()).then(|| command.message.clone()),
        ),
        _ => (None, None, None),
    };
    let labels = error.labels().iter().cloned().collect();
    mongo_error(
        protocol_code,
        error.to_string(),
        format!("{:?}", error.kind),
        labels,
        vendor_code,
        code_name,
        server_message,
    )
}
