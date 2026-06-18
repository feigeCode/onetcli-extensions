#![allow(clippy::result_large_err)]

use std::sync::atomic::{AtomicI64, Ordering};

use base64::Engine;
use extension_protocol::conn::{ConnPingParams, ConnTestParams, ConnUseParams};
use extension_protocol::data::{
    DataFormat, ExportParams, ExportResult, ImportAbortParams, ImportBeginParams,
    ImportBeginResult, ImportChunkParams, ImportChunkResult, ImportCommitParams,
    ImportCommitResult, StreamCloseParams, StreamReadParams, StreamReadResult,
};
use extension_protocol::error::{ProtocolError, error_codes};
use extension_protocol::lifecycle::{Capability, InitParams, InitResult, ShutdownParams};
use extension_protocol::method;
use extension_protocol::query::{
    CursorCancelParams, CursorCloseParams, CursorFetchParams, ExecBatchParams, ExecBatchResult,
    ExecRunParams, ExecRunResult, IsolationLevel, QueryStartParams, TxBeginParams, TxCommitParams,
    TxRollbackParams,
};
use extension_protocol::row::{CellValue, Row};
use extension_protocol::schema::{
    ChecksParams, ColumnInfo, ColumnsParams, DatabaseInfo, DatabasesParams, FunctionInfo,
    FunctionsParams, IndexInfo, IndexesParams, ObjectInfo, ObjectKind, ObjectsParams, SchemaInfo,
    SchemasParams, ViewInfo, ViewsParams,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::IotDbConnectionConfig;
use crate::server::{
    invalid_params, not_supported, params_deserialize_error, protocol_error_from_anyhow,
};
use crate::session::IotDbSession;
use crate::state::{ConnectionState, CursorState, ImportState, StreamState};

pub const SCHEMA_OBJECT_VIEW: &str = "schema/object_view";

pub fn handle_init(initialized: &AtomicI64, params: &Value) -> Result<Value, ProtocolError> {
    let _params: InitParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    if initialized
        .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err(ProtocolError::new(
            error_codes::ALREADY_INITIALIZED,
            "driver already initialized",
        ));
    }

    let mut result = InitResult::new(env!("CARGO_PKG_VERSION"))
        .with_api("database", "1.0")
        .with_feature(Capability::RICH_ERRORS)
        .with_feature(Capability::DDL_BUILDER)
        .with_feature(Capability::BATCH_EXEC)
        .with_feature(Capability::DATA_PIPE)
        .with_feature(Capability::SCHEMA_INTROSPECTION)
        .with_driver("iotdb");
    for method_name in declared_methods() {
        result = result.with_method(*method_name);
    }
    serde_json::to_value(result).map_err(params_deserialize_error)
}

pub fn handle_shutdown(params: &Value) -> Result<Value, ProtocolError> {
    let _: ShutdownParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    Ok(Value::Null)
}

pub fn handle_conn_test(params: &Value) -> Result<Value, ProtocolError> {
    let started = std::time::Instant::now();
    let p: ConnTestParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    ensure_driver(&p.driver_id)?;
    let cfg: IotDbConnectionConfig =
        serde_json::from_value(p.config.clone()).map_err(params_deserialize_error)?;
    let mut session = IotDbSession::connect(cfg)
        .map_err(|e| protocol_error_from_anyhow(error_codes::IO_CONNECTION_REFUSED, e))?;
    session
        .ping()
        .map_err(|e| protocol_error_from_anyhow(error_codes::SERVER_CLOSED_CONNECTION, e))?;
    session.close();
    Ok(json!({
        "ok": true,
        "server_version": Option::<String>::None,
        "warnings": [],
        "latency_ms": started.elapsed().as_millis() as u32,
    }))
}

pub fn handle_conn_ping(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let _: ConnPingParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    let started = std::time::Instant::now();
    state
        .conn_mut()
        .ping()
        .map_err(|e| protocol_error_from_anyhow(error_codes::SERVER_CLOSED_CONNECTION, e))?;
    Ok(json!({ "latency_ms": started.elapsed().as_millis() as u32 }))
}

pub fn handle_conn_use(
    _state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let _: ConnUseParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    Ok(Value::Null)
}

pub fn handle_query_start(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: QueryStartParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    reject_params(&p.params)?;
    let parsed = state
        .conn_mut()
        .query(&p.sql)
        .map_err(|e| protocol_error_from_anyhow(error_codes::SQL_SYNTAX_ERROR, e))?;
    let columns = parsed.column_specs();
    let rows = parsed.protocol_rows();
    let cursor = CursorState::new(columns.clone(), rows, p.max_rows);
    let cursor_id = state.open_cursor(cursor);
    Ok(json!({
        "cursor_id": cursor_id,
        "columns": columns,
        "row_count_known": false,
        "row_count_estimate": Option::<u64>::None,
    }))
}

pub fn handle_cursor_fetch(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: CursorFetchParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    let cursor = state
        .cursor_mut(&p.cursor_id)
        .ok_or_else(|| unknown_cursor(&p.cursor_id))?;
    let (rows, done) = cursor.fetch(p.n);
    Ok(json!({ "rows": rows, "done": done }))
}

pub fn handle_cursor_close(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: CursorCloseParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    if !state.close_cursor(&p.cursor_id) {
        return Err(unknown_cursor(&p.cursor_id));
    }
    Ok(Value::Null)
}

pub fn handle_cursor_cancel(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: CursorCancelParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    let cursor = state
        .cursor_mut(&p.cursor_id)
        .ok_or_else(|| unknown_cursor(&p.cursor_id))?;
    cursor.cancel();
    Ok(Value::Null)
}

pub fn handle_exec_run(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: ExecRunParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    reject_params(&p.params)?;
    state
        .conn_mut()
        .exec_update(&p.sql)
        .map_err(|e| protocol_error_from_anyhow(error_codes::SQL_SYNTAX_ERROR, e))?;
    serde_json::to_value(ExecRunResult::default()).map_err(params_deserialize_error)
}

pub fn handle_exec_batch(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: ExecBatchParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    if p.in_transaction {
        return Err(unsupported_transactional_batch_error());
    }
    state
        .conn_mut()
        .exec_batch(p.statements)
        .map_err(|e| protocol_error_from_anyhow(error_codes::SQL_SYNTAX_ERROR, e))?;
    serde_json::to_value(ExecBatchResult::default()).map_err(params_deserialize_error)
}

fn unsupported_transactional_batch_error() -> ProtocolError {
    ProtocolError::new(
        error_codes::TX_NESTED_NOT_SUPPORTED,
        "IoTDB does not expose SQL transactions through the 0.0.7 Rust client",
    )
}

pub fn handle_tx_begin(
    _state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: TxBeginParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    if p.isolation
        .is_some_and(|level| level != IsolationLevel::Default)
    {
        return Err(ProtocolError::new(
            error_codes::TX_ISOLATION_NOT_SUPPORTED,
            "IoTDB driver does not support transaction isolation levels",
        ));
    }
    Err(ProtocolError::new(
        error_codes::TX_NESTED_NOT_SUPPORTED,
        "IoTDB does not expose SQL transactions through the 0.0.7 Rust client",
    ))
}

pub fn handle_tx_commit(
    _state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let _: TxCommitParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    Err(ProtocolError::new(
        error_codes::UNKNOWN_TX_ID,
        "IoTDB transaction support is unavailable",
    ))
}

pub fn handle_tx_rollback(
    _state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let _: TxRollbackParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    Err(ProtocolError::new(
        error_codes::UNKNOWN_TX_ID,
        "IoTDB transaction support is unavailable",
    ))
}

pub fn handle_data_export(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: ExportParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    let sql = export_sql(&p)?;
    let parsed = state
        .conn_mut()
        .query(&sql)
        .map_err(|e| protocol_error_from_anyhow(error_codes::SQL_SYNTAX_ERROR, e))?;
    let text = match p.format {
        DataFormat::Ndjson => rows_to_ndjson(&parsed.column_specs(), &parsed.protocol_rows()),
        DataFormat::Json => serde_json::to_string(&rows_to_json_objects(
            &parsed.column_specs(),
            &parsed.protocol_rows(),
        ))
        .map_err(params_deserialize_error)?,
        DataFormat::Csv => rows_to_csv(&parsed.column_specs(), &parsed.protocol_rows()),
        other => {
            return Err(not_supported(format!(
                "IoTDB data/export does not support format `{other:?}`"
            )));
        }
    };
    let stream = StreamState::new(text.into_bytes());
    let estimated_bytes = stream.estimated_bytes();
    state.insert_stream(p.stream_id, stream);
    serde_json::to_value(ExportResult {
        estimated_bytes: Some(estimated_bytes),
        estimated_rows: None,
        metadata: json!({ "format": format!("{:?}", p.format).to_ascii_lowercase() }),
    })
    .map_err(params_deserialize_error)
}

pub fn handle_stream_read(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: StreamReadParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    let stream = state
        .stream_mut(&p.stream_id)
        .ok_or_else(|| unknown_stream(&p.stream_id))?;
    let (chunk, done) = stream.read(p.max_bytes);
    serde_json::to_value(StreamReadResult {
        data: base64::engine::general_purpose::STANDARD.encode(chunk),
        done,
    })
    .map_err(params_deserialize_error)
}

pub fn handle_stream_close(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: StreamCloseParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    if !state.close_stream(&p.stream_id) {
        return Err(unknown_stream(&p.stream_id));
    }
    Ok(Value::Null)
}

pub fn handle_data_import_begin(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: ImportBeginParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    if matches!(
        p.format,
        DataFormat::Parquet | DataFormat::Xlsx | DataFormat::Sql
    ) {
        return Err(not_supported(format!(
            "IoTDB data/import does not support format `{:?}`",
            p.format
        )));
    }
    let import_id = format!("iotdb-import-{}", uuid::Uuid::new_v4());
    state.insert_import(
        import_id.clone(),
        ImportState::new(p.table, p.columns, p.options),
    );
    serde_json::to_value(ImportBeginResult { import_id }).map_err(params_deserialize_error)
}

pub fn handle_data_import_chunk(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: ImportChunkParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    let mut import = state
        .remove_import(&p.import_id)
        .ok_or_else(|| unknown_import(&p.import_id))?;
    let mut inserted = 0;
    let mut failed = Vec::new();
    for row in p.rows {
        let sql = match build_insert_sql(import.table(), import.columns(), &row) {
            Ok(sql) => sql,
            Err(error) => {
                if let Some(f) = import.record_failed(error.message, error.code) {
                    failed.push(f);
                }
                continue;
            }
        };
        match state.conn_mut().exec_update(&sql) {
            Ok(()) => {
                import.record_inserted();
                inserted += 1;
            }
            Err(error) => {
                if let Some(f) =
                    import.record_failed(format!("{error:#}"), error_codes::SQL_SYNTAX_ERROR)
                {
                    failed.push(f);
                }
            }
        }
    }
    state.insert_import(p.import_id, import);
    serde_json::to_value(ImportChunkResult { inserted, failed }).map_err(params_deserialize_error)
}

pub fn handle_data_import_commit(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: ImportCommitParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    let import = state
        .remove_import(&p.import_id)
        .ok_or_else(|| unknown_import(&p.import_id))?;
    serde_json::to_value(ImportCommitResult {
        inserted: import.inserted(),
        updated: 0,
        deleted: 0,
        failed: import.failed().to_vec(),
        elapsed_ms: Some(import.elapsed_ms()),
    })
    .map_err(params_deserialize_error)
}

pub fn handle_data_import_abort(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: ImportAbortParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    if state.remove_import(&p.import_id).is_none() {
        return Err(unknown_import(&p.import_id));
    }
    Ok(Value::Null)
}

pub fn handle_schema_databases(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let _: DatabasesParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    let databases = state
        .conn_mut()
        .list_databases()
        .map_err(|e| protocol_error_from_anyhow(error_codes::SQL_SYNTAX_ERROR, e))?;
    serde_json::to_value(databases).map_err(params_deserialize_error)
}

pub fn handle_schema_schemas(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: SchemasParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    serde_json::to_value(state.conn_mut().list_schemas(&p.database))
        .map_err(params_deserialize_error)
}

pub fn handle_schema_objects(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: ObjectsParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    if !p.kinds.is_empty()
        && !p
            .kinds
            .iter()
            .any(|kind| *kind == extension_protocol::schema::ObjectKind::Table)
    {
        return Ok(json!([]));
    }
    let objects = state
        .conn_mut()
        .list_devices(p.database.as_deref(), p.schema.as_deref())
        .map_err(|e| protocol_error_from_anyhow(error_codes::SQL_SYNTAX_ERROR, e))?;
    serde_json::to_value(objects).map_err(params_deserialize_error)
}

pub fn handle_schema_columns(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: ColumnsParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    let columns = state
        .conn_mut()
        .list_columns(&p.table)
        .map_err(|e| protocol_error_from_anyhow(error_codes::SQL_SYNTAX_ERROR, e))?;
    serde_json::to_value(columns).map_err(params_deserialize_error)
}

pub fn handle_schema_views(
    _state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let _: ViewsParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    Ok(json!([]))
}

pub fn handle_schema_indexes(
    _state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let _: IndexesParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    Ok(json!([]))
}

pub fn handle_schema_checks(
    _state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let _: ChecksParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    Ok(json!([]))
}

pub fn handle_schema_functions(
    _state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let _: FunctionsParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    Ok(json!([]))
}

pub fn handle_schema_object_view(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: ObjectViewParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    let view = match p.view.as_str() {
        "databases" => {
            let params = json!({ "conn_id": p.conn_id });
            let rows: Vec<DatabaseInfo> =
                serde_json::from_value(handle_schema_databases(state, &params)?)
                    .map_err(params_deserialize_error)?;
            ObjectView::new(
                "Databases",
                vec![object_view_column("name", "Name", Some(220.0), None)],
                rows.into_iter().map(|row| vec![row.name]).collect(),
            )
        }
        "schemas" => {
            let params =
                json!({ "conn_id": p.conn_id, "database": p.database.unwrap_or_default() });
            let rows: Vec<SchemaInfo> =
                serde_json::from_value(handle_schema_schemas(state, &params)?)
                    .map_err(params_deserialize_error)?;
            ObjectView::new(
                "Schemas",
                vec![object_view_column("name", "Name", Some(220.0), None)],
                rows.into_iter().map(|row| vec![row.name]).collect(),
            )
        }
        "tables" => object_view_from_objects(state, &p, "Tables", ObjectKind::Table)?,
        "views" => {
            let params =
                json!({ "conn_id": p.conn_id, "database": p.database, "schema": p.schema });
            let rows: Vec<ViewInfo> = serde_json::from_value(handle_schema_views(state, &params)?)
                .map_err(params_deserialize_error)?;
            ObjectView::new(
                "Views",
                vec![object_view_column("name", "Name", Some(220.0), None)],
                rows.into_iter().map(|row| vec![row.name]).collect(),
            )
        }
        "columns" => {
            let params = json!({
                "conn_id": p.conn_id,
                "database": p.database,
                "schema": p.schema,
                "table": p.table.unwrap_or_default(),
            });
            let rows: Vec<ColumnInfo> =
                serde_json::from_value(handle_schema_columns(state, &params)?)
                    .map_err(params_deserialize_error)?;
            ObjectView::new(
                "Columns",
                vec![
                    object_view_column("name", "Field", Some(220.0), None),
                    object_view_column("type", "Type", Some(160.0), None),
                    object_view_column("nullable", "Null?", Some(72.0), Some("right")),
                    object_view_column("comment", "Comment", Some(260.0), None),
                ],
                rows.into_iter()
                    .map(|row| {
                        vec![
                            row.name,
                            row.type_str,
                            row.nullable.to_string(),
                            row.comment,
                        ]
                    })
                    .collect(),
            )
        }
        "indexes" => {
            let params = json!({
                "conn_id": p.conn_id,
                "database": p.database,
                "schema": p.schema,
                "table": p.table.unwrap_or_default(),
            });
            let rows: Vec<IndexInfo> =
                serde_json::from_value(handle_schema_indexes(state, &params)?)
                    .map_err(params_deserialize_error)?;
            ObjectView::new(
                "Indexes",
                vec![
                    object_view_column("name", "Name", Some(220.0), None),
                    object_view_column("columns", "Columns", Some(220.0), None),
                    object_view_column("unique", "Unique?", Some(90.0), Some("right")),
                ],
                rows.into_iter()
                    .map(|row| vec![row.name, row.columns.join(", "), row.is_unique.to_string()])
                    .collect(),
            )
        }
        "functions" => {
            let params =
                json!({ "conn_id": p.conn_id, "database": p.database, "schema": p.schema });
            let rows: Vec<FunctionInfo> =
                serde_json::from_value(handle_schema_functions(state, &params)?)
                    .map_err(params_deserialize_error)?;
            ObjectView::new(
                "Functions",
                vec![
                    object_view_column("name", "Name", Some(220.0), None),
                    object_view_column("returns", "Returns", Some(160.0), None),
                    object_view_column("comment", "Comment", Some(260.0), None),
                ],
                rows.into_iter()
                    .map(|row| vec![row.name, row.return_type.unwrap_or_default(), row.comment])
                    .collect(),
            )
        }
        other => return Err(not_supported(format!("unsupported object view: {other}"))),
    };
    serde_json::to_value(view).map_err(params_deserialize_error)
}

fn object_view_from_objects(
    state: &mut ConnectionState,
    p: &ObjectViewParams,
    title: &str,
    kind: ObjectKind,
) -> Result<ObjectView, ProtocolError> {
    let params = json!({
        "conn_id": p.conn_id,
        "database": p.database,
        "schema": p.schema,
        "kinds": [kind.as_str()],
    });
    let rows: Vec<ObjectInfo> = serde_json::from_value(handle_schema_objects(state, &params)?)
        .map_err(params_deserialize_error)?;
    Ok(ObjectView::new(
        title,
        vec![
            object_view_column("name", "Name", Some(220.0), None),
            object_view_column("comment", "Comment", Some(260.0), None),
        ],
        rows.into_iter()
            .map(|row| vec![row.name, row.comment])
            .collect(),
    ))
}

#[derive(Debug, Clone, Deserialize)]
struct ObjectViewParams {
    conn_id: extension_protocol::conn::ConnId,
    view: String,
    #[serde(default)]
    database: Option<String>,
    #[serde(default)]
    schema: Option<String>,
    #[serde(default)]
    table: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ObjectView {
    title: String,
    columns: Vec<ObjectViewColumn>,
    rows: Vec<Vec<String>>,
}

impl ObjectView {
    fn new(title: &str, columns: Vec<ObjectViewColumn>, rows: Vec<Vec<String>>) -> Self {
        Self {
            title: title.to_string(),
            columns,
            rows,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct ObjectViewColumn {
    key: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    width_px: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    align: Option<String>,
}

fn object_view_column(
    key: &str,
    name: &str,
    width_px: Option<f64>,
    align: Option<&str>,
) -> ObjectViewColumn {
    ObjectViewColumn {
        key: key.to_string(),
        name: name.to_string(),
        width_px,
        align: align.map(str::to_string),
    }
}

pub fn declared_methods() -> &'static [&'static str] {
    &[
        method::PING,
        method::SHUTDOWN,
        method::CONN_TEST,
        method::CONN_OPEN,
        method::CONN_CLOSE,
        method::CONN_PING,
        method::CONN_USE,
        method::QUERY_START,
        method::CURSOR_FETCH,
        method::CURSOR_CLOSE,
        method::CURSOR_CANCEL,
        method::EXEC_RUN,
        method::EXEC_BATCH,
        method::DATA_EXPORT,
        method::DATA_IMPORT_BEGIN,
        method::DATA_IMPORT_CHUNK,
        method::DATA_IMPORT_COMMIT,
        method::DATA_IMPORT_ABORT,
        method::STREAM_READ,
        method::STREAM_CLOSE,
        SCHEMA_OBJECT_VIEW,
        method::SCHEMA_DATABASES,
        method::SCHEMA_SCHEMAS,
        method::SCHEMA_OBJECTS,
        method::SCHEMA_COLUMNS,
        method::SCHEMA_VIEWS,
        method::SCHEMA_INDEXES,
        method::SCHEMA_CHECKS,
        method::SCHEMA_FUNCTIONS,
        method::DDL_BUILD,
        method::DDL_BUILD_CREATE_TABLE,
        method::DDL_BUILD_ALTER_TABLE,
        method::DDL_BUILD_DROP,
    ]
}

pub fn ensure_driver(driver_id: &str) -> Result<(), ProtocolError> {
    if driver_id != "iotdb" {
        return Err(invalid_params(format!(
            "unsupported driver_id `{driver_id}` (this driver only handles `iotdb`)"
        )));
    }
    Ok(())
}

fn reject_params(params: &[extension_protocol::row::CellValue]) -> Result<(), ProtocolError> {
    if !params.is_empty() {
        return Err(not_supported(
            "IoTDB 0.0.7 Rust client does not expose parameterized SQL execution",
        ));
    }
    Ok(())
}

fn unknown_cursor(cursor_id: &str) -> ProtocolError {
    ProtocolError::new(
        error_codes::UNKNOWN_CURSOR_ID,
        format!("unknown cursor_id `{cursor_id}`"),
    )
}

fn unknown_stream(stream_id: &str) -> ProtocolError {
    ProtocolError::new(
        error_codes::RESOURCE_CLOSED,
        format!("unknown stream_id `{stream_id}`"),
    )
}

fn unknown_import(import_id: &str) -> ProtocolError {
    ProtocolError::new(
        error_codes::UNKNOWN_IMPORT_ID,
        format!("unknown import_id `{import_id}`"),
    )
}

fn export_sql(p: &ExportParams) -> Result<String, ProtocolError> {
    let sql = if let Some(sql) = p.sql.as_deref() {
        sql.to_string()
    } else if let Some(table) = p.table.as_deref() {
        let projection = if p.include_columns.is_empty() {
            "*".to_string()
        } else {
            p.include_columns.join(", ")
        };
        let mut sql = format!("SELECT {projection} FROM {table}");
        if let Some(where_clause) = p.where_clause.as_deref() {
            sql.push_str(" WHERE ");
            sql.push_str(where_clause);
        }
        sql
    } else {
        return Err(invalid_params("data/export requires `sql` or `table`"));
    };
    Ok(if let Some(max_rows) = p.max_rows {
        format!("{sql} LIMIT {max_rows}")
    } else {
        sql
    })
}

fn rows_to_json_objects(
    columns: &[extension_protocol::row::ColumnSpec],
    rows: &[Row],
) -> Vec<serde_json::Map<String, Value>> {
    rows.iter()
        .map(|row| {
            columns
                .iter()
                .zip(row.iter())
                .map(|(column, cell)| (column.name.clone(), cell_to_json(cell)))
                .collect()
        })
        .collect()
}

fn rows_to_ndjson(columns: &[extension_protocol::row::ColumnSpec], rows: &[Row]) -> String {
    rows_to_json_objects(columns, rows)
        .into_iter()
        .map(|object| serde_json::to_string(&object).unwrap_or_else(|_| "{}".to_string()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn rows_to_csv(columns: &[extension_protocol::row::ColumnSpec], rows: &[Row]) -> String {
    let mut lines = vec![
        columns
            .iter()
            .map(|column| csv_escape(&column.name))
            .collect::<Vec<_>>()
            .join(","),
    ];
    lines.extend(rows.iter().map(|row| {
        row.iter()
            .map(|cell| csv_escape(&cell_to_plain_string(cell)))
            .collect::<Vec<_>>()
            .join(",")
    }));
    lines.join("\n")
}

fn csv_escape(value: &str) -> String {
    if value.contains([',', '"', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn cell_to_json(cell: &CellValue) -> Value {
    match cell {
        CellValue::Null => Value::Null,
        CellValue::Bool { value } => json!(value),
        CellValue::I64 { value } => json!(value),
        CellValue::U64 { value } => json!(value),
        CellValue::F64 { value } => json!(value),
        CellValue::Decimal { value }
        | CellValue::Text { value }
        | CellValue::Bytes { value }
        | CellValue::Uuid { value }
        | CellValue::Date { value }
        | CellValue::Time { value }
        | CellValue::Datetime { value }
        | CellValue::Duration { value } => json!(value),
        CellValue::Json { value } => value.clone(),
        other => json!(format!("{other:?}")),
    }
}

fn cell_to_plain_string(cell: &CellValue) -> String {
    match cell_to_json(cell) {
        Value::Null => String::new(),
        Value::String(value) => value,
        other => other.to_string(),
    }
}

fn build_insert_sql(table: &str, columns: &[String], row: &Row) -> Result<String, ProtocolError> {
    let time_idx = columns
        .iter()
        .position(|column| column.eq_ignore_ascii_case("time"))
        .unwrap_or(0);
    let timestamp = row
        .get(time_idx)
        .ok_or_else(|| invalid_params("import row does not contain timestamp cell"))
        .and_then(cell_to_i64)?;
    let measurement_pairs = columns
        .iter()
        .enumerate()
        .filter(|(idx, column)| *idx != time_idx && !column.eq_ignore_ascii_case("time"))
        .filter_map(|(idx, column)| row.get(idx).map(|cell| (column, cell)))
        .filter(|(_, cell)| !matches!(cell, CellValue::Null))
        .collect::<Vec<_>>();
    if measurement_pairs.is_empty() {
        return Err(invalid_params(
            "import row does not contain measurement values",
        ));
    }
    let mut names = vec!["timestamp".to_string()];
    names.extend(measurement_pairs.iter().map(|(name, _)| (*name).clone()));
    let mut values = vec![timestamp.to_string()];
    values.extend(
        measurement_pairs
            .iter()
            .map(|(_, cell)| cell_to_sql_literal(cell)),
    );
    Ok(format!(
        "INSERT INTO {table}({}) VALUES ({})",
        names.join(","),
        values.join(",")
    ))
}

fn cell_to_i64(cell: &CellValue) -> Result<i64, ProtocolError> {
    match cell {
        CellValue::I64 { value } => Ok(*value),
        CellValue::U64 { value } => i64::try_from(*value)
            .map_err(|_| invalid_params("timestamp u64 is larger than i64 max")),
        CellValue::Text { value } => value
            .parse::<i64>()
            .map_err(|_| invalid_params("timestamp text is not an i64")),
        _ => Err(invalid_params("timestamp cell must be i64/u64/text")),
    }
}

fn cell_to_sql_literal(cell: &CellValue) -> String {
    match cell {
        CellValue::Null => "null".to_string(),
        CellValue::Bool { value } => value.to_string(),
        CellValue::I64 { value } => value.to_string(),
        CellValue::U64 { value } => value.to_string(),
        CellValue::F64 { value } => value.to_string(),
        CellValue::Decimal { value } | CellValue::Text { value } => {
            format!("'{}'", value.replace('\'', "''"))
        }
        CellValue::Json { value } => format!("'{}'", value.to_string().replace('\'', "''")),
        other => format!("'{}'", format!("{other:?}").replace('\'', "''")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_methods_include_supported_reference_surface() {
        for method_name in [
            method::DATA_EXPORT,
            method::DATA_IMPORT_BEGIN,
            method::DATA_IMPORT_CHUNK,
            method::DATA_IMPORT_COMMIT,
            method::DATA_IMPORT_ABORT,
            method::STREAM_READ,
            method::STREAM_CLOSE,
            method::SCHEMA_VIEWS,
            method::SCHEMA_INDEXES,
            method::SCHEMA_CHECKS,
            method::SCHEMA_FUNCTIONS,
            method::DDL_BUILD,
            method::DDL_BUILD_CREATE_TABLE,
            method::DDL_BUILD_ALTER_TABLE,
            method::DDL_BUILD_DROP,
        ] {
            assert!(
                declared_methods()
                    .iter()
                    .any(|declared| *declared == method_name),
                "missing declared method {method_name}"
            );
        }
    }

    #[test]
    fn declared_methods_omit_unsupported_transaction_surface() {
        for method_name in [method::TX_BEGIN, method::TX_COMMIT, method::TX_ROLLBACK] {
            assert!(
                !declared_methods()
                    .iter()
                    .any(|declared| *declared == method_name),
                "unsupported transaction method {method_name} must not be declared"
            );
        }
    }

    #[test]
    fn init_omits_transactions_feature() {
        let initialized = AtomicI64::new(0);
        let init = handle_init(
            &initialized,
            &serde_json::to_value(InitParams::new("onetcli-test", "iotdb-test"))
                .expect("init params should encode"),
        )
        .expect("init should succeed");
        let init: InitResult = serde_json::from_value(init).expect("init result should decode");

        assert!(!init.has_feature(Capability::TRANSACTIONS));
        for method_name in [method::TX_BEGIN, method::TX_COMMIT, method::TX_ROLLBACK] {
            assert!(
                !init.declares_method(method_name),
                "unsupported transaction method {method_name} must not be advertised"
            );
        }
    }

    #[test]
    fn transactional_exec_batch_returns_transaction_error() {
        let error = unsupported_transactional_batch_error();

        assert_eq!(error_codes::TX_NESTED_NOT_SUPPORTED, error.code);
        assert!(error.is_tx_error());
        assert!(error.message.contains("transactions"));
    }
}
