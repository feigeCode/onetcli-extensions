use std::collections::BTreeSet;
use std::time::Duration;

use base64::Engine;
use extension_host::{FramedTransport, JsonRpcClient, JsonRpcClientHandle, RequestOptions};
use extension_protocol::lifecycle::{InitParams, InitResult};
use extension_protocol::method;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

const SCHEMA_OBJECT_VIEW: &str = "schema/object_view";

#[tokio::test]
async fn duckdb_driver_declared_methods_are_callable() {
    let (client_stream, server_stream) = tokio::io::duplex(1024 * 1024);
    let (server_reader, server_writer) = tokio::io::split(server_stream);
    let server = tokio::spawn(async move {
        duckdb_driver::server::handle_stream(server_reader, server_writer).await
    });

    let (reader, writer) = tokio::io::split(client_stream);
    let client = JsonRpcClient::start(FramedTransport::new(reader, writer));
    let handle = client.handle();
    let timeout = RequestOptions::default().with_timeout(Duration::from_secs(2));
    let mut called = BTreeSet::new();

    let ping = call_value(&handle, &timeout, &mut called, method::PING, json!({})).await;
    assert_eq!(Some(true), ping["pong"].as_bool());

    let init: InitResult = handle
        .call(
            method::INIT,
            serde_json::to_value(InitParams::new("onetcli-test", "duckdb-all-methods")).unwrap(),
            timeout.clone(),
        )
        .await
        .expect("init should succeed");
    for method_name in declared_driver_methods() {
        assert!(
            init.declares_method(&method_name),
            "init result should declare {method_name}"
        );
    }

    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("all-methods.db");
    let config = json!({ "host": db_path.to_string_lossy() });

    let conn_test: Value = call(
        &handle,
        &timeout,
        &mut called,
        method::CONN_TEST,
        json!({ "driver_id": "duckdb", "config": config }),
    )
    .await;
    assert_eq!(Some(true), conn_test["ok"].as_bool());

    let open: Value = call(
        &handle,
        &timeout,
        &mut called,
        method::CONN_OPEN,
        json!({ "driver_id": "duckdb", "config": config }),
    )
    .await;
    let conn_id = open["conn_id"].as_u64().expect("conn/open returns conn_id");

    let ping = call_value(
        &handle,
        &timeout,
        &mut called,
        method::CONN_PING,
        json!({ "conn_id": conn_id }),
    )
    .await;
    assert!(ping["latency_ms"].is_u64());

    let databases: Vec<Value> = call(
        &handle,
        &timeout,
        &mut called,
        method::SCHEMA_DATABASES,
        json!({ "conn_id": conn_id }),
    )
    .await;
    let database = databases
        .first()
        .and_then(|db| db["name"].as_str())
        .expect("schema/databases returns current database")
        .to_string();

    call_value(
        &handle,
        &timeout,
        &mut called,
        method::CONN_USE,
        json!({ "conn_id": conn_id, "database": database }),
    )
    .await;

    call_value(
        &handle,
        &timeout,
        &mut called,
        method::EXEC_RUN,
        json!({
            "conn_id": conn_id,
            "sql": "CREATE TABLE events(id INTEGER PRIMARY KEY, name TEXT, amount INTEGER CHECK(amount >= 0))"
        }),
    )
    .await;
    call_value(
        &handle,
        &timeout,
        &mut called,
        method::EXEC_RUN,
        json!({
            "conn_id": conn_id,
            "sql": "CREATE VIEW event_names AS SELECT name FROM events"
        }),
    )
    .await;

    let batch: Value = call(
        &handle,
        &timeout,
        &mut called,
        method::EXEC_BATCH,
        json!({
            "conn_id": conn_id,
            "statements": [
                "INSERT INTO events VALUES (1, 'Ada', 10)",
                "INSERT INTO events VALUES (2, 'Linus', 20)",
                "CREATE INDEX idx_events_name ON events(name)"
            ],
            "stop_on_error": true,
            "in_transaction": true
        }),
    )
    .await;
    assert!(batch["errors"].as_array().unwrap().is_empty());

    let query: Value = call(
        &handle,
        &timeout,
        &mut called,
        method::QUERY_START,
        json!({
            "conn_id": conn_id,
            "sql": "SELECT id, name FROM events ORDER BY id"
        }),
    )
    .await;
    let cursor_id = query["cursor_id"].as_str().unwrap().to_string();
    assert_eq!(2, query["columns"].as_array().unwrap().len());

    let fetched: Value = call(
        &handle,
        &timeout,
        &mut called,
        method::CURSOR_FETCH,
        json!({ "cursor_id": cursor_id, "n": 10 }),
    )
    .await;
    assert_eq!(2, fetched["rows"].as_array().unwrap().len());
    assert_eq!(Some(true), fetched["done"].as_bool());
    call_value(
        &handle,
        &timeout,
        &mut called,
        method::CURSOR_CLOSE,
        json!({ "cursor_id": cursor_id }),
    )
    .await;

    let cancel_query: Value = call(
        &handle,
        &timeout,
        &mut called,
        method::QUERY_START,
        json!({ "conn_id": conn_id, "sql": "SELECT * FROM range(100)" }),
    )
    .await;
    let cancel_cursor_id = cancel_query["cursor_id"].as_str().unwrap().to_string();
    call_value(
        &handle,
        &timeout,
        &mut called,
        method::CURSOR_CANCEL,
        json!({ "cursor_id": cancel_cursor_id }),
    )
    .await;
    call_value(
        &handle,
        &timeout,
        &mut called,
        method::CURSOR_CLOSE,
        json!({ "cursor_id": cancel_cursor_id }),
    )
    .await;

    let tx_commit: Value = call(
        &handle,
        &timeout,
        &mut called,
        method::TX_BEGIN,
        json!({ "conn_id": conn_id }),
    )
    .await;
    let commit_tx_id = tx_commit["tx_id"].as_str().unwrap().to_string();
    call_value(
        &handle,
        &timeout,
        &mut called,
        method::EXEC_RUN,
        json!({
            "conn_id": conn_id,
            "tx_id": commit_tx_id,
            "sql": "INSERT INTO events VALUES (3, 'Grace', 30)"
        }),
    )
    .await;
    call_value(
        &handle,
        &timeout,
        &mut called,
        method::TX_COMMIT,
        json!({ "tx_id": commit_tx_id }),
    )
    .await;

    let tx_rollback: Value = call(
        &handle,
        &timeout,
        &mut called,
        method::TX_BEGIN,
        json!({ "conn_id": conn_id }),
    )
    .await;
    let rollback_tx_id = tx_rollback["tx_id"].as_str().unwrap().to_string();
    call_value(
        &handle,
        &timeout,
        &mut called,
        method::EXEC_RUN,
        json!({
            "conn_id": conn_id,
            "tx_id": rollback_tx_id,
            "sql": "INSERT INTO events VALUES (4, 'Rollback', 40)"
        }),
    )
    .await;
    call_value(
        &handle,
        &timeout,
        &mut called,
        method::TX_ROLLBACK,
        json!({ "tx_id": rollback_tx_id }),
    )
    .await;

    let schemas: Vec<Value> = call(
        &handle,
        &timeout,
        &mut called,
        method::SCHEMA_SCHEMAS,
        json!({ "conn_id": conn_id, "database": database }),
    )
    .await;
    assert!(schemas.iter().any(|schema| schema["name"] == "main"));

    let objects: Vec<Value> = call(
        &handle,
        &timeout,
        &mut called,
        method::SCHEMA_OBJECTS,
        json!({
            "conn_id": conn_id,
            "database": database,
            "schema": "main",
            "kinds": ["table", "view"]
        }),
    )
    .await;
    assert!(objects.iter().any(|object| object["name"] == "events"));
    assert!(objects.iter().any(|object| object["name"] == "event_names"));

    let columns: Vec<Value> = call(
        &handle,
        &timeout,
        &mut called,
        method::SCHEMA_COLUMNS,
        json!({
            "conn_id": conn_id,
            "database": database,
            "schema": "main",
            "table": "events"
        }),
    )
    .await;
    assert!(columns.iter().any(|column| column["name"] == "id"));
    assert!(columns.iter().any(|column| column["name"] == "amount"));

    let column_view: Value = call(
        &handle,
        &timeout,
        &mut called,
        SCHEMA_OBJECT_VIEW,
        json!({
            "conn_id": conn_id,
            "view": "columns",
            "database": database,
            "schema": "main",
            "table": "events"
        }),
    )
    .await;
    assert_eq!(Some("Columns"), column_view["title"].as_str());
    assert_eq!(Some("name"), column_view["columns"][0]["key"].as_str());
    assert_eq!(Some("Field"), column_view["columns"][0]["name"].as_str());
    assert!(
        column_view["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row[0] == "amount" && row[1] == "INTEGER")
    );

    let views: Vec<Value> = call(
        &handle,
        &timeout,
        &mut called,
        method::SCHEMA_VIEWS,
        json!({ "conn_id": conn_id, "database": database, "schema": "main" }),
    )
    .await;
    assert!(views.iter().any(|view| view["name"] == "event_names"));

    let indexes: Vec<Value> = call(
        &handle,
        &timeout,
        &mut called,
        method::SCHEMA_INDEXES,
        json!({
            "conn_id": conn_id,
            "database": database,
            "schema": "main",
            "table": "events"
        }),
    )
    .await;
    assert!(
        indexes
            .iter()
            .any(|index| index["name"] == "idx_events_name")
    );

    let checks: Vec<Value> = call(
        &handle,
        &timeout,
        &mut called,
        method::SCHEMA_CHECKS,
        json!({ "conn_id": conn_id, "schema": "main", "table": "events" }),
    )
    .await;
    assert!(checks.iter().any(|check| {
        check["definition"]
            .as_str()
            .is_some_and(|sql| sql.contains("amount"))
    }));

    let functions: Vec<Value> = call(
        &handle,
        &timeout,
        &mut called,
        method::SCHEMA_FUNCTIONS,
        json!({ "conn_id": conn_id, "database": database }),
    )
    .await;
    assert!(functions.iter().any(|function| function["name"] == "lower"));

    call_value(
        &handle,
        &timeout,
        &mut called,
        method::DATA_EXPORT,
        json!({
            "conn_id": conn_id,
            "sql": "SELECT id, name FROM events ORDER BY id",
            "format": "ndjson",
            "stream_id": "all-methods-export"
        }),
    )
    .await;
    let stream: Value = call(
        &handle,
        &timeout,
        &mut called,
        method::STREAM_READ,
        json!({ "stream_id": "all-methods-export", "max_bytes": 4096 }),
    )
    .await;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(stream["data"].as_str().unwrap())
        .unwrap();
    let exported = String::from_utf8(bytes).unwrap();
    assert!(exported.contains(r#""name":"Ada""#));
    call_value(
        &handle,
        &timeout,
        &mut called,
        method::STREAM_CLOSE,
        json!({ "stream_id": "all-methods-export" }),
    )
    .await;

    call_value(
        &handle,
        &timeout,
        &mut called,
        method::EXEC_RUN,
        json!({
            "conn_id": conn_id,
            "sql": "CREATE TABLE imported_commit(id INTEGER, name TEXT)"
        }),
    )
    .await;
    let import_begin: Value = call(
        &handle,
        &timeout,
        &mut called,
        method::DATA_IMPORT_BEGIN,
        json!({
            "conn_id": conn_id,
            "table": "imported_commit",
            "format": "json",
            "columns": ["id", "name"]
        }),
    )
    .await;
    let import_id = import_begin["import_id"].as_str().unwrap().to_string();
    let import_chunk: Value = call(
        &handle,
        &timeout,
        &mut called,
        method::DATA_IMPORT_CHUNK,
        json!({
            "import_id": import_id,
            "rows": [[
                { "type": "i64", "value": 10 },
                { "type": "text", "value": "Imported" }
            ]]
        }),
    )
    .await;
    assert_eq!(Some(1), import_chunk["inserted"].as_u64());
    let import_commit: Value = call(
        &handle,
        &timeout,
        &mut called,
        method::DATA_IMPORT_COMMIT,
        json!({ "import_id": import_id }),
    )
    .await;
    assert_eq!(Some(1), import_commit["inserted"].as_u64());

    call_value(
        &handle,
        &timeout,
        &mut called,
        method::EXEC_RUN,
        json!({
            "conn_id": conn_id,
            "sql": "CREATE TABLE imported_abort(id INTEGER, name TEXT)"
        }),
    )
    .await;
    let import_abort_begin: Value = call(
        &handle,
        &timeout,
        &mut called,
        method::DATA_IMPORT_BEGIN,
        json!({
            "conn_id": conn_id,
            "table": "imported_abort",
            "format": "json",
            "columns": ["id", "name"]
        }),
    )
    .await;
    let abort_import_id = import_abort_begin["import_id"]
        .as_str()
        .unwrap()
        .to_string();
    call_value(
        &handle,
        &timeout,
        &mut called,
        method::DATA_IMPORT_CHUNK,
        json!({
            "import_id": abort_import_id,
            "rows": [[
                { "type": "i64", "value": 20 },
                { "type": "text", "value": "Abort" }
            ]]
        }),
    )
    .await;
    call_value(
        &handle,
        &timeout,
        &mut called,
        method::DATA_IMPORT_ABORT,
        json!({ "import_id": abort_import_id }),
    )
    .await;

    let create_spec = json!({
        "name": "ddl_created",
        "schema": "main",
        "columns": [
            { "name": "id", "type": "INTEGER", "nullable": false },
            { "name": "body", "type": "TEXT" }
        ],
        "primary_key": ["id"]
    });
    let create_sql: Value = call(
        &handle,
        &timeout,
        &mut called,
        method::DDL_BUILD_CREATE_TABLE,
        json!({ "spec": create_spec, "options": { "if_not_exists": true } }),
    )
    .await;
    assert!(create_sql["sql"].as_str().unwrap().contains("CREATE TABLE"));

    let alter_sql: Value = call(
        &handle,
        &timeout,
        &mut called,
        method::DDL_BUILD_ALTER_TABLE,
        json!({
            "from_spec": {
                "name": "ddl_created",
                "schema": "main",
                "columns": [{ "name": "id", "type": "INTEGER" }]
            },
            "to_spec": {
                "name": "ddl_created",
                "schema": "main",
                "columns": [
                    { "name": "id", "type": "INTEGER" },
                    { "name": "body", "type": "TEXT" }
                ]
            },
            "column_renames": [],
            "options": { "allow_destructive": true }
        }),
    )
    .await;
    assert!(!alter_sql["statements"].as_array().unwrap().is_empty());

    let drop_sql: Value = call(
        &handle,
        &timeout,
        &mut called,
        method::DDL_BUILD_DROP,
        json!({ "kind": "table", "name": "ddl_created", "schema": "main", "if_exists": true }),
    )
    .await;
    assert!(drop_sql["sql"].as_str().unwrap().contains("DROP TABLE"));

    let generic_ddl: Value = call(
        &handle,
        &timeout,
        &mut called,
        method::DDL_BUILD,
        json!({
            "op": "column_definition",
            "payload": { "name": "payload", "type": "TEXT", "nullable": false }
        }),
    )
    .await;
    assert_eq!(
        Some("\"payload\" TEXT NOT NULL"),
        generic_ddl["statements"][0].as_str()
    );

    call_value(
        &handle,
        &timeout,
        &mut called,
        method::CONN_CLOSE,
        json!({ "conn_id": conn_id }),
    )
    .await;

    call_value(
        &handle,
        &timeout,
        &mut called,
        method::SHUTDOWN,
        json!({ "grace_ms": 100 }),
    )
    .await;

    assert_eq!(declared_driver_methods(), called);

    client.shutdown().await;
    let _ = server.await;
}

async fn call<T>(
    handle: &JsonRpcClientHandle,
    timeout: &RequestOptions,
    called: &mut BTreeSet<String>,
    method_name: &str,
    params: Value,
) -> T
where
    T: DeserializeOwned,
{
    let value = call_value(handle, timeout, called, method_name, params).await;
    serde_json::from_value(value)
        .unwrap_or_else(|error| panic!("{method_name} should return expected shape: {error}"))
}

async fn call_value(
    handle: &JsonRpcClientHandle,
    timeout: &RequestOptions,
    called: &mut BTreeSet<String>,
    method_name: &str,
    params: Value,
) -> Value {
    let value = handle
        .call_raw(method_name, params, timeout.clone())
        .await
        .unwrap_or_else(|error| panic!("{method_name} should succeed: {error}"));
    called.insert(method_name.to_string());
    value
}

fn declared_driver_methods() -> BTreeSet<String> {
    serde_json::from_str::<Value>(include_str!("../driver.json")).unwrap()["methods"]
        .as_array()
        .unwrap()
        .iter()
        .map(|method| method.as_str().unwrap().to_string())
        .collect()
}
