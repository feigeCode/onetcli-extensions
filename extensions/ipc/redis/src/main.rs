#![allow(clippy::result_large_err)]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use async_trait::async_trait;
use base64::Engine as _;
use extension_driver::{
    AsyncDriverConnection, AsyncNativeDriver, AsyncOpenedConnection, serve_async_from_env,
};
use extension_protocol::blob::WireBytes;
use extension_protocol::conn::{ConnOpenParams, ConnOpenResult};
use extension_protocol::error::{ErrorData, ProtocolError, error_codes};
use extension_protocol::event_stream::{
    EventCloseParams, EventOpenParams, EventOpenResult, EventReadParams, EventReadResult,
};
use extension_protocol::lifecycle::{Capability, InitResult};
use extension_protocol::method;
use extension_protocol::redis::{
    RedisCommandParams, RedisCommandResult, RedisConnectionConfig, RedisPipelineParams,
    RedisPipelineResult, RedisRespValue,
};
use futures::StreamExt;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use redis_client::aio::{ConnectionManager, ConnectionManagerConfig};
use redis_client::{Client, Cmd, Pipeline, RedisError, Value};
use serde_json::{Value as JsonValue, json};
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;
use tokio::time::{Duration, timeout};

const SOCKET_ENV: &str = "ONETCLI_EXT_SOCKET";
const REDIS_USERINFO_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

struct RedisDriver {
    next_conn_id: AtomicU64,
}

struct RedisConnection {
    config: RedisConnectionConfig,
    connections: HashMap<u8, ConnectionManager>,
    events: HashMap<String, RedisEvent>,
}

struct RedisEvent {
    commands: mpsc::Sender<PubSubCommand>,
    events: Arc<Mutex<mpsc::Receiver<JsonValue>>>,
    dropped: Arc<AtomicU64>,
    task: JoinHandle<()>,
}

enum PubSubCommand {
    Subscribe(String),
    PSubscribe(String),
    Unsubscribe(String),
    PUnsubscribe(String),
}

#[async_trait]
impl AsyncNativeDriver for RedisDriver {
    async fn init(&self, _params: &JsonValue) -> Result<JsonValue, ProtocolError> {
        let result = InitResult::new(env!("CARGO_PKG_VERSION"))
            .with_api("redis", extension_protocol::WIRE_PROTOCOL_VERSION)
            .with_feature(Capability::CANCEL_REQUEST)
            .with_method(method::REDIS_COMMAND)
            .with_method(method::REDIS_PIPELINE)
            .with_method(method::EVENT_OPEN)
            .with_method(method::EVENT_READ)
            .with_method(method::EVENT_CLOSE)
            .with_method(method::REDIS_PUBSUB_CONTROL)
            .with_driver("redis");
        serde_json::to_value(result).map_err(internal_error)
    }

    async fn open_connection(
        &self,
        params: &JsonValue,
    ) -> Result<AsyncOpenedConnection, ProtocolError> {
        let open: ConnOpenParams =
            serde_json::from_value(params.clone()).map_err(invalid_params)?;
        let config: RedisConnectionConfig =
            serde_json::from_value(open.config).map_err(invalid_params)?;
        let manager = open_connection_manager(&config).await?;
        let conn_id = self.next_conn_id.fetch_add(1, Ordering::Relaxed);
        let mut connections = HashMap::new();
        connections.insert(config.database, manager);
        let result = ConnOpenResult {
            conn_id,
            server_info: None,
        };
        Ok(AsyncOpenedConnection {
            conn_id,
            open_result: serde_json::to_value(result).map_err(internal_error)?,
            connection: Box::new(RedisConnection {
                config,
                connections,
                events: HashMap::new(),
            }),
        })
    }

    async fn call_connless(
        &self,
        method_name: &str,
        _params: &JsonValue,
    ) -> Result<JsonValue, ProtocolError> {
        Err(ProtocolError::new(
            error_codes::METHOD_NOT_FOUND,
            format!("unsupported connless Redis method `{method_name}`"),
        ))
    }
}

#[async_trait]
impl AsyncDriverConnection for RedisConnection {
    async fn call(
        &mut self,
        method_name: &str,
        params: &JsonValue,
    ) -> Result<JsonValue, ProtocolError> {
        match method_name {
            method::REDIS_COMMAND => {
                let params: RedisCommandParams =
                    serde_json::from_value(params.clone()).map_err(invalid_params)?;
                let value = self.execute(params.database, params.args).await?;
                serde_json::to_value(RedisCommandResult { value }).map_err(internal_error)
            }
            method::REDIS_PIPELINE => {
                let params: RedisPipelineParams =
                    serde_json::from_value(params.clone()).map_err(invalid_params)?;
                params
                    .validate()
                    .map_err(|message| ProtocolError::new(error_codes::INVALID_PARAMS, message))?;
                let values = self
                    .execute_pipeline(params.database, params.commands)
                    .await?;
                serde_json::to_value(RedisPipelineResult { values }).map_err(internal_error)
            }
            method::EVENT_OPEN => self.open_event(params).await,
            method::EVENT_READ => self.read_event(params).await,
            method::EVENT_CLOSE => self.close_event(params).await,
            method::REDIS_PUBSUB_CONTROL => self.control_event(params).await,
            _ => Err(ProtocolError::new(
                error_codes::METHOD_NOT_FOUND,
                format!("unsupported Redis method `{method_name}`"),
            )),
        }
    }

    async fn close(&mut self) {
        self.connections.clear();
        for (_, event) in self.events.drain() {
            event.task.abort();
        }
    }
}

impl RedisConnection {
    fn database(&self, database: Option<u8>) -> u8 {
        database.unwrap_or(self.config.database)
    }

    async fn manager(&mut self, database: u8) -> Result<&mut ConnectionManager, ProtocolError> {
        if !self.connections.contains_key(&database) {
            let mut config = self.config.clone();
            config.database = database;
            let manager = open_connection_manager(&config).await?;
            self.connections.insert(database, manager);
        }
        self.connections.get_mut(&database).ok_or_else(|| {
            ProtocolError::new(error_codes::INTERNAL_ERROR, "Redis manager was not cached")
        })
    }

    async fn reconnect_manager(&mut self, database: u8) -> Result<(), ProtocolError> {
        self.connections.remove(&database);
        let mut config = self.config.clone();
        config.database = database;
        let manager = open_connection_manager(&config).await?;
        self.connections.insert(database, manager);
        Ok(())
    }

    async fn execute(
        &mut self,
        database: Option<u8>,
        args: Vec<WireBytes>,
    ) -> Result<RedisRespValue, ProtocolError> {
        let database = self.database(database);
        let replay_safe = can_retry_command(&args);
        let cmd = build_command(args)?;
        let result: Result<Value, RedisError> = {
            let manager = self.manager(database).await?;
            cmd.query_async(manager).await
        };
        let value = match result {
            Ok(value) => value,
            Err(error) if error.is_connection_dropped() => {
                let reconnect = self.reconnect_manager(database).await;
                if !replay_safe {
                    return Err(command_error(error, false));
                }
                reconnect?;
                cmd.query_async(self.manager(database).await?)
                    .await
                    .map_err(|error| command_error(error, true))?
            }
            Err(error) => return Err(command_error(error, replay_safe)),
        };
        Ok(redis_value(value))
    }

    async fn execute_pipeline(
        &mut self,
        database: Option<u8>,
        commands: Vec<Vec<WireBytes>>,
    ) -> Result<Vec<RedisRespValue>, ProtocolError> {
        let database = self.database(database);
        let replay_safe = can_retry_pipeline(&commands);
        let pipeline = build_pipeline(commands)?;
        let result: Result<Vec<Value>, RedisError> = {
            let manager = self.manager(database).await?;
            pipeline.query_async(manager).await
        };
        let values = match result {
            Ok(values) => values,
            Err(error) if error.is_connection_dropped() => {
                let reconnect = self.reconnect_manager(database).await;
                if !replay_safe {
                    return Err(command_error(error, false));
                }
                reconnect?;
                pipeline
                    .query_async(self.manager(database).await?)
                    .await
                    .map_err(|error| command_error(error, true))?
            }
            Err(error) => return Err(command_error(error, replay_safe)),
        };
        Ok(values.into_iter().map(redis_value).collect())
    }

    async fn open_event(&mut self, params: &JsonValue) -> Result<JsonValue, ProtocolError> {
        let params: EventOpenParams =
            serde_json::from_value(params.clone()).map_err(invalid_params)?;
        if params.kind != "redis_pubsub" {
            return Err(ProtocolError::new(
                error_codes::INVALID_PARAMS,
                "Redis sidecar only supports redis_pubsub event streams",
            ));
        }
        let capacity = params.capacity.unwrap_or(128).clamp(1, 1024) as usize;
        let client =
            Client::open(connection_url(&self.config).as_str()).map_err(connection_error)?;
        let pubsub = match connection_timeout(&self.config) {
            Some(duration) => timeout(duration, client.get_async_pubsub())
                .await
                .map_err(|_| connection_timeout_error("Redis Pub/Sub connection timed out"))?,
            None => client.get_async_pubsub().await,
        }
        .map_err(connection_error)?;
        let stream_id = format!("redis-events-{}", EVENT_ID.fetch_add(1, Ordering::Relaxed));
        let (command_tx, mut command_rx) = mpsc::channel::<PubSubCommand>(32);
        let (event_tx, event_rx) = mpsc::channel::<JsonValue>(capacity);
        let dropped = Arc::new(AtomicU64::new(0));
        let dropped_for_task = Arc::clone(&dropped);
        let task = tokio::spawn(async move {
            let (mut sink, mut stream) = pubsub.split();
            loop {
                tokio::select! {
                    command = command_rx.recv() => {
                        let Some(command) = command else { break; };
                        let result = match command {
                            PubSubCommand::Subscribe(channel) => sink.subscribe(channel).await,
                            PubSubCommand::PSubscribe(pattern) => sink.psubscribe(pattern).await,
                            PubSubCommand::Unsubscribe(channel) => sink.unsubscribe(channel).await,
                            PubSubCommand::PUnsubscribe(pattern) => sink.punsubscribe(pattern).await,
                        };
                        if result.is_err() { break; }
                    }
                    message = stream.next() => {
                        let Some(message) = message else { break; };
                        let payload = message.get_payload_bytes();
                        let payload = match String::from_utf8(payload.to_vec()) {
                            Ok(value) => json!({"encoding": "utf8", "value": value}),
                            Err(error) => json!({"encoding": "base64", "value": base64::engine::general_purpose::STANDARD.encode(error.into_bytes())}),
                        };
                        let event = json!({
                            "kind": if message.from_pattern() { "pmessage" } else { "message" },
                            "channel": message.get_channel_name(),
                            "pattern": message.get_pattern::<String>().ok(),
                            "payload": payload,
                        });
                        if event_tx.try_send(event).is_err() {
                            dropped_for_task.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            }
        });
        self.events.insert(
            stream_id.clone(),
            RedisEvent {
                commands: command_tx,
                events: Arc::new(Mutex::new(event_rx)),
                dropped,
                task,
            },
        );
        serde_json::to_value(EventOpenResult { stream_id }).map_err(internal_error)
    }

    async fn control_event(&mut self, params: &JsonValue) -> Result<JsonValue, ProtocolError> {
        let params: extension_protocol::redis::RedisPubSubControlParams =
            serde_json::from_value(params.clone()).map_err(invalid_params)?;
        let event = self.events.get(&params.stream_id).ok_or_else(|| {
            ProtocolError::new(error_codes::RESOURCE_CLOSED, "event stream is closed")
        })?;
        let command = match params.control {
            extension_protocol::redis::RedisPubSubControl::Subscribe(value) => {
                PubSubCommand::Subscribe(
                    String::from_utf8(decode_wire_bytes(value)?).map_err(invalid_params)?,
                )
            }
            extension_protocol::redis::RedisPubSubControl::PSubscribe(value) => {
                PubSubCommand::PSubscribe(
                    String::from_utf8(decode_wire_bytes(value)?).map_err(invalid_params)?,
                )
            }
            extension_protocol::redis::RedisPubSubControl::Unsubscribe(value) => {
                PubSubCommand::Unsubscribe(
                    String::from_utf8(decode_wire_bytes(value)?).map_err(invalid_params)?,
                )
            }
            extension_protocol::redis::RedisPubSubControl::PUnsubscribe(value) => {
                PubSubCommand::PUnsubscribe(
                    String::from_utf8(decode_wire_bytes(value)?).map_err(invalid_params)?,
                )
            }
        };
        event.commands.send(command).await.map_err(|_| {
            ProtocolError::new(error_codes::RESOURCE_CLOSED, "event stream is closed")
        })?;
        Ok(JsonValue::Null)
    }

    async fn read_event(&mut self, params: &JsonValue) -> Result<JsonValue, ProtocolError> {
        let params: EventReadParams =
            serde_json::from_value(params.clone()).map_err(invalid_params)?;
        let event = self.events.get(&params.stream_id).ok_or_else(|| {
            ProtocolError::new(error_codes::RESOURCE_CLOSED, "event stream is closed")
        })?;
        let max_events = params.effective_max_events() as usize;
        let mut receiver = event.events.lock().await;
        let mut values = Vec::with_capacity(max_events);
        if let Some(wait_ms) = params.wait_ms {
            if values.is_empty() && wait_ms > 0 {
                let first = timeout(Duration::from_millis(wait_ms as u64), receiver.recv()).await;
                if let Ok(Some(value)) = first {
                    values.push(value);
                }
            }
        }
        while values.len() < max_events {
            match receiver.try_recv() {
                Ok(value) => values.push(value),
                Err(_) => break,
            }
        }
        let dropped_count = event.dropped.swap(0, Ordering::Relaxed);
        serde_json::to_value(EventReadResult {
            events: values,
            closed: receiver.is_closed(),
            dropped_count,
        })
        .map_err(internal_error)
    }

    async fn close_event(&mut self, params: &JsonValue) -> Result<JsonValue, ProtocolError> {
        let params: EventCloseParams =
            serde_json::from_value(params.clone()).map_err(invalid_params)?;
        if let Some(event) = self.events.remove(&params.stream_id) {
            event.task.abort();
            Ok(JsonValue::Null)
        } else {
            Err(ProtocolError::new(
                error_codes::RESOURCE_CLOSED,
                "event stream is closed",
            ))
        }
    }
}

static EVENT_ID: AtomicU64 = AtomicU64::new(1);

fn decode_wire_bytes(value: WireBytes) -> Result<Vec<u8>, ProtocolError> {
    match value {
        WireBytes::Utf8(value) => Ok(value.into_bytes()),
        WireBytes::Base64(value) => base64::engine::general_purpose::STANDARD
            .decode(value)
            .map_err(|error| {
                ProtocolError::new(error_codes::DATA_INVALID_ENCODING, error.to_string())
            }),
    }
}

fn encode_bytes(bytes: Vec<u8>) -> WireBytes {
    match String::from_utf8(bytes) {
        Ok(value) => WireBytes::Utf8(value),
        Err(error) => {
            WireBytes::Base64(base64::engine::general_purpose::STANDARD.encode(error.into_bytes()))
        }
    }
}

fn redis_value(value: Value) -> RedisRespValue {
    match value {
        Value::Nil => RedisRespValue::Nil,
        Value::Int(value) => RedisRespValue::Integer(value),
        Value::BulkString(value) => RedisRespValue::Bytes(encode_bytes(value)),
        Value::Array(values) => {
            RedisRespValue::Array(values.into_iter().map(redis_value).collect())
        }
        Value::SimpleString(value) => RedisRespValue::SimpleString(value),
        Value::Okay => RedisRespValue::SimpleString("OK".into()),
        Value::Map(values) => RedisRespValue::Map(
            values
                .into_iter()
                .map(|(key, value)| (redis_value(key), redis_value(value)))
                .collect(),
        ),
        Value::Attribute { data, .. } => redis_value(*data),
        Value::Set(values) => RedisRespValue::Set(values.into_iter().map(redis_value).collect()),
        Value::Double(value) => RedisRespValue::Double(value),
        Value::Boolean(value) => RedisRespValue::Boolean(value),
        Value::VerbatimString { text, .. } => RedisRespValue::SimpleString(text),
        Value::BigNumber(value) => RedisRespValue::SimpleString(value.to_string()),
        Value::Push { data, .. } => {
            RedisRespValue::Array(data.into_iter().map(redis_value).collect())
        }
        Value::ServerError(error) => RedisRespValue::Error(format!("{error:?}")),
    }
}

fn connection_url(config: &RedisConnectionConfig) -> String {
    let scheme = if config.use_tls { "rediss" } else { "redis" };
    let auth = match (&config.username, &config.password) {
        (Some(username), Some(password)) => format!(
            "{}:{}@",
            encode_userinfo(username),
            encode_userinfo(password)
        ),
        // 未配置用户名时生成 `:pass@`（空用户名），使 redis-rs 以单参数 `AUTH <pass>`
        // 认证；若生成 `default:<pass>@` 会以双参数 `AUTH default <pass>` 认证，
        // 在不支持/不期望用户名认证的 Redis 上会导致连接失败或超时。
        (None, Some(password)) => format!(":{}@", encode_userinfo(password)),
        _ => String::new(),
    };
    let host = if config.host.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{}]", config.host)
    } else {
        config.host.clone()
    };
    format!(
        "{scheme}://{auth}{host}:{}/{}",
        config.port, config.database
    )
}

fn encode_userinfo(value: &str) -> String {
    utf8_percent_encode(value, REDIS_USERINFO_ENCODE_SET).to_string()
}

fn connection_timeout(config: &RedisConnectionConfig) -> Option<Duration> {
    config
        .connect_timeout_ms
        .filter(|milliseconds| *milliseconds > 0)
        .map(|milliseconds| Duration::from_millis(milliseconds.into()))
}

async fn open_connection_manager(
    config: &RedisConnectionConfig,
) -> Result<ConnectionManager, ProtocolError> {
    let client = Client::open(connection_url(config).as_str()).map_err(connection_error)?;
    let manager_config = connection_manager_config(config);
    client
        .get_connection_manager_with_config(manager_config)
        .await
        .map_err(connection_error)
}

fn connection_manager_config(config: &RedisConnectionConfig) -> ConnectionManagerConfig {
    match connection_timeout(config) {
        Some(duration) => ConnectionManagerConfig::new()
            .set_connection_timeout(duration)
            .set_response_timeout(duration),
        None => ConnectionManagerConfig::new(),
    }
}

fn build_pipeline(commands: Vec<Vec<WireBytes>>) -> Result<Pipeline, ProtocolError> {
    let mut pipeline = Pipeline::with_capacity(commands.len());
    for args in commands {
        pipeline.add_command(build_command(args)?);
    }
    Ok(pipeline)
}

fn build_command(args: Vec<WireBytes>) -> Result<Cmd, ProtocolError> {
    let mut args = args.into_iter();
    let command = args.next().ok_or_else(|| {
        ProtocolError::new(error_codes::INVALID_PARAMS, "Redis command cannot be empty")
    })?;
    let mut cmd = Cmd::new();
    cmd.arg(decode_wire_bytes(command)?);
    for arg in args {
        cmd.arg(decode_wire_bytes(arg)?);
    }
    Ok(cmd)
}

fn can_retry_pipeline(commands: &[Vec<WireBytes>]) -> bool {
    !commands.is_empty() && commands.iter().all(|args| can_retry_command(args))
}

fn can_retry_command(args: &[WireBytes]) -> bool {
    let Some(command) = args.first().and_then(wire_bytes_for_retry_check) else {
        return false;
    };
    if command.eq_ignore_ascii_case(b"MEMORY") {
        return args
            .get(1)
            .and_then(wire_bytes_for_retry_check)
            .is_some_and(|subcommand| subcommand.eq_ignore_ascii_case(b"USAGE"));
    }
    [
        b"PING".as_slice(),
        b"GET",
        b"MGET",
        b"EXISTS",
        b"TYPE",
        b"TTL",
        b"PTTL",
        b"SCAN",
        b"SSCAN",
        b"HSCAN",
        b"ZSCAN",
        b"KEYS",
        b"STRLEN",
        b"HLEN",
        b"HGET",
        b"HMGET",
        b"HGETALL",
        b"HKEYS",
        b"HVALS",
        b"LLEN",
        b"LRANGE",
        b"SCARD",
        b"SMEMBERS",
        b"ZCARD",
        b"ZRANGE",
        b"XRANGE",
        b"XLEN",
        b"INFO",
        b"DBSIZE",
    ]
    .iter()
    .any(|read_only| command.eq_ignore_ascii_case(read_only))
}

fn wire_bytes_for_retry_check(value: &WireBytes) -> Option<Vec<u8>> {
    match value {
        WireBytes::Utf8(value) => Some(value.as_bytes().to_vec()),
        WireBytes::Base64(value) => base64::engine::general_purpose::STANDARD.decode(value).ok(),
    }
}

fn invalid_params(error: impl std::fmt::Display) -> ProtocolError {
    ProtocolError::new(error_codes::INVALID_PARAMS, error.to_string())
}

fn connection_error(error: RedisError) -> ProtocolError {
    let code = if error.is_timeout() {
        error_codes::IO_TIMEOUT
    } else {
        error_codes::IO_CONNECTION_REFUSED
    };
    redis_protocol_error(code, error)
}

fn command_error(error: RedisError, replay_safe: bool) -> ProtocolError {
    redis_protocol_error_with_retryability(error_codes::EXTENSION_CUSTOM_START, error, replay_safe)
}

fn connection_timeout_error(message: impl Into<String>) -> ProtocolError {
    let message = message.into();
    ProtocolError::new(error_codes::IO_TIMEOUT, message.clone()).with_data(
        ErrorData::new().retryable(true).with_extra(json!({
            "category": "timeout",
            "source": message,
        })),
    )
}

fn redis_protocol_error(code: i32, error: RedisError) -> ProtocolError {
    redis_protocol_error_with_retryability(code, error, true)
}

fn redis_protocol_error_with_retryability(
    code: i32,
    error: RedisError,
    replay_safe: bool,
) -> ProtocolError {
    let message = error.to_string();
    let retryable = replay_safe
        && (error.is_timeout()
            || error.is_connection_refusal()
            || error.is_connection_dropped()
            || error.is_cluster_error());
    ProtocolError::new(code, message.clone()).with_data(
        ErrorData::new().retryable(retryable).with_extra(json!({
            "kind": format!("{:?}", error.kind()),
            "category": error.category(),
            "code": error.code(),
            "detail": error.detail(),
            "source": message,
        })),
    )
}

fn internal_error(error: impl std::fmt::Display) -> ProtocolError {
    ProtocolError::new(error_codes::INTERNAL_ERROR, error.to_string())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    serve_async_from_env(
        RedisDriver {
            next_conn_id: AtomicU64::new(1),
        },
        SOCKET_ENV,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_values_round_trip_through_base64_wire_form() {
        let bytes = vec![0, 0xff, 1];
        let wire = encode_bytes(bytes.clone());
        assert_eq!(bytes, decode_wire_bytes(wire).unwrap());
    }

    #[test]
    fn redis_arrays_preserve_nested_value_kinds() {
        assert_eq!(
            RedisRespValue::Array(vec![
                RedisRespValue::Integer(1),
                RedisRespValue::Bytes(WireBytes::Utf8("ok".into())),
            ]),
            redis_value(Value::Array(vec![
                Value::Int(1),
                Value::BulkString(b"ok".to_vec()),
            ]))
        );
    }

    #[test]
    fn connection_url_percent_encodes_username_and_password() {
        let config = RedisConnectionConfig {
            host: "redis.internal".into(),
            port: 6379,
            username: Some("user:name@example.com".into()),
            password: Some("p@ss:/?#%word".into()),
            database: 3,
            use_tls: false,
            connect_timeout_ms: None,
        };

        assert_eq!(
            "redis://user%3Aname%40example.com:p%40ss%3A%2F%3F%23%25word@redis.internal:6379/3",
            connection_url(&config)
        );
        Client::open(connection_url(&config).as_str()).expect("encoded Redis URL should parse");
    }

    #[test]
    fn connection_url_uses_empty_username_for_password_only_auth() {
        let config = RedisConnectionConfig {
            host: "127.0.0.1".into(),
            port: 6379,
            username: None,
            password: Some("secret".into()),
            database: 0,
            use_tls: true,
            connect_timeout_ms: None,
        };

        assert_eq!(
            "rediss://:secret@127.0.0.1:6379/0",
            connection_url(&config)
        );
    }

    #[test]
    fn connection_url_empty_username_is_same_as_no_username() {
        let config = RedisConnectionConfig {
            host: "127.0.0.1".into(),
            port: 6379,
            username: Some(String::new()),
            password: Some("secret".into()),
            database: 0,
            use_tls: false,
            connect_timeout_ms: None,
        };

        assert_eq!(
            "redis://:secret@127.0.0.1:6379/0",
            connection_url(&config)
        );
    }

    #[test]
    fn connection_url_brackets_ipv6_hosts() {
        let config = RedisConnectionConfig {
            host: "::1".into(),
            port: 6379,
            username: None,
            password: None,
            database: 0,
            use_tls: false,
            connect_timeout_ms: None,
        };

        assert_eq!("redis://[::1]:6379/0", connection_url(&config));
        Client::open(connection_url(&config).as_str()).expect("IPv6 Redis URL should parse");
    }

    #[test]
    fn connection_timeout_uses_wire_milliseconds() {
        let config = RedisConnectionConfig {
            host: "127.0.0.1".into(),
            port: 6379,
            username: None,
            password: None,
            database: 0,
            use_tls: false,
            connect_timeout_ms: Some(2500),
        };

        assert_eq!(
            Some(Duration::from_millis(2500)),
            connection_timeout(&config)
        );
    }

    #[test]
    fn connection_manager_config_applies_connection_and_response_timeouts() {
        let config = RedisConnectionConfig {
            host: "127.0.0.1".into(),
            port: 6379,
            username: None,
            password: None,
            database: 0,
            use_tls: false,
            connect_timeout_ms: Some(2500),
        };

        let debug = format!("{:?}", connection_manager_config(&config));

        assert!(debug.contains("response_timeout: Some(2.5s)"));
        assert!(debug.contains("connection_timeout: Some(2.5s)"));
    }

    #[test]
    fn native_pipeline_packs_all_commands_into_one_redis_request() {
        let mut commands = (0..200)
            .map(|index| {
                vec![
                    WireBytes::Utf8("TYPE".into()),
                    WireBytes::Utf8(format!("key-{index}")),
                ]
            })
            .collect::<Vec<_>>();
        commands.push(vec![
            WireBytes::Utf8("TYPE".into()),
            WireBytes::Base64("AP8=".into()),
        ]);

        let packed = build_pipeline(commands)
            .expect("pipeline should build")
            .get_packed_pipeline();

        assert_eq!(
            201,
            packed
                .windows(b"$4\r\nTYPE\r\n".len())
                .filter(|window| *window == b"$4\r\nTYPE\r\n")
                .count()
        );
        assert!(packed.ends_with(b"*2\r\n$4\r\nTYPE\r\n$2\r\n\0\xff\r\n"));
    }

    #[test]
    fn pipeline_rejects_an_empty_command_before_sending_anything() {
        let error = match build_pipeline(vec![vec![WireBytes::Utf8("PING".into())], Vec::new()]) {
            Ok(_) => panic!("empty pipeline command should fail"),
            Err(error) => error,
        };

        assert_eq!(error_codes::INVALID_PARAMS, error.code);
        assert!(error.message.contains("cannot be empty"));
    }

    #[test]
    fn redis_errors_preserve_native_details() {
        let error = RedisError::from((
            redis_client::ErrorKind::AuthenticationFailed,
            "Redis authentication failed",
            "NOAUTH Authentication required".to_string(),
        ));

        let protocol_error = command_error(error, true);
        let data = protocol_error.data.expect("Redis error data");
        let extra = data.extra.expect("Redis native details");

        assert!(
            protocol_error
                .message
                .contains("NOAUTH Authentication required")
        );
        assert_eq!(
            Some("AuthenticationFailed"),
            extra.get("kind").and_then(JsonValue::as_str)
        );
        assert_eq!(
            Some("authentication failed"),
            extra.get("category").and_then(JsonValue::as_str)
        );
        assert_eq!(Some(false), data.retryable);
    }

    #[test]
    fn redis_timeouts_use_timeout_code_and_are_retryable() {
        let error = RedisError::from(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "Redis socket timed out",
        ));

        let protocol_error = connection_error(error);

        assert_eq!(error_codes::IO_TIMEOUT, protocol_error.code);
        assert!(protocol_error.message.contains("Redis socket timed out"));
        assert_eq!(
            Some(true),
            protocol_error.data.and_then(|data| data.retryable)
        );
    }

    #[test]
    fn only_known_read_only_commands_are_safe_to_replay() {
        for command in [
            "PING", "get", "MGET", "EXISTS", "TYPE", "TTL", "PTTL", "SCAN", "SSCAN", "HSCAN",
            "ZSCAN", "KEYS", "STRLEN", "HLEN", "HGET", "HMGET", "HGETALL", "HKEYS", "HVALS",
            "LLEN", "LRANGE", "SCARD", "SMEMBERS", "ZCARD", "ZRANGE", "XRANGE", "XLEN", "INFO",
            "DBSIZE",
        ] {
            assert!(
                can_retry_command(&[WireBytes::Utf8(command.into())]),
                "{command} should be safe to replay"
            );
        }

        for command in ["SET", "DEL", "INCR", "EVAL", "MULTI", "SELECT", "UNKNOWN"] {
            assert!(
                !can_retry_command(&[WireBytes::Utf8(command.into())]),
                "{command} must not be replayed"
            );
        }
        assert!(!can_retry_command(&[]));
        assert!(!can_retry_command(&[WireBytes::Base64("AP8=".into())]));
    }

    #[test]
    fn memory_retry_requires_the_read_only_usage_subcommand() {
        assert!(can_retry_command(&[
            WireBytes::Utf8("MEMORY".into()),
            WireBytes::Utf8("usage".into()),
            WireBytes::Utf8("key".into()),
        ]));
        assert!(!can_retry_command(&[
            WireBytes::Utf8("MEMORY".into()),
            WireBytes::Utf8("PURGE".into()),
        ]));
        assert!(!can_retry_command(&[WireBytes::Utf8("MEMORY".into())]));
    }

    #[test]
    fn pipeline_retry_requires_every_command_to_be_read_only() {
        let read_only = vec![
            vec![
                WireBytes::Utf8("TYPE".into()),
                WireBytes::Utf8("key".into()),
            ],
            vec![WireBytes::Utf8("TTL".into()), WireBytes::Utf8("key".into())],
        ];
        assert!(can_retry_pipeline(&read_only));

        let mut mixed = read_only;
        mixed.push(vec![
            WireBytes::Utf8("SET".into()),
            WireBytes::Utf8("key".into()),
            WireBytes::Utf8("value".into()),
        ]);
        assert!(!can_retry_pipeline(&mixed));
        assert!(!can_retry_pipeline(&[]));
    }

    #[test]
    fn broken_pipe_is_reconnectable_but_ambiguous_writes_are_not_retryable() {
        let broken_pipe = || {
            RedisError::from(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "broken pipe",
            ))
        };

        assert!(broken_pipe().is_connection_dropped());
        assert_eq!(
            Some(true),
            command_error(broken_pipe(), true)
                .data
                .and_then(|data| data.retryable)
        );
        assert_eq!(
            Some(false),
            command_error(broken_pipe(), false)
                .data
                .and_then(|data| data.retryable)
        );
    }
}
