//! v2 server 的方法处理函数集合。
//!
//! 每个 handler 接收 `params: &Value`,返回 `Result<Value, ProtocolError>`。
//! 真正的业务逻辑(连接 DuckDB、跑 SQL)委托给 [`crate::duckdb_session`],
//! handlers 只负责参数解码 / 结果编码 / 错误码归类。

// `ProtocolError` 是 wire 契约类型,大小固定为 ~248 bytes(由 ErrorData
// 的多个 Option<String> 决定)。所有 handler 都返回 `Result<Value, ProtocolError>`
// 以与 dispatch 层签名对齐;无法改成 `Box<ProtocolError>`,否则 dispatch 层
// 还需多一次解装。这里全模块允许 result_large_err。
#![allow(clippy::result_large_err)]

use std::sync::atomic::{AtomicI64, Ordering};

use extension_protocol::conn::{
    ConnCloseParams, ConnId, ConnOpenParams, ConnPingParams, ConnTestParams, ConnUseParams,
};
use extension_protocol::data::{
    CsvOptions, DataFormat, ExportParams, ExportResult, StreamCloseParams, StreamReadParams,
    StreamReadResult,
};
use extension_protocol::ddl::{
    BuildAlterTableParams, BuildCreateTableParams, BuildDdlParams, BuildDropParams,
};
use extension_protocol::error::{ProtocolError, error_codes};
use extension_protocol::lifecycle::{Capability, InitParams, InitResult, ShutdownParams};
use extension_protocol::method;
use extension_protocol::query::{
    BatchError, CursorCancelParams, CursorCloseParams, CursorFetchParams, ExecBatchParams,
    ExecBatchResult, ExecRunParams, ExecRunResult, IsolationLevel, QueryStartParams, TxBeginParams,
    TxBeginResult, TxCommitParams, TxRollbackParams,
};
use extension_protocol::row::{CellValue, ColumnSpec, Row};
use extension_protocol::schema::{
    CheckInfo, ChecksParams, ColumnInfo, ColumnsParams, DatabaseInfo, DatabasesParams, FunctionArg,
    FunctionInfo, FunctionsParams, IndexInfo, IndexesParams, ObjectInfo, ObjectKind, ObjectsParams,
    SchemaInfo, SchemasParams, ViewInfo, ViewsParams,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::builtin_functions::DUCKDB_BUILTIN_FUNCTIONS;
use crate::duckdb_session::{
    DbConnectionConfig, DuckDbSession, current_database_name, is_default_catalog_reference,
    is_duckdb_catalog_schema,
};
use crate::server::{
    invalid_params, missing_param, params_deserialize_error, protocol_error_from_anyhow,
};
use crate::state::{ConnectionState, CursorState, ExportStreamFormat, ExportStreamState};
use crate::value::{cell_value_to_duckdb_value, map_column_type_kind, value_ref_to_cell};

const DEFAULT_CURSOR_FETCH_SIZE: u32 = 1_000;
const MAX_CURSOR_FETCH_SIZE: u32 = 10_000;
const DEFAULT_STREAM_READ_BYTES: usize = 64 * 1024;
const MAX_STREAM_READ_BYTES: usize = 1024 * 1024;
const EXPORT_FETCH_ROWS: u32 = 1_000;
pub const SCHEMA_OBJECT_VIEW: &str = "schema/object_view";

// ===================== Lifecycle =====================

/// 处理 `init` 请求,返回 InitResult + features。
///
/// `initialized` 是 server 共享的 once-flag;第二次 init 报 ALREADY_INITIALIZED。
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
        .with_feature(Capability::SERVER_CURSOR)
        .with_feature(Capability::BATCH_EXEC)
        .with_feature(Capability::TRANSACTIONS)
        .with_feature(Capability::DATA_PIPE)
        .with_driver("duckdb");
    for method_name in declared_methods() {
        result = result.with_method(*method_name);
    }
    serde_json::to_value(result).map_err(params_deserialize_error)
}

pub fn handle_shutdown(params: &Value) -> Result<Value, ProtocolError> {
    let _: ShutdownParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    // run_loop 在外层根据 method == SHUTDOWN 自行关闭 stream
    Ok(Value::Null)
}

// ===================== Connection =====================

pub fn handle_conn_test(params: &Value) -> Result<Value, ProtocolError> {
    let started = std::time::Instant::now();
    let p: ConnTestParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    if p.driver_id != "duckdb" {
        return Err(invalid_params(format!(
            "unsupported driver_id `{}` (this driver only handles `duckdb`)",
            p.driver_id
        )));
    }

    let cfg: DbConnectionConfig =
        serde_json::from_value(p.config.clone()).map_err(params_deserialize_error)?;
    let mut session = DuckDbSession::new();
    session
        .connect(cfg)
        .map_err(|e| protocol_error_from_anyhow(error_codes::IO_CONNECTION_REFUSED, e))?;
    session
        .ping()
        .map_err(|e| protocol_error_from_anyhow(error_codes::SERVER_CLOSED_CONNECTION, e))?;
    session.disconnect();

    Ok(serde_json::json!({
        "ok": true,
        "server_version": duckdb_version(),
        "warnings": [],
        "latency_ms": started.elapsed().as_millis() as u32,
    }))
}

pub fn handle_conn_open(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: ConnOpenParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    if p.driver_id != "duckdb" {
        return Err(invalid_params(format!(
            "unsupported driver_id `{}` (this driver only handles `duckdb`)",
            p.driver_id
        )));
    }

    let cfg: DbConnectionConfig =
        serde_json::from_value(p.config.clone()).map_err(params_deserialize_error)?;

    let mut session = DuckDbSession::new();
    session
        .connect(cfg)
        .map_err(|e| protocol_error_from_anyhow(error_codes::IO_CONNECTION_REFUSED, e))?;
    let conn_id = state.open_conn(session);

    Ok(serde_json::json!({
        "conn_id": conn_id,
        "server_info": {
            "version": duckdb_version(),
            "features": ["embedded", "single_file"],
        }
    }))
}

pub fn handle_conn_close(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: ConnCloseParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    if !state.close_conn(p.conn_id) {
        return Err(unknown_conn(p.conn_id));
    }
    Ok(Value::Null)
}

pub fn handle_conn_ping(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: ConnPingParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    let started = std::time::Instant::now();
    let session = state
        .get_conn(p.conn_id)
        .ok_or_else(|| unknown_conn(p.conn_id))?;
    session
        .ping()
        .map_err(|e| protocol_error_from_anyhow(error_codes::SERVER_CLOSED_CONNECTION, e))?;
    Ok(serde_json::json!({ "latency_ms": started.elapsed().as_millis() as u32 }))
}

pub fn handle_conn_use(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: ConnUseParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    let session = state
        .get_conn(p.conn_id)
        .ok_or_else(|| unknown_conn(p.conn_id))?;
    // DuckDB 单文件,只接受当前 catalog / main / 空;别的库切换报错。
    if let Some(db) = p.database.as_deref() {
        let current_catalog = current_duckdb_catalog(session)?;
        if !is_default_catalog_reference(db, &current_catalog) {
            return Err(ProtocolError::new(
                error_codes::SQL_OBJECT_NOT_FOUND,
                format!("DuckDB single-file connection only exposes database `{current_catalog}`"),
            ));
        }
    }
    // schema / role 无作用,直接 ack
    Ok(Value::Null)
}

// ===================== Query / Cursor =====================

pub fn handle_query_start(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: QueryStartParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    validate_tx_for_conn(state, p.conn_id, p.tx_id.as_deref())?;
    let session = state
        .get_conn(p.conn_id)
        .ok_or_else(|| unknown_conn(p.conn_id))?;
    let conn = session
        .connection()
        .map_err(|e| protocol_error_from_anyhow(error_codes::NOT_INITIALIZED, e))?;

    let (sql, columns) = prepare_query_columns_with_catalog_retry(conn, &p.sql, &p.params)?;
    let cursor_state = CursorState::new(
        p.conn_id,
        columns.clone(),
        sql,
        p.params,
        p.fetch_size,
        p.max_rows,
    );
    let cursor_id = state.open_cursor(cursor_state);

    Ok(serde_json::json!({
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
    let mut cursor = state
        .remove_cursor(&p.cursor_id)
        .ok_or_else(|| unknown_cursor(&p.cursor_id))?;
    let fetch_result = fetch_cursor_page(state, &mut cursor, p.n);
    state.insert_cursor(p.cursor_id, cursor);
    let (rows, done) = fetch_result?;
    Ok(serde_json::json!({
        "rows": rows,
        "done": done,
    }))
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
    if let Some(cursor) = state.get_cursor_mut(&p.cursor_id) {
        cursor.cancel();
        Ok(Value::Null)
    } else {
        Err(unknown_cursor(&p.cursor_id))
    }
}

pub fn handle_exec_run(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: ExecRunParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    validate_tx_for_conn(state, p.conn_id, p.tx_id.as_deref())?;
    let session = state
        .get_conn(p.conn_id)
        .ok_or_else(|| unknown_conn(p.conn_id))?;
    let conn = session
        .connection()
        .map_err(|e| protocol_error_from_anyhow(error_codes::NOT_INITIALIZED, e))?;
    let params = duckdb_params(&p.params)?;
    let affected = conn
        .execute(&p.sql, duckdb::params_from_iter(params.iter()))
        .map_err(duckdb_sql_error)?;
    Ok(serde_json::json!({ "affected_rows": affected as u64, "warnings": [] }))
}

pub fn handle_exec_batch(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: ExecBatchParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    let session = state
        .get_conn(p.conn_id)
        .ok_or_else(|| unknown_conn(p.conn_id))?;
    let conn = session
        .connection()
        .map_err(|e| protocol_error_from_anyhow(error_codes::NOT_INITIALIZED, e))?;

    if p.in_transaction {
        exec_tx_control(conn, "BEGIN")?;
    }
    let result = run_exec_batch(conn, &p.statements, p.stop_on_error);
    if p.in_transaction {
        let command = if result.errors.is_empty() {
            "COMMIT"
        } else {
            "ROLLBACK"
        };
        exec_tx_control(conn, command)?;
    }
    serde_json::to_value(result).map_err(params_deserialize_error)
}

fn run_exec_batch(
    conn: &duckdb::Connection,
    statements: &[String],
    stop_on_error: bool,
) -> ExecBatchResult {
    let mut results = Vec::new();
    let mut errors = Vec::new();
    for (index, sql) in statements.iter().enumerate() {
        match exec_statement(conn, sql) {
            Ok(result) => results.push(result),
            Err(error) => {
                results.push(ExecRunResult::default());
                errors.push(BatchError {
                    index: index as u32,
                    code: error.code,
                    message: error.message,
                });
                if stop_on_error {
                    break;
                }
            }
        }
    }
    ExecBatchResult { results, errors }
}

fn exec_statement(conn: &duckdb::Connection, sql: &str) -> Result<ExecRunResult, ProtocolError> {
    let affected = conn.execute(sql, []).map_err(duckdb_sql_error)?;
    Ok(ExecRunResult {
        affected_rows: affected as u64,
        last_insert_id: None,
        warnings: Vec::new(),
    })
}

fn exec_tx_control(conn: &duckdb::Connection, command: &str) -> Result<(), ProtocolError> {
    conn.execute(command, []).map(|_| ()).map_err(|error| {
        protocol_error_from_anyhow(
            error_codes::TX_ROLLBACK_REQUIRED,
            anyhow::Error::from(error),
        )
    })
}

// ===================== Transaction =====================

pub fn handle_tx_begin(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: TxBeginParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    validate_tx_begin_options(&p)?;
    if state.has_active_tx(p.conn_id) {
        return Err(ProtocolError::new(
            error_codes::TX_NESTED_NOT_SUPPORTED,
            "DuckDB driver supports one active transaction per connection",
        ));
    }
    {
        let session = state
            .get_conn(p.conn_id)
            .ok_or_else(|| unknown_conn(p.conn_id))?;
        let conn = session
            .connection()
            .map_err(|e| protocol_error_from_anyhow(error_codes::NOT_INITIALIZED, e))?;
        exec_tx_control(conn, "BEGIN TRANSACTION")?;
    }
    let tx_id = state.begin_tx(p.conn_id).ok_or_else(|| {
        ProtocolError::new(
            error_codes::TX_NESTED_NOT_SUPPORTED,
            "DuckDB driver supports one active transaction per connection",
        )
    })?;
    serde_json::to_value(TxBeginResult { tx_id }).map_err(params_deserialize_error)
}

pub fn handle_tx_commit(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: TxCommitParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    let conn_id = state
        .tx_conn(&p.tx_id)
        .ok_or_else(|| unknown_tx(&p.tx_id))?;
    {
        let session = state
            .get_conn(conn_id)
            .ok_or_else(|| unknown_conn(conn_id))?;
        let conn = session
            .connection()
            .map_err(|e| protocol_error_from_anyhow(error_codes::NOT_INITIALIZED, e))?;
        exec_tx_control(conn, "COMMIT")?;
    }
    state.close_tx(&p.tx_id);
    Ok(Value::Null)
}

pub fn handle_tx_rollback(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: TxRollbackParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    let conn_id = state
        .tx_conn(&p.tx_id)
        .ok_or_else(|| unknown_tx(&p.tx_id))?;
    if p.to_savepoint.is_some() {
        return Err(ProtocolError::new(
            error_codes::TX_NESTED_NOT_SUPPORTED,
            "DuckDB driver does not support savepoints",
        ));
    }
    {
        let session = state
            .get_conn(conn_id)
            .ok_or_else(|| unknown_conn(conn_id))?;
        let conn = session
            .connection()
            .map_err(|e| protocol_error_from_anyhow(error_codes::NOT_INITIALIZED, e))?;
        exec_tx_control(conn, "ROLLBACK")?;
    }
    state.close_tx(&p.tx_id);
    Ok(Value::Null)
}

fn validate_tx_begin_options(params: &TxBeginParams) -> Result<(), ProtocolError> {
    if params.read_only || params.deferrable.unwrap_or(false) {
        return Err(ProtocolError::new(
            error_codes::TX_ISOLATION_NOT_SUPPORTED,
            "DuckDB driver does not support read_only or deferrable transaction options",
        ));
    }
    match params.isolation {
        None | Some(IsolationLevel::Default) => Ok(()),
        Some(_) => Err(ProtocolError::new(
            error_codes::TX_ISOLATION_NOT_SUPPORTED,
            "DuckDB driver does not support custom transaction isolation levels",
        )),
    }
}

fn validate_tx_for_conn(
    state: &ConnectionState,
    conn_id: ConnId,
    tx_id: Option<&str>,
) -> Result<(), ProtocolError> {
    match tx_id {
        Some(tx_id) if !state.tx_matches_conn(tx_id, conn_id) => Err(unknown_tx(tx_id)),
        _ => Ok(()),
    }
}

// ===================== Data / Stream =====================

pub fn handle_data_export(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: ExportParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    if p.stream_id.trim().is_empty() {
        return Err(missing_param("stream_id"));
    }
    let session = state
        .get_conn(p.conn_id)
        .ok_or_else(|| unknown_conn(p.conn_id))?;
    let conn = session
        .connection()
        .map_err(|e| protocol_error_from_anyhow(error_codes::NOT_INITIALIZED, e))?;
    let (sql, columns) = build_export_sql(conn, &p)?;
    let format = export_stream_format(p.format, &p.options)?;
    state.open_stream(
        p.stream_id,
        ExportStreamState::new(p.conn_id, sql, columns.clone(), format, p.max_rows),
    );
    serde_json::to_value(ExportResult {
        estimated_bytes: None,
        estimated_rows: p.max_rows,
        metadata: serde_json::json!({
            "columns": columns,
            "format": p.format,
        }),
    })
    .map_err(params_deserialize_error)
}

pub fn handle_stream_read(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: StreamReadParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    let max_bytes = stream_read_limit(p.max_bytes);
    ensure_stream_buffered(state, &p.stream_id)?;
    let stream = state
        .get_stream_mut(&p.stream_id)
        .ok_or_else(|| unknown_stream(&p.stream_id))?;
    let bytes = stream.drain_pending(max_bytes);
    let done = stream.is_done() && stream.pending().is_empty();
    use base64::Engine;
    serde_json::to_value(StreamReadResult {
        data: base64::engine::general_purpose::STANDARD.encode(bytes),
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
    crate::data_import::handle_data_import_begin(state, params)
}

pub fn handle_data_import_chunk(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    crate::data_import::handle_data_import_chunk(state, params)
}

pub fn handle_data_import_commit(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    crate::data_import::handle_data_import_commit(state, params)
}

pub fn handle_data_import_abort(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    crate::data_import::handle_data_import_abort(state, params)
}

fn ensure_stream_buffered(
    state: &mut ConnectionState,
    stream_id: &str,
) -> Result<(), ProtocolError> {
    let Some(stream) = state.get_stream(stream_id) else {
        return Err(unknown_stream(stream_id));
    };
    if !stream.pending().is_empty() || stream.is_done() {
        return Ok(());
    }
    fill_stream_buffer(state, stream_id)
}

fn fill_stream_buffer(state: &mut ConnectionState, stream_id: &str) -> Result<(), ProtocolError> {
    let (conn_id, sql, offset, limit) = stream_fetch_plan(state, stream_id)?;
    let session = state
        .get_conn(conn_id)
        .ok_or_else(|| unknown_conn(conn_id))?;
    let conn = session
        .connection()
        .map_err(|e| protocol_error_from_anyhow(error_codes::NOT_INITIALIZED, e))?;
    let rows = run_query_page(conn, &sql, &[], limit, offset)?;
    let fetched = rows.len();
    let stream = state
        .get_stream_mut(stream_id)
        .ok_or_else(|| unknown_stream(stream_id))?;
    let columns = stream.columns().to_vec();
    let bytes = render_export_rows(stream.format_mut(), &columns, rows)?;
    stream.append_pending(bytes);
    stream.advance(fetched, limit);
    Ok(())
}

fn stream_fetch_plan(
    state: &ConnectionState,
    stream_id: &str,
) -> Result<(ConnId, String, u64, u32), ProtocolError> {
    let stream = state
        .get_stream(stream_id)
        .ok_or_else(|| unknown_stream(stream_id))?;
    let limit = match stream.remaining_max_rows() {
        Some(0) => 0,
        Some(remaining) => EXPORT_FETCH_ROWS.min(remaining.min(u32::MAX as u64) as u32),
        None => EXPORT_FETCH_ROWS,
    };
    Ok((
        stream.conn_id(),
        stream.sql().to_string(),
        stream.offset(),
        limit,
    ))
}

fn stream_read_limit(max_bytes: Option<u32>) -> usize {
    max_bytes
        .map(|value| value.max(1) as usize)
        .unwrap_or(DEFAULT_STREAM_READ_BYTES)
        .min(MAX_STREAM_READ_BYTES)
}

fn export_stream_format(
    format: DataFormat,
    options: &Value,
) -> Result<ExportStreamFormat, ProtocolError> {
    match format {
        DataFormat::Csv => Ok(ExportStreamFormat::Csv {
            options: parse_csv_options(options)?,
            header_written: false,
        }),
        DataFormat::Ndjson => Ok(ExportStreamFormat::Ndjson),
        other => Err(invalid_params(format!(
            "DuckDB data/export does not support format `{other:?}` yet"
        ))),
    }
}

fn parse_csv_options(options: &Value) -> Result<CsvOptions, ProtocolError> {
    let options: CsvOptions = if options.is_null() {
        CsvOptions::default()
    } else {
        serde_json::from_value(options.clone()).map_err(params_deserialize_error)?
    };
    if options.delimiter.is_empty() || options.quote.is_empty() {
        return Err(invalid_params("CSV delimiter and quote must not be empty"));
    }
    if !options.encoding.eq_ignore_ascii_case("utf-8") {
        return Err(invalid_params("DuckDB data/export only supports utf-8 CSV"));
    }
    Ok(options)
}

fn build_export_sql(
    conn: &duckdb::Connection,
    params: &ExportParams,
) -> Result<(String, Vec<String>), ProtocolError> {
    let current_catalog = current_duckdb_catalog_from_connection(conn)?;
    validate_export_database(params.database.as_deref(), &current_catalog)?;
    let raw_sql = raw_export_sql(params)?;
    let raw_columns: Vec<String> = prepare_query_columns(conn, &raw_sql, &[])?
        .into_iter()
        .map(|column| column.name)
        .collect();
    let columns = selected_export_columns(&raw_columns, params)?;
    let sql = apply_export_projection(&raw_sql, &columns);
    Ok((sql, columns))
}

fn validate_export_database(
    database: Option<&str>,
    current_catalog: &str,
) -> Result<(), ProtocolError> {
    match database {
        None => Ok(()),
        Some(database) if is_default_catalog_reference(database, current_catalog) => Ok(()),
        Some(database) => Err(invalid_params(format!(
            "DuckDB data/export only supports database `{current_catalog}`, got `{database}`"
        ))),
    }
}

fn current_duckdb_catalog(session: &DuckDbSession) -> Result<String, ProtocolError> {
    session
        .current_catalog()
        .map_err(|e| protocol_error_from_anyhow(error_codes::INTERNAL_ERROR, e))
}

fn current_duckdb_catalog_from_connection(
    conn: &duckdb::Connection,
) -> Result<String, ProtocolError> {
    current_database_name(conn).map_err(duckdb_sql_error)
}

fn append_optional_filter(
    sql: &mut String,
    values: &mut Vec<String>,
    column: &str,
    value: Option<String>,
) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        sql.push_str(" AND ");
        sql.push_str(column);
        sql.push_str(" = ?");
        values.push(value);
    }
}

fn append_duckdb_database_filter(
    sql: &mut String,
    values: &mut Vec<String>,
    value: Option<String>,
    column: &str,
    current_catalog: &str,
) {
    if let Some(value) = value.filter(|value| !is_default_catalog_reference(value, current_catalog))
    {
        append_optional_filter(sql, values, column, Some(value));
    }
}

fn append_database_schema_filters(
    sql: &mut String,
    values: &mut Vec<String>,
    database: Option<String>,
    schema: Option<String>,
    database_column: &str,
    schema_column: &str,
    current_catalog: &str,
) {
    append_duckdb_database_filter(sql, values, database, database_column, current_catalog);
    append_optional_filter(sql, values, schema_column, schema);
}

fn should_include_internal_schema_objects(schema: Option<&str>) -> bool {
    schema.is_some_and(is_duckdb_catalog_schema)
}

fn raw_export_sql(params: &ExportParams) -> Result<String, ProtocolError> {
    match (params.sql.as_deref(), params.table.as_deref()) {
        (Some(_), Some(_)) => Err(invalid_params("data/export accepts either sql or table")),
        (Some(sql), None) if !sql.trim().is_empty() => Ok(trimmed_query_sql(sql).to_string()),
        (None, Some(table)) if !table.trim().is_empty() => Ok(table_export_sql(params, table)),
        _ => Err(invalid_params("data/export requires sql or table")),
    }
}

fn table_export_sql(params: &ExportParams, table: &str) -> String {
    let table_name = match params.schema.as_deref().filter(|schema| !schema.is_empty()) {
        Some(schema) => format!(
            "{}.{}",
            quote_sql_identifier(schema),
            quote_sql_identifier(table)
        ),
        None => quote_sql_identifier(table),
    };
    let mut sql = format!("SELECT * FROM {table_name}");
    if let Some(where_clause) = params
        .where_clause
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        sql.push_str(" WHERE ");
        sql.push_str(where_clause);
    }
    sql
}

fn selected_export_columns(
    raw_columns: &[String],
    params: &ExportParams,
) -> Result<Vec<String>, ProtocolError> {
    let mut columns = if params.include_columns.is_empty() {
        raw_columns.to_vec()
    } else {
        params.include_columns.clone()
    };
    columns.retain(|column| {
        !params
            .exclude_columns
            .iter()
            .any(|excluded| excluded == column)
    });
    if columns.is_empty() {
        return Err(invalid_params("data/export selected no columns"));
    }
    for column in &columns {
        if !raw_columns.iter().any(|raw| raw == column) {
            return Err(invalid_params(format!("unknown export column `{column}`")));
        }
    }
    Ok(columns)
}

fn apply_export_projection(raw_sql: &str, columns: &[String]) -> String {
    let projection = columns
        .iter()
        .map(|column| quote_sql_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    format!("SELECT {projection} FROM ({raw_sql}) AS __onetcli_export_source")
}

fn quote_sql_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn render_export_rows(
    format: &mut ExportStreamFormat,
    columns: &[String],
    rows: Vec<Row>,
) -> Result<Vec<u8>, ProtocolError> {
    match format {
        ExportStreamFormat::Csv {
            options,
            header_written,
        } => Ok(render_csv_rows(options, header_written, columns, rows)),
        ExportStreamFormat::Ndjson => render_ndjson_rows(columns, rows),
    }
}

fn render_csv_rows(
    options: &CsvOptions,
    header_written: &mut bool,
    columns: &[String],
    rows: Vec<Row>,
) -> Vec<u8> {
    let mut out = String::new();
    if options.header && !*header_written {
        out.push_str(&csv_line(
            columns.iter().map(|value| value.as_str()),
            options,
        ));
        *header_written = true;
    }
    for row in rows {
        let values = row
            .into_iter()
            .map(|cell| cell_to_csv_string(cell, options))
            .collect::<Vec<_>>();
        out.push_str(&csv_line(
            values.iter().map(|value| value.as_str()),
            options,
        ));
    }
    out.into_bytes()
}

fn csv_line<'a>(values: impl Iterator<Item = &'a str>, options: &CsvOptions) -> String {
    let mut line = values
        .map(|value| csv_escape(value, options))
        .collect::<Vec<_>>()
        .join(&options.delimiter);
    line.push('\n');
    line
}

fn csv_escape(value: &str, options: &CsvOptions) -> String {
    let needs_quote = value.contains(&options.delimiter)
        || value.contains('\n')
        || value.contains('\r')
        || value.contains(&options.quote);
    if !needs_quote {
        return value.to_string();
    }
    let escaped = value.replace(
        &options.quote,
        &format!("{}{}", options.quote, options.quote),
    );
    format!("{}{}{}", options.quote, escaped, options.quote)
}

fn cell_to_csv_string(cell: CellValue, options: &CsvOptions) -> String {
    match cell_to_json_value(cell) {
        Value::Null => options
            .null_string
            .clone()
            .unwrap_or_else(|| "\\N".to_string()),
        Value::String(value) => value,
        other => other.to_string(),
    }
}

fn render_ndjson_rows(columns: &[String], rows: Vec<Row>) -> Result<Vec<u8>, ProtocolError> {
    let mut out = Vec::new();
    for row in rows {
        let mut obj = serde_json::Map::new();
        for (column, cell) in columns.iter().zip(row) {
            obj.insert(column.clone(), cell_to_json_value(cell));
        }
        serde_json::to_writer(&mut out, &Value::Object(obj)).map_err(params_deserialize_error)?;
        out.push(b'\n');
    }
    Ok(out)
}

fn cell_to_json_value(cell: CellValue) -> Value {
    match cell {
        CellValue::Null => Value::Null,
        CellValue::Bool { value } => Value::Bool(value),
        CellValue::I64 { value } => serde_json::json!(value),
        CellValue::U64 { value } => serde_json::json!(value),
        CellValue::F64 { value } => serde_json::json!(value),
        CellValue::Json { value } => value,
        CellValue::Map { value } => Value::Object(value),
        CellValue::Array { value, .. } => serde_json::json!(value),
        CellValue::Decimal { value }
        | CellValue::Text { value }
        | CellValue::Uuid { value }
        | CellValue::Date { value }
        | CellValue::Time { value }
        | CellValue::Datetime { value }
        | CellValue::Duration { value }
        | CellValue::Bytes { value }
        | CellValue::Geo { value, .. } => Value::String(value),
        CellValue::Custom { subtype, raw } => Value::String(format!("custom:{subtype}({raw})")),
    }
}

// ===================== Schema =====================

pub fn handle_schema_databases(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: DatabasesParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    let session = state
        .get_conn(p.conn_id)
        .ok_or_else(|| unknown_conn(p.conn_id))?;
    let current_catalog = current_duckdb_catalog(session)?;
    // DuckDB 单文件 = 单 database。
    let info = DatabaseInfo {
        name: current_catalog,
        ..Default::default()
    };
    Ok(serde_json::json!([info]))
}

pub fn handle_schema_schemas(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: SchemasParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    let session = state
        .get_conn(p.conn_id)
        .ok_or_else(|| unknown_conn(p.conn_id))?;
    let conn = session
        .connection()
        .map_err(|e| protocol_error_from_anyhow(error_codes::NOT_INITIALIZED, e))?;
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT schema_name FROM information_schema.schemata ORDER BY schema_name",
        )
        .map_err(|e| {
            protocol_error_from_anyhow(error_codes::SQL_SYNTAX_ERROR, anyhow::Error::from(e))
        })?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| {
            protocol_error_from_anyhow(error_codes::INTERNAL_ERROR, anyhow::Error::from(e))
        })?;
    let mut out: Vec<SchemaInfo> = Vec::new();
    for r in rows {
        let name = r.map_err(|e| {
            protocol_error_from_anyhow(error_codes::INTERNAL_ERROR, anyhow::Error::from(e))
        })?;
        out.push(SchemaInfo {
            name,
            ..Default::default()
        });
    }
    serde_json::to_value(out).map_err(params_deserialize_error)
}

pub fn handle_schema_objects(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: ObjectsParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    let session = state
        .get_conn(p.conn_id)
        .ok_or_else(|| unknown_conn(p.conn_id))?;
    let conn = session
        .connection()
        .map_err(|e| protocol_error_from_anyhow(error_codes::NOT_INITIALIZED, e))?;
    let current_catalog = current_duckdb_catalog(session)?;

    let mut objects = Vec::new();

    // tables
    if p.kinds.is_empty() || p.kinds.contains(&ObjectKind::Table) {
        let include_internal = should_include_internal_schema_objects(p.schema.as_deref());
        let mut sql = if include_internal {
            String::from(
                "SELECT view_name AS table_name, schema_name FROM duckdb_views() WHERE 1=1",
            )
        } else {
            String::from("SELECT table_name, schema_name FROM duckdb_tables() WHERE 1=1")
        };
        if !include_internal {
            sql.push_str(" AND internal = FALSE AND temporary = FALSE");
        }
        let mut values = Vec::new();
        append_database_schema_filters(
            &mut sql,
            &mut values,
            p.database.clone(),
            p.schema.clone(),
            "database_name",
            "schema_name",
            &current_catalog,
        );
        sql.push_str(" ORDER BY schema_name, table_name");

        let mut stmt = conn.prepare(&sql).map_err(|e| {
            protocol_error_from_anyhow(error_codes::INTERNAL_ERROR, anyhow::Error::from(e))
        })?;
        let rows = stmt
            .query_map(duckdb::params_from_iter(values.iter()), |row| {
                let name: String = row.get(0)?;
                Ok(ObjectInfo {
                    name,
                    kind: ObjectKind::Table,
                    comment: String::new(),
                    row_count_estimate: None,
                    size_bytes: None,
                    created_at: None,
                    updated_at: None,
                    extra: Value::Null,
                })
            })
            .map_err(|e| {
                protocol_error_from_anyhow(error_codes::INTERNAL_ERROR, anyhow::Error::from(e))
            })?;
        for r in rows {
            objects.push(r.map_err(|e| {
                protocol_error_from_anyhow(error_codes::INTERNAL_ERROR, anyhow::Error::from(e))
            })?);
        }
    }

    // views
    let catalog_views_returned_as_tables =
        should_include_internal_schema_objects(p.schema.as_deref())
            && (p.kinds.is_empty() || p.kinds.contains(&ObjectKind::Table));
    if (p.kinds.is_empty() || p.kinds.contains(&ObjectKind::View))
        && !catalog_views_returned_as_tables
    {
        let include_internal = should_include_internal_schema_objects(p.schema.as_deref());
        let mut sql = String::from("SELECT view_name FROM duckdb_views() WHERE 1=1");
        if !include_internal {
            sql.push_str(" AND internal = FALSE AND temporary = FALSE");
        }
        let mut values = Vec::new();
        append_database_schema_filters(
            &mut sql,
            &mut values,
            p.database.clone(),
            p.schema.clone(),
            "database_name",
            "schema_name",
            &current_catalog,
        );
        sql.push_str(" ORDER BY schema_name, view_name");

        let mut stmt = conn.prepare(&sql).map_err(|e| {
            protocol_error_from_anyhow(error_codes::INTERNAL_ERROR, anyhow::Error::from(e))
        })?;
        let rows = stmt
            .query_map(duckdb::params_from_iter(values.iter()), |row| {
                let name: String = row.get(0)?;
                Ok(ObjectInfo {
                    name,
                    kind: ObjectKind::View,
                    comment: String::new(),
                    row_count_estimate: None,
                    size_bytes: None,
                    created_at: None,
                    updated_at: None,
                    extra: Value::Null,
                })
            })
            .map_err(|e| {
                protocol_error_from_anyhow(error_codes::INTERNAL_ERROR, anyhow::Error::from(e))
            })?;
        for r in rows {
            objects.push(r.map_err(|e| {
                protocol_error_from_anyhow(error_codes::INTERNAL_ERROR, anyhow::Error::from(e))
            })?);
        }
    }

    serde_json::to_value(objects).map_err(params_deserialize_error)
}

pub fn handle_schema_columns(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: ColumnsParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    if p.table.trim().is_empty() {
        return Err(missing_param("table"));
    }
    let session = state
        .get_conn(p.conn_id)
        .ok_or_else(|| unknown_conn(p.conn_id))?;
    let conn = session
        .connection()
        .map_err(|e| protocol_error_from_anyhow(error_codes::NOT_INITIALIZED, e))?;
    let current_catalog = current_duckdb_catalog(session)?;
    let include_internal = should_include_internal_schema_objects(p.schema.as_deref());
    let mut sql = String::from(
        "SELECT column_index, column_name, data_type, is_nullable, column_default \
         FROM duckdb_columns() WHERE table_name = ?",
    );
    if !include_internal {
        sql.push_str(" AND internal = FALSE");
    }
    let mut values = vec![p.table.clone()];
    append_database_schema_filters(
        &mut sql,
        &mut values,
        p.database,
        p.schema,
        "database_name",
        "schema_name",
        &current_catalog,
    );
    sql.push_str(" ORDER BY column_index");

    let mut stmt = conn.prepare(&sql).map_err(|e| {
        protocol_error_from_anyhow(error_codes::INTERNAL_ERROR, anyhow::Error::from(e))
    })?;
    let rows = stmt
        .query_map(duckdb::params_from_iter(values.iter()), |row| {
            let ordinal: i64 = row.get(0)?;
            let name: String = row.get(1)?;
            let type_str: String = row.get(2)?;
            let nullable: bool = row.get(3)?;
            let default: Option<String> = row.get(4)?;
            Ok(ColumnInfo {
                ordinal: ordinal as u32,
                name,
                raw_type: Some(type_str.clone()),
                type_str,
                nullable,
                default,
                ..Default::default()
            })
        })
        .map_err(|e| {
            protocol_error_from_anyhow(error_codes::INTERNAL_ERROR, anyhow::Error::from(e))
        })?;

    let mut out: Vec<ColumnInfo> = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| {
            protocol_error_from_anyhow(error_codes::INTERNAL_ERROR, anyhow::Error::from(e))
        })?);
    }
    serde_json::to_value(out).map_err(params_deserialize_error)
}

pub fn handle_schema_views(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: ViewsParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    let session = state
        .get_conn(p.conn_id)
        .ok_or_else(|| unknown_conn(p.conn_id))?;
    let conn = session
        .connection()
        .map_err(|e| protocol_error_from_anyhow(error_codes::NOT_INITIALIZED, e))?;
    let current_catalog = current_duckdb_catalog(session)?;
    let include_internal = should_include_internal_schema_objects(p.schema.as_deref());
    let mut sql = String::from(
        "SELECT view_name, sql FROM duckdb_views() \
         WHERE 1=1",
    );
    if !include_internal {
        sql.push_str(" AND internal = FALSE AND temporary = FALSE");
    }
    let mut values = Vec::new();
    append_database_schema_filters(
        &mut sql,
        &mut values,
        p.database,
        p.schema,
        "database_name",
        "schema_name",
        &current_catalog,
    );
    sql.push_str(" ORDER BY schema_name, view_name");

    let mut stmt = conn.prepare(&sql).map_err(|e| {
        protocol_error_from_anyhow(error_codes::INTERNAL_ERROR, anyhow::Error::from(e))
    })?;
    let rows = stmt
        .query_map(duckdb::params_from_iter(values.iter()), |row| {
            let name: String = row.get(0)?;
            let sql: Option<String> = row.get(1)?;
            Ok(ViewInfo {
                name,
                kind: ObjectKind::View,
                definition_sql: sql.unwrap_or_default(),
                comment: String::new(),
                extra: Value::Null,
            })
        })
        .map_err(|e| {
            protocol_error_from_anyhow(error_codes::INTERNAL_ERROR, anyhow::Error::from(e))
        })?;
    let mut out: Vec<ViewInfo> = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| {
            protocol_error_from_anyhow(error_codes::INTERNAL_ERROR, anyhow::Error::from(e))
        })?);
    }
    serde_json::to_value(out).map_err(params_deserialize_error)
}

pub fn handle_schema_indexes(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: IndexesParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    if p.table.trim().is_empty() {
        return Err(missing_param("table"));
    }
    let session = state
        .get_conn(p.conn_id)
        .ok_or_else(|| unknown_conn(p.conn_id))?;
    let conn = session
        .connection()
        .map_err(|e| protocol_error_from_anyhow(error_codes::NOT_INITIALIZED, e))?;
    let current_catalog = current_duckdb_catalog(session)?;
    let mut sql = String::from(
        "SELECT index_name, table_name, is_unique FROM duckdb_indexes() \
         WHERE table_name = ?",
    );
    let mut values = vec![p.table.clone()];
    append_database_schema_filters(
        &mut sql,
        &mut values,
        p.database,
        p.schema,
        "database_name",
        "schema_name",
        &current_catalog,
    );
    sql.push_str(" ORDER BY index_name");

    let mut stmt = conn.prepare(&sql).map_err(|e| {
        protocol_error_from_anyhow(error_codes::INTERNAL_ERROR, anyhow::Error::from(e))
    })?;
    let rows = stmt
        .query_map(duckdb::params_from_iter(values.iter()), |row| {
            let name: String = row.get(0)?;
            let table: String = row.get(1)?;
            let is_unique: bool = row.get(2)?;
            Ok(IndexInfo {
                name,
                table,
                columns: vec![],
                kind: Some("art".to_string()),
                is_unique,
                is_primary: false,
                where_clause: None,
                comment: String::new(),
                extra: Value::Null,
            })
        })
        .map_err(|e| {
            protocol_error_from_anyhow(error_codes::INTERNAL_ERROR, anyhow::Error::from(e))
        })?;
    let mut out: Vec<IndexInfo> = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| {
            protocol_error_from_anyhow(error_codes::INTERNAL_ERROR, anyhow::Error::from(e))
        })?);
    }
    serde_json::to_value(out).map_err(params_deserialize_error)
}

pub fn handle_schema_object_view(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: ObjectViewParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    let view = match p.view.as_str() {
        "databases" => {
            let params = serde_json::json!({ "conn_id": p.conn_id });
            let rows: Vec<DatabaseInfo> =
                serde_json::from_value(handle_schema_databases(state, &params)?)
                    .map_err(params_deserialize_error)?;
            ObjectView::new(
                "Databases",
                vec![
                    object_view_column("name", "Name", Some(220.0), None),
                    object_view_column("comment", "Comment", Some(260.0), None),
                ],
                rows.into_iter()
                    .map(|row| vec![row.name, row.comment])
                    .collect(),
            )
        }
        "schemas" => {
            let params = serde_json::json!({ "conn_id": p.conn_id, "database": p.database.unwrap_or_default() });
            let rows: Vec<SchemaInfo> =
                serde_json::from_value(handle_schema_schemas(state, &params)?)
                    .map_err(params_deserialize_error)?;
            ObjectView::new(
                "Schemas",
                vec![
                    object_view_column("name", "Name", Some(220.0), None),
                    object_view_column("owner", "Owner", Some(160.0), None),
                    object_view_column("comment", "Comment", Some(260.0), None),
                ],
                rows.into_iter()
                    .map(|row| vec![row.name, row.owner.unwrap_or_default(), row.comment])
                    .collect(),
            )
        }
        "tables" => object_view_from_objects(state, &p, "Tables", ObjectKind::Table)?,
        "views" => {
            let params = serde_json::json!({ "conn_id": p.conn_id, "database": p.database, "schema": p.schema });
            let rows: Vec<ViewInfo> = serde_json::from_value(handle_schema_views(state, &params)?)
                .map_err(params_deserialize_error)?;
            ObjectView::new(
                "Views",
                vec![
                    object_view_column("name", "Name", Some(220.0), None),
                    object_view_column("kind", "Kind", Some(140.0), None),
                    object_view_column("comment", "Comment", Some(260.0), None),
                ],
                rows.into_iter()
                    .map(|row| vec![row.name, row.kind.as_str().to_string(), row.comment])
                    .collect(),
            )
        }
        "columns" => {
            let table = p.table.clone().unwrap_or_default();
            let params = serde_json::json!({
                "conn_id": p.conn_id,
                "database": p.database,
                "schema": p.schema,
                "table": table,
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
                    object_view_column("default", "Default", Some(180.0), None),
                    object_view_column("comment", "Comment", Some(260.0), None),
                ],
                rows.into_iter()
                    .map(|row| {
                        vec![
                            row.name,
                            row.type_str,
                            row.nullable.to_string(),
                            row.default.unwrap_or_default(),
                            row.comment,
                        ]
                    })
                    .collect(),
            )
        }
        "indexes" => {
            let table = p.table.clone().unwrap_or_default();
            let params = serde_json::json!({
                "conn_id": p.conn_id,
                "database": p.database,
                "schema": p.schema,
                "table": table,
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
                    object_view_column("primary", "Primary?", Some(90.0), Some("right")),
                    object_view_column("type", "Type", Some(140.0), None),
                ],
                rows.into_iter()
                    .map(|row| {
                        vec![
                            row.name,
                            row.columns.join(", "),
                            row.is_unique.to_string(),
                            row.is_primary.to_string(),
                            row.kind.unwrap_or_default(),
                        ]
                    })
                    .collect(),
            )
        }
        "functions" => {
            let params = serde_json::json!({ "conn_id": p.conn_id, "database": p.database, "schema": p.schema });
            let rows: Vec<FunctionInfo> =
                serde_json::from_value(handle_schema_functions(state, &params)?)
                    .map_err(params_deserialize_error)?;
            ObjectView::new(
                "Functions",
                vec![
                    object_view_column("name", "Name", Some(220.0), None),
                    object_view_column("returns", "Returns", Some(160.0), None),
                    object_view_column("language", "Language", Some(120.0), None),
                    object_view_column("comment", "Comment", Some(260.0), None),
                ],
                rows.into_iter()
                    .map(|row| {
                        vec![
                            row.name,
                            row.return_type.unwrap_or_default(),
                            row.language.unwrap_or_default(),
                            row.comment,
                        ]
                    })
                    .collect(),
            )
        }
        other => {
            return Err(ProtocolError::new(
                error_codes::METHOD_NOT_FOUND,
                format!("unsupported object view: {other}"),
            ));
        }
    };
    serde_json::to_value(view).map_err(params_deserialize_error)
}

fn object_view_from_objects(
    state: &mut ConnectionState,
    p: &ObjectViewParams,
    title: &str,
    kind: ObjectKind,
) -> Result<ObjectView, ProtocolError> {
    let params = serde_json::json!({
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
    conn_id: ConnId,
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

pub fn handle_schema_checks(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: ChecksParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    if p.table.trim().is_empty() {
        return Err(missing_param("table"));
    }
    let session = state
        .get_conn(p.conn_id)
        .ok_or_else(|| unknown_conn(p.conn_id))?;
    let conn = session
        .connection()
        .map_err(|e| protocol_error_from_anyhow(error_codes::NOT_INITIALIZED, e))?;

    let mut sql = String::from(
        "SELECT tc.constraint_name, tc.table_name, cc.check_clause \
         FROM information_schema.table_constraints AS tc \
         JOIN information_schema.check_constraints AS cc \
           ON tc.constraint_catalog = cc.constraint_catalog \
          AND tc.constraint_schema = cc.constraint_schema \
          AND tc.constraint_name = cc.constraint_name \
         WHERE tc.constraint_type = 'CHECK' AND tc.table_name = ?",
    );
    let mut values = vec![p.table.clone()];
    if let Some(schema) = p.schema.filter(|schema| !schema.trim().is_empty()) {
        sql.push_str(" AND tc.table_schema = ?");
        values.push(schema);
    }
    sql.push_str(" ORDER BY tc.constraint_name");

    let mut stmt = conn.prepare(&sql).map_err(|e| {
        protocol_error_from_anyhow(error_codes::INTERNAL_ERROR, anyhow::Error::from(e))
    })?;
    let rows = stmt
        .query_map(duckdb::params_from_iter(values.iter()), |row| {
            Ok(CheckInfo {
                name: row.get(0)?,
                table: row.get(1)?,
                definition: row.get(2)?,
                ..Default::default()
            })
        })
        .map_err(|e| {
            protocol_error_from_anyhow(error_codes::INTERNAL_ERROR, anyhow::Error::from(e))
        })?;
    let mut out: Vec<CheckInfo> = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| {
            protocol_error_from_anyhow(error_codes::INTERNAL_ERROR, anyhow::Error::from(e))
        })?);
    }
    serde_json::to_value(out).map_err(params_deserialize_error)
}

pub fn handle_schema_functions(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: FunctionsParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    state
        .get_conn(p.conn_id)
        .ok_or_else(|| unknown_conn(p.conn_id))?;

    let out = DUCKDB_BUILTIN_FUNCTIONS
        .iter()
        .map(|function| FunctionInfo {
            name: function.name.to_string(),
            return_type: function.return_type.map(str::to_string),
            args: duckdb_signature_args(function.signature),
            language: Some("duckdb".to_string()),
            definition: None,
            comment: function.description.to_string(),
            extra: serde_json::json!({
                "kind": function.kind,
                "builtin": true,
                "signature": function.signature,
            }),
        })
        .collect::<Vec<_>>();
    serde_json::to_value(out).map_err(params_deserialize_error)
}

// ===================== DDL builder =====================

pub fn handle_ddl_build_create_table(params: &Value) -> Result<Value, ProtocolError> {
    let p: BuildCreateTableParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    serde_json::to_value(crate::ddl::build_create_table(p)).map_err(params_deserialize_error)
}

pub fn handle_ddl_build(params: &Value) -> Result<Value, ProtocolError> {
    let p: BuildDdlParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    let result = crate::ddl::build_ddl(p).map_err(invalid_params)?;
    serde_json::to_value(result).map_err(params_deserialize_error)
}

pub fn handle_ddl_build_alter_table(params: &Value) -> Result<Value, ProtocolError> {
    let p: BuildAlterTableParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    serde_json::to_value(crate::ddl::build_alter_table(p)).map_err(params_deserialize_error)
}

pub fn handle_ddl_build_drop(params: &Value) -> Result<Value, ProtocolError> {
    let p: BuildDropParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    serde_json::to_value(crate::ddl::build_drop(p)).map_err(params_deserialize_error)
}

// ===================== Helpers =====================

pub(crate) fn duckdb_version() -> String {
    // duckdb crate 没有暴露版本号常量,只能用 SELECT version()——为简化,这里硬编码。
    // 真实运行时可以替换为对 conn 跑一次 SELECT version()。
    "duckdb-bundled".to_string()
}

fn declared_methods() -> &'static [&'static str] {
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
        method::TX_BEGIN,
        method::TX_COMMIT,
        method::TX_ROLLBACK,
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

fn duckdb_signature_args(signature: &str) -> Vec<FunctionArg> {
    let Some(inner) = signature
        .split_once('(')
        .and_then(|(_, rest)| rest.rsplit_once(')').map(|(inner, _)| inner))
    else {
        return Vec::new();
    };
    inner
        .split(',')
        .map(|item| item.trim().trim_matches('[').trim_matches(']').trim())
        .filter(|item| !item.is_empty())
        .filter(|item| *item != "...")
        .map(|item| FunctionArg {
            name: item.to_string(),
            type_str: String::new(),
            mode: Some("in".to_string()),
            default: None,
        })
        .collect()
}

fn unknown_conn(id: ConnId) -> ProtocolError {
    ProtocolError::new(
        error_codes::UNKNOWN_CONN_ID,
        format!("unknown conn_id {id}"),
    )
}

fn unknown_cursor(id: &str) -> ProtocolError {
    ProtocolError::new(
        error_codes::UNKNOWN_CURSOR_ID,
        format!("unknown cursor_id `{id}`"),
    )
}

fn unknown_stream(id: &str) -> ProtocolError {
    ProtocolError::new(
        error_codes::RESOURCE_CLOSED,
        format!("unknown stream_id `{id}`"),
    )
}

fn unknown_tx(id: &str) -> ProtocolError {
    ProtocolError::new(error_codes::UNKNOWN_TX_ID, format!("unknown tx_id `{id}`"))
}

fn prepare_query_columns(
    conn: &duckdb::Connection,
    sql: &str,
    bound_params: &[CellValue],
) -> Result<Vec<ColumnSpec>, ProtocolError> {
    let page_sql = page_query_sql(sql, 0, 0);
    let mut stmt = conn.prepare(&page_sql).map_err(duckdb_sql_error)?;
    let params = duckdb_params(bound_params)?;
    let rows_iter = stmt
        .query(duckdb::params_from_iter(params.iter()))
        .map_err(duckdb_sql_error)?;
    Ok(rows_iter
        .as_ref()
        .map(column_specs_from_statement)
        .unwrap_or_default())
}

fn prepare_query_columns_with_catalog_retry(
    conn: &duckdb::Connection,
    sql: &str,
    bound_params: &[CellValue],
) -> Result<(String, Vec<ColumnSpec>), ProtocolError> {
    match prepare_query_columns(conn, sql, bound_params) {
        Ok(columns) => Ok((sql.to_string(), columns)),
        Err(original_error) => {
            let rewritten_sql = rewrite_current_catalog_system_table_references(conn, sql)
                .map_err(|_| original_error.clone())?;
            if rewritten_sql == sql {
                return Err(original_error);
            }
            prepare_query_columns(conn, &rewritten_sql, bound_params)
                .map(|columns| (rewritten_sql, columns))
                .map_err(|_| original_error)
        }
    }
}

fn rewrite_current_catalog_system_table_references(
    conn: &duckdb::Connection,
    sql: &str,
) -> Result<String, ProtocolError> {
    let current_catalog = current_duckdb_catalog_from_connection(conn)?;
    let current_catalog_ref = quote_sql_identifier(&current_catalog);
    if !sql.contains(&current_catalog_ref) {
        return Ok(sql.to_string());
    }

    let mut stmt = conn
        .prepare(
            "SELECT view_name, schema_name FROM duckdb_views() \
             WHERE schema_name IN ('information_schema', 'pg_catalog')",
        )
        .map_err(duckdb_sql_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(duckdb_sql_error)?;
    let mut rewritten = sql.to_string();
    for row in rows {
        let (name, schema) = row.map_err(duckdb_sql_error)?;
        let incorrect_ref = format!("{current_catalog_ref}.{}", quote_sql_identifier(&name));
        let correct_ref = format!(
            "{}.{}",
            quote_sql_identifier(&schema),
            quote_sql_identifier(&name)
        );
        rewritten = rewritten.replace(&incorrect_ref, &correct_ref);
    }
    Ok(rewritten)
}

fn fetch_cursor_page(
    state: &ConnectionState,
    cursor: &mut CursorState,
    requested: Option<u32>,
) -> Result<(Vec<Row>, bool), ProtocolError> {
    if cursor.is_done() {
        return Ok((Vec::new(), true));
    }
    let limit = cursor_fetch_limit(cursor, requested);
    if limit == 0 {
        return Ok((Vec::new(), cursor.is_done()));
    }
    let session = state
        .get_conn(cursor.conn_id())
        .ok_or_else(|| unknown_conn(cursor.conn_id()))?;
    let conn = session
        .connection()
        .map_err(|e| protocol_error_from_anyhow(error_codes::NOT_INITIALIZED, e))?;
    let rows = run_query_page(conn, cursor.sql(), cursor.params(), limit, cursor.offset())?;
    cursor.advance(rows.len(), limit);
    Ok((rows, cursor.is_done()))
}

fn cursor_fetch_limit(cursor: &CursorState, requested: Option<u32>) -> u32 {
    if cursor.remaining_max_rows() == Some(0) {
        return 0;
    }
    let requested = match requested {
        Some(n) => n,
        None => cursor
            .fetch_size()
            .filter(|n| *n > 0)
            .unwrap_or(DEFAULT_CURSOR_FETCH_SIZE),
    }
    .min(MAX_CURSOR_FETCH_SIZE);
    match cursor.remaining_max_rows() {
        Some(remaining) => requested.min(remaining.min(u32::MAX as u64) as u32),
        None => requested,
    }
}

fn run_query_page(
    conn: &duckdb::Connection,
    sql: &str,
    bound_params: &[CellValue],
    limit: u32,
    offset: u64,
) -> Result<Vec<Row>, ProtocolError> {
    let page_sql = page_query_sql(sql, limit, offset);
    let mut stmt = conn.prepare(&page_sql).map_err(duckdb_sql_error)?;
    let params = duckdb_params(bound_params)?;
    let mut rows_iter = stmt
        .query(duckdb::params_from_iter(params.iter()))
        .map_err(duckdb_sql_error)?;
    let column_count = rows_iter
        .as_ref()
        .map(duckdb::Statement::column_count)
        .unwrap_or(0);
    let mut out_rows: Vec<Row> = Vec::new();

    while let Some(row) = rows_iter.next().map_err(|e| {
        protocol_error_from_anyhow(error_codes::INTERNAL_ERROR, anyhow::Error::from(e))
    })? {
        let mut cells: Row = Vec::with_capacity(column_count);
        for i in 0..column_count {
            let cell = row
                .get_ref(i)
                .map(value_ref_to_cell)
                .unwrap_or_else(|_| CellValue::Null);
            cells.push(cell);
        }
        out_rows.push(cells);
    }

    Ok(out_rows)
}

fn page_query_sql(sql: &str, limit: u32, offset: u64) -> String {
    let inner = trimmed_query_sql(sql);
    format!("SELECT * FROM ({inner}) AS __onetcli_cursor_page LIMIT {limit} OFFSET {offset}")
}

fn trimmed_query_sql(sql: &str) -> &str {
    let mut trimmed = sql.trim();
    while let Some(rest) = trimmed.strip_suffix(';') {
        trimmed = rest.trim_end();
    }
    trimmed
}

fn column_specs_from_statement(stmt_ref: &duckdb::Statement<'_>) -> Vec<ColumnSpec> {
    let names = stmt_ref.column_names();
    (0..stmt_ref.column_count())
        .map(|i| {
            let type_str = format!("{:?}", stmt_ref.column_type(i));
            let kind = map_column_type_kind(&type_str);
            let name = names.get(i).cloned().unwrap_or_else(|| format!("col_{i}"));
            ColumnSpec::new(name, type_str, kind)
        })
        .collect()
}

pub(crate) fn duckdb_sql_error(error: duckdb::Error) -> ProtocolError {
    let code = classify_duckdb_sql_error(&error.to_string());
    protocol_error_from_anyhow(code, anyhow::Error::from(error))
}

fn classify_duckdb_sql_error(message: &str) -> i32 {
    let lower = message.to_ascii_lowercase();
    if lower.contains("table") && lower.contains("does not exist") {
        error_codes::SQL_UNKNOWN_TABLE
    } else if lower.contains("column") && lower.contains("does not exist") {
        error_codes::SQL_UNKNOWN_COLUMN
    } else if lower.contains("function") && lower.contains("does not exist") {
        error_codes::SQL_UNKNOWN_FUNCTION
    } else if lower.contains("duplicate key")
        || lower.contains("unique constraint")
        || lower.contains("primary key")
    {
        error_codes::SQL_UNIQUE_VIOLATION
    } else if lower.contains("not null constraint") || lower.contains("not-null constraint") {
        error_codes::SQL_NOT_NULL_VIOLATION
    } else if lower.contains("check constraint") {
        error_codes::SQL_CHECK_VIOLATION
    } else if lower.contains("constraint") {
        error_codes::SQL_CONSTRAINT_VIOLATION
    } else if lower.contains("already exists") {
        error_codes::SQL_OBJECT_ALREADY_EXISTS
    } else {
        error_codes::SQL_SYNTAX_ERROR
    }
}

fn duckdb_params(params: &[CellValue]) -> Result<Vec<duckdb::types::Value>, ProtocolError> {
    params
        .iter()
        .map(cell_value_to_duckdb_value)
        .collect::<anyhow::Result<Vec<_>>>()
        .map_err(|e| invalid_params(format!("invalid bound parameter: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_state() -> ConnectionState {
        ConnectionState::new()
    }

    fn open_main_conn(state: &mut ConnectionState) -> ConnId {
        // 用 TempDir + 不存在的子路径,让 DuckDB 自己创建文件。
        // 不能直接用 NamedTempFile——它会先创建空文件,DuckDB 拒绝把空文件当 db。
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.duckdb").to_string_lossy().to_string();
        // 让 dir 持续到测试结束(每个 conn 单独的 dir,可接受泄漏)
        std::mem::forget(dir);
        let params = serde_json::json!({
            "driver_id": "duckdb",
            "config": { "host": path },
        });
        let result = handle_conn_open(state, &params).unwrap();
        result["conn_id"].as_u64().unwrap()
    }

    fn current_catalog_for_test(state: &ConnectionState, conn_id: ConnId) -> String {
        state
            .get_conn(conn_id)
            .unwrap()
            .connection()
            .unwrap()
            .query_row("SELECT current_database()", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn init_returns_features_and_drivers() {
        let flag = AtomicI64::new(0);
        let params = serde_json::json!({
            "host_version": "1.0.0",
            "api_offered": { "database": "1.0" },
            "instance_id": "test",
            "config": {},
        });
        let v = handle_init(&flag, &params).unwrap();
        assert!(
            v["features"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f == "rich_errors")
        );
        assert!(
            v["features"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f == "server_cursor")
        );
        assert!(
            v["features"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f == "batch_exec")
        );
        assert!(
            v["features"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f == "transactions")
        );
        assert!(
            !v["features"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f == "nested_transactions")
        );
        assert!(
            v["features"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f == "data_pipe")
        );
        assert!(
            !v["features"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f == "sql_tools")
        );
        assert!(
            !v["features"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f == "schema_introspection")
        );
        assert!(
            v["drivers_ready"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d == "duckdb")
        );
    }

    #[test]
    fn init_twice_returns_already_initialized() {
        let flag = AtomicI64::new(0);
        let params = serde_json::json!({
            "host_version": "1.0.0",
            "api_offered": {},
            "instance_id": "test",
            "config": {},
        });
        handle_init(&flag, &params).unwrap();
        let err = handle_init(&flag, &params).unwrap_err();
        assert_eq!(err.code, error_codes::ALREADY_INITIALIZED);
    }

    #[test]
    fn conn_open_rejects_wrong_driver() {
        let mut state = fresh_state();
        let params = serde_json::json!({
            "driver_id": "postgres",
            "config": {},
        });
        let err = handle_conn_open(&mut state, &params).unwrap_err();
        assert_eq!(err.code, error_codes::INVALID_PARAMS);
    }

    #[test]
    fn conn_open_close_ping_full_flow() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        assert_eq!(state.conn_count(), 1);

        let ping =
            handle_conn_ping(&mut state, &serde_json::json!({ "conn_id": conn_id })).unwrap();
        assert!(ping["latency_ms"].is_u64());

        let _ = handle_conn_close(&mut state, &serde_json::json!({ "conn_id": conn_id })).unwrap();
        assert_eq!(state.conn_count(), 0);
    }

    #[test]
    fn conn_test_opens_database_without_persistent_conn() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test-conn.duckdb");
        let result = handle_conn_test(&serde_json::json!({
            "driver_id": "duckdb",
            "config": { "host": path.to_string_lossy() },
        }))
        .unwrap();

        assert_eq!(result["ok"], true);
        assert!(result["latency_ms"].is_u64());
    }

    #[test]
    fn conn_close_unknown_returns_unknown_conn_id() {
        let mut state = fresh_state();
        let err = handle_conn_close(&mut state, &serde_json::json!({"conn_id": 999})).unwrap_err();
        assert_eq!(err.code, error_codes::UNKNOWN_CONN_ID);
    }

    #[test]
    fn conn_use_rejects_unknown_database() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        let err = handle_conn_use(
            &mut state,
            &serde_json::json!({ "conn_id": conn_id, "database": "other" }),
        )
        .unwrap_err();
        assert_eq!(err.code, error_codes::SQL_OBJECT_NOT_FOUND);
    }

    #[test]
    fn query_start_and_fetch_returns_rows() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        let start = handle_query_start(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "SELECT 1 AS a, 'hi' AS b UNION ALL SELECT 2, 'lo'"
            }),
        )
        .unwrap();
        let cursor_id = start["cursor_id"].as_str().unwrap().to_string();
        assert_eq!(start["columns"].as_array().unwrap().len(), 2);
        assert_eq!(start["row_count_known"], false);

        let fetch = handle_cursor_fetch(
            &mut state,
            &serde_json::json!({ "cursor_id": cursor_id, "n": 10 }),
        )
        .unwrap();
        let rows = fetch["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert!(fetch["done"].as_bool().unwrap());
    }

    #[test]
    fn query_fetch_formats_timestamp_values() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        let start = handle_query_start(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "SELECT TIMESTAMP '2026-01-16 09:22:05' AS created_at"
            }),
        )
        .unwrap();
        let cursor_id = start["cursor_id"].as_str().unwrap().to_string();

        let fetch = handle_cursor_fetch(
            &mut state,
            &serde_json::json!({ "cursor_id": cursor_id, "n": 10 }),
        )
        .unwrap();
        let rows = fetch["rows"].as_array().unwrap();

        assert_eq!("datetime", rows[0][0]["type"]);
        assert_eq!("2026-01-16 09:22:05", rows[0][0]["value"]);
    }

    #[test]
    fn query_start_rewrites_current_catalog_system_table_reference() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        let current_catalog = current_catalog_for_test(&state, conn_id);
        let sql = format!(
            "SELECT * FROM \"{}\".\"character_sets\" ORDER BY character_set_name",
            current_catalog
        );

        let start = handle_query_start(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": sql,
                "fetch_size": 10
            }),
        )
        .unwrap();
        let cursor_id = start["cursor_id"].as_str().unwrap().to_string();

        let fetch = handle_cursor_fetch(
            &mut state,
            &serde_json::json!({ "cursor_id": cursor_id, "n": 10 }),
        )
        .unwrap();
        let rows = fetch["rows"].as_array().unwrap();

        assert!(
            !rows.is_empty(),
            "information_schema.character_sets should be queryable through the host-generated catalog-qualified form"
        );
    }

    #[test]
    fn query_start_opens_cursor_without_known_row_count() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        let start = handle_query_start(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "SELECT range AS n FROM range(5) ORDER BY n"
            }),
        )
        .unwrap();
        let cursor_id = start["cursor_id"].as_str().unwrap().to_string();

        assert_eq!(start["row_count_known"], false);
        assert!(start["row_count_estimate"].is_null());

        let first = handle_cursor_fetch(
            &mut state,
            &serde_json::json!({ "cursor_id": cursor_id, "n": 2 }),
        )
        .unwrap();
        assert_eq!(first["rows"].as_array().unwrap().len(), 2);
        assert_eq!(first["done"], false);

        let second = handle_cursor_fetch(
            &mut state,
            &serde_json::json!({ "cursor_id": cursor_id, "n": 10 }),
        )
        .unwrap();
        assert_eq!(second["rows"].as_array().unwrap().len(), 3);
        assert_eq!(second["done"], true);
    }

    #[test]
    fn cursor_fetch_without_n_uses_query_fetch_size() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        let start = handle_query_start(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "SELECT range AS n FROM range(5) ORDER BY n",
                "fetch_size": 2
            }),
        )
        .unwrap();
        let cursor_id = start["cursor_id"].as_str().unwrap().to_string();

        let fetch = handle_cursor_fetch(&mut state, &serde_json::json!({ "cursor_id": cursor_id }))
            .unwrap();

        assert_eq!(fetch["rows"].as_array().unwrap().len(), 2);
        assert_eq!(fetch["done"], false);
    }

    #[test]
    fn zero_fetch_size_falls_back_to_driver_default() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        let start = handle_query_start(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "SELECT range AS n FROM range(5) ORDER BY n",
                "fetch_size": 0
            }),
        )
        .unwrap();
        let cursor_id = start["cursor_id"].as_str().unwrap().to_string();

        let fetch = handle_cursor_fetch(&mut state, &serde_json::json!({ "cursor_id": cursor_id }))
            .unwrap();

        assert_eq!(fetch["rows"].as_array().unwrap().len(), 5);
        assert_eq!(fetch["done"], true);
    }

    #[test]
    fn cursor_close_unknown_returns_unknown_cursor() {
        let mut state = fresh_state();
        let err = handle_cursor_close(&mut state, &serde_json::json!({ "cursor_id": "c-999" }))
            .unwrap_err();
        assert_eq!(err.code, error_codes::UNKNOWN_CURSOR_ID);
    }

    #[test]
    fn exec_run_executes_ddl() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        let result = handle_exec_run(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "CREATE TABLE t (x INT)"
            }),
        )
        .unwrap();
        assert!(result["affected_rows"].is_u64());
    }

    #[test]
    fn exec_run_executes_bound_params() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        handle_exec_run(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "CREATE TABLE bound_exec (id INT, name VARCHAR)"
            }),
        )
        .unwrap();

        let result = handle_exec_run(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "INSERT INTO bound_exec VALUES (?, ?)",
                "params": [
                    { "type": "i64", "value": 7 },
                    { "type": "text", "value": "alice" }
                ]
            }),
        )
        .unwrap();

        assert_eq!(result["affected_rows"], 1);
    }

    #[test]
    fn exec_run_duplicate_key_returns_unique_violation() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        handle_exec_run(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "CREATE TABLE unique_users (id INTEGER PRIMARY KEY)"
            }),
        )
        .unwrap();
        handle_exec_run(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "INSERT INTO unique_users VALUES (1)"
            }),
        )
        .unwrap();

        let err = handle_exec_run(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "INSERT INTO unique_users VALUES (1)"
            }),
        )
        .unwrap_err();

        assert_eq!(err.code, error_codes::SQL_UNIQUE_VIOLATION);
    }

    #[test]
    fn exec_batch_executes_multiple_statements() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        let result = handle_exec_batch(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "statements": [
                    "CREATE TABLE batch_users (id INTEGER)",
                    "INSERT INTO batch_users VALUES (1)",
                    "INSERT INTO batch_users VALUES (2)"
                ],
                "stop_on_error": true,
                "in_transaction": false
            }),
        )
        .unwrap();

        assert_eq!(result["errors"].as_array().unwrap().len(), 0);
        assert_eq!(result["results"].as_array().unwrap().len(), 3);
        let start = handle_query_start(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "SELECT COUNT(*) AS n FROM batch_users"
            }),
        )
        .unwrap();
        let cursor_id = start["cursor_id"].as_str().unwrap();
        let rows = handle_cursor_fetch(
            &mut state,
            &serde_json::json!({ "cursor_id": cursor_id, "n": 1 }),
        )
        .unwrap();
        assert_eq!(rows["rows"][0][0]["value"], 2);
    }

    #[test]
    fn exec_batch_transaction_rolls_back_on_error() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        let result = handle_exec_batch(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "statements": [
                    "CREATE TABLE batch_tx (id INTEGER PRIMARY KEY)",
                    "INSERT INTO batch_tx VALUES (1)",
                    "INSERT INTO batch_tx VALUES (1)"
                ],
                "stop_on_error": true,
                "in_transaction": true
            }),
        )
        .unwrap();

        assert_eq!(result["errors"][0]["index"], 2);
        assert_eq!(
            result["errors"][0]["code"],
            error_codes::SQL_UNIQUE_VIOLATION
        );
        let err = handle_query_start(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "SELECT * FROM batch_tx"
            }),
        )
        .unwrap_err();
        assert_eq!(err.code, error_codes::SQL_UNKNOWN_TABLE);
    }

    #[test]
    fn data_export_streams_ndjson_chunks() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        handle_exec_run(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "CREATE TABLE export_users (id INTEGER, name VARCHAR)"
            }),
        )
        .unwrap();
        handle_exec_batch(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "statements": [
                    "INSERT INTO export_users VALUES (1, 'Ada')",
                    "INSERT INTO export_users VALUES (2, 'Linus')"
                ],
                "stop_on_error": true,
                "in_transaction": false
            }),
        )
        .unwrap();

        let result = handle_data_export(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "SELECT id, name FROM export_users ORDER BY id",
                "format": "ndjson",
                "stream_id": "export-1"
            }),
        )
        .unwrap();
        assert_eq!(
            result["metadata"]["columns"],
            serde_json::json!(["id", "name"])
        );

        let mut bytes = Vec::new();
        loop {
            let chunk = handle_stream_read(
                &mut state,
                &serde_json::json!({ "stream_id": "export-1", "max_bytes": 8 }),
            )
            .unwrap();
            let data = chunk["data"].as_str().unwrap();
            bytes.extend(
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data).unwrap(),
            );
            if chunk["done"].as_bool().unwrap() {
                break;
            }
        }

        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains(r#""id":1"#));
        assert!(text.contains(r#""name":"Ada""#));
        assert!(text.contains(r#""id":2"#));
        assert!(text.ends_with('\n'));
        handle_stream_close(&mut state, &serde_json::json!({ "stream_id": "export-1" })).unwrap();
        let err = handle_stream_read(
            &mut state,
            &serde_json::json!({ "stream_id": "export-1", "max_bytes": 8 }),
        )
        .unwrap_err();
        assert_eq!(err.code, error_codes::RESOURCE_CLOSED);
    }

    #[test]
    fn data_import_inserts_rows_and_commit_reports_totals() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        handle_exec_run(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "CREATE TABLE import_users (id INTEGER, name VARCHAR)"
            }),
        )
        .unwrap();

        let begin = handle_data_import_begin(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "table": "import_users",
                "format": "json",
                "columns": ["id", "name"]
            }),
        )
        .unwrap();
        let import_id = begin["import_id"].as_str().unwrap();
        let chunk = handle_data_import_chunk(
            &mut state,
            &serde_json::json!({
                "import_id": import_id,
                "rows": [
                    [
                        {"type": "i64", "value": 1},
                        {"type": "text", "value": "Ada"}
                    ],
                    [
                        {"type": "i64", "value": 2},
                        {"type": "text", "value": "Linus"}
                    ]
                ]
            }),
        )
        .unwrap();
        assert_eq!(chunk["inserted"], 2);
        assert!(chunk["failed"].as_array().unwrap().is_empty());

        let commit =
            handle_data_import_commit(&mut state, &serde_json::json!({ "import_id": import_id }))
                .unwrap();
        assert_eq!(commit["inserted"], 2);

        let start = handle_query_start(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "SELECT id, name FROM import_users ORDER BY id"
            }),
        )
        .unwrap();
        let cursor_id = start["cursor_id"].as_str().unwrap();
        let rows = handle_cursor_fetch(
            &mut state,
            &serde_json::json!({ "cursor_id": cursor_id, "n": 10 }),
        )
        .unwrap();
        assert_eq!(rows["rows"][0][0]["value"], 1);
        assert_eq!(rows["rows"][0][1]["value"], "Ada");
        assert_eq!(rows["rows"][1][0]["value"], 2);
        assert_eq!(rows["rows"][1][1]["value"], "Linus");
    }

    #[test]
    fn data_import_abort_removes_import_state() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        handle_exec_run(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "CREATE TABLE aborted_import (id INTEGER)"
            }),
        )
        .unwrap();
        let begin = handle_data_import_begin(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "table": "aborted_import",
                "format": "json",
                "columns": ["id"]
            }),
        )
        .unwrap();
        let import_id = begin["import_id"].as_str().unwrap();

        handle_data_import_abort(&mut state, &serde_json::json!({ "import_id": import_id }))
            .unwrap();
        let err =
            handle_data_import_commit(&mut state, &serde_json::json!({ "import_id": import_id }))
                .unwrap_err();

        assert_eq!(err.code, error_codes::UNKNOWN_IMPORT_ID);
    }

    #[test]
    fn data_import_abort_rolls_back_inserted_rows() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        handle_exec_run(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "CREATE TABLE rollback_import (id INTEGER)"
            }),
        )
        .unwrap();
        let begin = handle_data_import_begin(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "table": "rollback_import",
                "format": "json",
                "columns": ["id"]
            }),
        )
        .unwrap();
        let import_id = begin["import_id"].as_str().unwrap();
        handle_data_import_chunk(
            &mut state,
            &serde_json::json!({
                "import_id": import_id,
                "rows": [[{"type": "i64", "value": 9}]]
            }),
        )
        .unwrap();

        handle_data_import_abort(&mut state, &serde_json::json!({ "import_id": import_id }))
            .unwrap();
        let start = handle_query_start(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "SELECT COUNT(*) AS n FROM rollback_import"
            }),
        )
        .unwrap();
        let cursor_id = start["cursor_id"].as_str().unwrap();
        let rows = handle_cursor_fetch(
            &mut state,
            &serde_json::json!({ "cursor_id": cursor_id, "n": 1 }),
        )
        .unwrap();

        assert_eq!(rows["rows"][0][0]["value"], 0);
    }

    #[test]
    fn tx_begin_commit_persists_changes() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        handle_exec_run(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "CREATE TABLE tx_commit_users (id INTEGER)"
            }),
        )
        .unwrap();
        let begin =
            handle_tx_begin(&mut state, &serde_json::json!({ "conn_id": conn_id })).unwrap();
        let tx_id = begin["tx_id"].as_str().unwrap();

        handle_exec_run(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "tx_id": tx_id,
                "sql": "INSERT INTO tx_commit_users VALUES (7)"
            }),
        )
        .unwrap();
        handle_tx_commit(&mut state, &serde_json::json!({ "tx_id": tx_id })).unwrap();

        let start = handle_query_start(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "SELECT COUNT(*) AS n FROM tx_commit_users"
            }),
        )
        .unwrap();
        let cursor_id = start["cursor_id"].as_str().unwrap();
        let rows = handle_cursor_fetch(
            &mut state,
            &serde_json::json!({ "cursor_id": cursor_id, "n": 1 }),
        )
        .unwrap();
        assert_eq!(rows["rows"][0][0]["value"], 1);
    }

    #[test]
    fn tx_begin_rollback_discards_changes() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        handle_exec_run(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "CREATE TABLE tx_rollback_users (id INTEGER)"
            }),
        )
        .unwrap();
        let begin =
            handle_tx_begin(&mut state, &serde_json::json!({ "conn_id": conn_id })).unwrap();
        let tx_id = begin["tx_id"].as_str().unwrap();

        handle_exec_run(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "tx_id": tx_id,
                "sql": "INSERT INTO tx_rollback_users VALUES (7)"
            }),
        )
        .unwrap();
        handle_tx_rollback(&mut state, &serde_json::json!({ "tx_id": tx_id })).unwrap();

        let start = handle_query_start(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "SELECT COUNT(*) AS n FROM tx_rollback_users"
            }),
        )
        .unwrap();
        let cursor_id = start["cursor_id"].as_str().unwrap();
        let rows = handle_cursor_fetch(
            &mut state,
            &serde_json::json!({ "cursor_id": cursor_id, "n": 1 }),
        )
        .unwrap();
        assert_eq!(rows["rows"][0][0]["value"], 0);
    }

    #[test]
    fn tx_begin_rejects_nested_transaction() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        handle_tx_begin(&mut state, &serde_json::json!({ "conn_id": conn_id })).unwrap();

        let err =
            handle_tx_begin(&mut state, &serde_json::json!({ "conn_id": conn_id })).unwrap_err();

        assert_eq!(err.code, error_codes::TX_NESTED_NOT_SUPPORTED);
    }

    #[test]
    fn tx_rollback_to_savepoint_is_not_supported() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        let begin =
            handle_tx_begin(&mut state, &serde_json::json!({ "conn_id": conn_id })).unwrap();
        let tx_id = begin["tx_id"].as_str().unwrap();

        let err = handle_tx_rollback(
            &mut state,
            &serde_json::json!({ "tx_id": tx_id, "to_savepoint": "sp1" }),
        )
        .unwrap_err();

        assert_eq!(err.code, error_codes::TX_NESTED_NOT_SUPPORTED);
    }

    #[test]
    fn exec_run_rejects_unknown_tx_id() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);

        let err = handle_exec_run(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "tx_id": "missing-tx",
                "sql": "CREATE TABLE missing_tx (id INTEGER)"
            }),
        )
        .unwrap_err();

        assert_eq!(err.code, error_codes::UNKNOWN_TX_ID);
    }

    #[test]
    fn query_start_executes_bound_params() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        let start = handle_query_start(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "SELECT ?::INTEGER AS id, ?::VARCHAR AS name",
                "params": [
                    { "type": "i64", "value": 1 },
                    { "type": "text", "value": "alice" }
                ]
            }),
        )
        .unwrap();
        let cursor_id = start["cursor_id"].as_str().unwrap().to_string();

        let fetch = handle_cursor_fetch(
            &mut state,
            &serde_json::json!({ "cursor_id": cursor_id, "n": 10 }),
        )
        .unwrap();
        let rows = fetch["rows"].as_array().unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0]["value"], 1);
        assert_eq!(rows[0][1]["value"], "alice");
    }

    #[test]
    fn query_start_unknown_table_returns_unknown_table() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        let err = handle_query_start(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "SELECT * FROM missing_table"
            }),
        )
        .unwrap_err();

        assert_eq!(err.code, error_codes::SQL_UNKNOWN_TABLE);
    }

    #[test]
    fn query_start_supports_show_tables() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        handle_exec_run(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "CREATE TABLE shown_table (id INTEGER)"
            }),
        )
        .unwrap();

        let start = handle_query_start(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "SHOW TABLES"
            }),
        )
        .unwrap();
        let cursor_id = start["cursor_id"].as_str().unwrap().to_string();
        let fetch = handle_cursor_fetch(
            &mut state,
            &serde_json::json!({ "cursor_id": cursor_id, "n": 10 }),
        )
        .unwrap();
        let rows = fetch["rows"].as_array().unwrap();

        assert!(rows.iter().any(|row| row[0]["value"] == "shown_table"));
    }

    #[test]
    fn query_start_supports_describe_table() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        handle_exec_run(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "CREATE TABLE described_table (id INTEGER, name VARCHAR)"
            }),
        )
        .unwrap();

        let start = handle_query_start(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "DESCRIBE described_table"
            }),
        )
        .unwrap();
        let cursor_id = start["cursor_id"].as_str().unwrap().to_string();
        let fetch = handle_cursor_fetch(
            &mut state,
            &serde_json::json!({ "cursor_id": cursor_id, "n": 10 }),
        )
        .unwrap();
        let rows = fetch["rows"].as_array().unwrap();

        assert!(rows.iter().any(|row| row[0]["value"] == "id"));
        assert!(rows.iter().any(|row| row[0]["value"] == "name"));
    }

    #[test]
    fn schema_databases_returns_catalog_name() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        let result =
            handle_schema_databases(&mut state, &serde_json::json!({ "conn_id": conn_id }))
                .unwrap();
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["name"], current_catalog_for_test(&state, conn_id));
    }

    #[test]
    fn schema_objects_lists_created_table() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        handle_exec_run(
            &mut state,
            &serde_json::json!({ "conn_id": conn_id, "sql": "CREATE TABLE users (id INT, name VARCHAR)" }),
        )
        .unwrap();
        let result =
            handle_schema_objects(&mut state, &serde_json::json!({ "conn_id": conn_id })).unwrap();
        let arr = result.as_array().unwrap();
        assert!(
            arr.iter()
                .any(|o| o["name"] == "users" && o["kind"] == "table")
        );
    }

    #[test]
    fn schema_objects_treats_main_database_as_default_duckdb_database() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        handle_exec_run(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "CREATE TABLE users (id INT, name VARCHAR)"
            }),
        )
        .unwrap();

        let result = handle_schema_objects(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "database": "main",
                "schema": "main",
                "kinds": ["table"]
            }),
        )
        .unwrap();
        let objects = result.as_array().unwrap();

        assert!(
            objects
                .iter()
                .any(|object| object["name"] == "users" && object["kind"] == "table"),
            "main database filter should not hide main schema tables: {objects:?}"
        );
    }

    #[test]
    fn schema_objects_treats_catalog_name_as_default_duckdb_database() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        handle_exec_run(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "CREATE TABLE users (id INT, name VARCHAR)"
            }),
        )
        .unwrap();

        let current_catalog = current_catalog_for_test(&state, conn_id);
        let result = handle_schema_objects(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "database": current_catalog,
                "schema": "main",
                "kinds": ["table"]
            }),
        )
        .unwrap();
        let objects = result.as_array().unwrap();

        assert!(
            objects
                .iter()
                .any(|object| object["name"] == "users" && object["kind"] == "table"),
            "catalog database filter should not hide main schema tables: {objects:?}"
        );
    }

    #[test]
    fn schema_objects_lists_information_schema_tables() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);

        let result = handle_schema_objects(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "schema": "information_schema",
                "kinds": ["table"]
            }),
        )
        .unwrap();
        let objects = result.as_array().unwrap();

        assert!(
            !objects.is_empty(),
            "information_schema should expose its catalog tables"
        );
    }

    #[test]
    fn schema_objects_lists_pg_catalog_tables() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);

        let result = handle_schema_objects(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "schema": "pg_catalog",
                "kinds": ["table"]
            }),
        )
        .unwrap();
        let objects = result.as_array().unwrap();

        assert!(
            !objects.is_empty(),
            "pg_catalog should expose its catalog tables"
        );
    }

    #[test]
    fn schema_objects_respects_schema_filter() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        for sql in [
            "CREATE SCHEMA alt",
            "CREATE TABLE main.main_only (id INT)",
            "CREATE TABLE alt.alt_only (id INT)",
        ] {
            handle_exec_run(
                &mut state,
                &serde_json::json!({ "conn_id": conn_id, "sql": sql }),
            )
            .unwrap();
        }

        let main_result = handle_schema_objects(
            &mut state,
            &serde_json::json!({ "conn_id": conn_id, "schema": "main" }),
        )
        .unwrap();
        let main_objects = main_result.as_array().unwrap();

        assert!(
            main_objects
                .iter()
                .any(|object| object["name"] == "main_only"),
            "main schema should include main_only: {main_objects:?}"
        );
        assert!(
            !main_objects
                .iter()
                .any(|object| object["name"] == "alt_only"),
            "main schema should not include alt_only: {main_objects:?}"
        );

        let alt_result = handle_schema_objects(
            &mut state,
            &serde_json::json!({ "conn_id": conn_id, "schema": "alt" }),
        )
        .unwrap();
        let alt_objects = alt_result.as_array().unwrap();

        assert!(
            alt_objects
                .iter()
                .any(|object| object["name"] == "alt_only"),
            "alt schema should include alt_only: {alt_objects:?}"
        );
        assert!(
            !alt_objects
                .iter()
                .any(|object| object["name"] == "main_only"),
            "alt schema should not include main_only: {alt_objects:?}"
        );
    }

    #[test]
    fn schema_columns_returns_definition() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        handle_exec_run(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "CREATE TABLE u (id INT NOT NULL, name VARCHAR)"
            }),
        )
        .unwrap();
        let result = handle_schema_columns(
            &mut state,
            &serde_json::json!({ "conn_id": conn_id, "table": "u" }),
        )
        .unwrap();
        let cols = result.as_array().unwrap();
        assert_eq!(cols.len(), 2);
        assert_eq!(cols[0]["name"], "id");
        assert_eq!(cols[0]["nullable"], false);
        assert_eq!(cols[1]["name"], "name");
    }

    #[test]
    fn schema_columns_respects_schema_filter_for_same_table_name() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        for sql in [
            "CREATE SCHEMA alt",
            "CREATE TABLE main.dupe (main_id INT)",
            "CREATE TABLE alt.dupe (alt_id INT)",
        ] {
            handle_exec_run(
                &mut state,
                &serde_json::json!({ "conn_id": conn_id, "sql": sql }),
            )
            .unwrap();
        }

        let result = handle_schema_columns(
            &mut state,
            &serde_json::json!({ "conn_id": conn_id, "schema": "main", "table": "dupe" }),
        )
        .unwrap();
        let columns = result.as_array().unwrap();

        assert_eq!(1, columns.len(), "main.dupe columns: {columns:?}");
        assert_eq!("main_id", columns[0]["name"]);
    }

    #[test]
    fn schema_indexes_respects_schema_filter_for_same_table_name() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        for sql in [
            "CREATE SCHEMA alt",
            "CREATE TABLE main.dupe_idx (main_id INT)",
            "CREATE TABLE alt.dupe_idx (alt_id INT)",
            "CREATE INDEX idx_main_dupe ON main.dupe_idx (main_id)",
            "CREATE INDEX idx_alt_dupe ON alt.dupe_idx (alt_id)",
        ] {
            handle_exec_run(
                &mut state,
                &serde_json::json!({ "conn_id": conn_id, "sql": sql }),
            )
            .unwrap();
        }

        let result = handle_schema_indexes(
            &mut state,
            &serde_json::json!({ "conn_id": conn_id, "schema": "main", "table": "dupe_idx" }),
        )
        .unwrap();
        let indexes = result.as_array().unwrap();

        assert_eq!(1, indexes.len(), "main.dupe_idx indexes: {indexes:?}");
        assert_eq!("idx_main_dupe", indexes[0]["name"]);
        assert_eq!("art", indexes[0]["kind"]);
    }

    #[test]
    fn schema_columns_requires_table() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        let err = handle_schema_columns(
            &mut state,
            &serde_json::json!({ "conn_id": conn_id, "table": "" }),
        )
        .unwrap_err();
        assert_eq!(err.code, error_codes::INVALID_PARAMS);
    }

    #[test]
    fn schema_schemas_returns_main_namespace() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        let result = handle_schema_schemas(
            &mut state,
            &serde_json::json!({ "conn_id": conn_id, "database": "main" }),
        )
        .unwrap();
        let arr = result.as_array().unwrap();
        let names = arr
            .iter()
            .map(|schema| schema["name"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        let mut unique_names = names.clone();
        unique_names.sort();
        unique_names.dedup();

        assert!(names.iter().any(|name| name == "main"));
        assert_eq!(
            unique_names, names,
            "schema list should be unique: {names:?}"
        );
    }

    #[test]
    fn schema_checks_returns_check_constraints() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);
        handle_exec_run(
            &mut state,
            &serde_json::json!({
                "conn_id": conn_id,
                "sql": "CREATE TABLE guarded (
                    id INT,
                    amount INT,
                    CONSTRAINT positive_amount CHECK (amount > 0)
                )"
            }),
        )
        .unwrap();

        let result = handle_schema_checks(
            &mut state,
            &serde_json::json!({ "conn_id": conn_id, "table": "guarded" }),
        )
        .unwrap();
        let checks = result.as_array().unwrap();

        assert_eq!(checks.len(), 1);
        assert!(!checks[0]["name"].as_str().unwrap_or_default().is_empty());
        assert_eq!(checks[0]["table"], "guarded");
        assert!(
            checks[0]["definition"]
                .as_str()
                .unwrap_or_default()
                .contains("amount")
        );
    }

    #[test]
    fn schema_functions_returns_builtin_functions() {
        let mut state = fresh_state();
        let conn_id = open_main_conn(&mut state);

        let result = handle_schema_functions(
            &mut state,
            &serde_json::json!({ "conn_id": conn_id, "database": "main" }),
        )
        .unwrap();
        let functions = result.as_array().unwrap();

        let lower = functions
            .iter()
            .find(|function| function["name"] == "lower")
            .expect("DuckDB builtin functions should include lower()");
        assert_eq!(lower["extra"]["kind"], "scalar");
        assert_eq!(lower["return_type"], "VARCHAR");
        assert!(
            lower["args"]
                .as_array()
                .is_some_and(|args| !args.is_empty())
        );
    }

    #[test]
    fn init_advertises_declared_ddl_methods() {
        let flag = AtomicI64::new(0);
        let result = handle_init(
            &flag,
            &serde_json::json!({
                "host_version": "1.0.0",
                "api_offered": { "database": "1.0" },
                "instance_id": "test",
                "config": {},
            }),
        )
        .unwrap();
        let methods = result["methods"].as_array().unwrap();
        assert!(methods.iter().any(|m| m == method::CONN_TEST));
        assert!(methods.iter().any(|m| m == method::SCHEMA_CHECKS));
        assert!(methods.iter().any(|m| m == method::SCHEMA_FUNCTIONS));
        assert!(methods.iter().any(|m| m == method::EXEC_BATCH));
        assert!(methods.iter().any(|m| m == method::TX_BEGIN));
        assert!(methods.iter().any(|m| m == method::TX_COMMIT));
        assert!(methods.iter().any(|m| m == method::TX_ROLLBACK));
        assert!(!methods.iter().any(|m| m == method::TX_SAVEPOINT));
        assert!(!methods.iter().any(|m| m == method::TX_RELEASE));
        assert!(methods.iter().any(|m| m == method::DATA_EXPORT));
        assert!(methods.iter().any(|m| m == method::DATA_IMPORT_BEGIN));
        assert!(methods.iter().any(|m| m == method::DATA_IMPORT_CHUNK));
        assert!(methods.iter().any(|m| m == method::DATA_IMPORT_COMMIT));
        assert!(methods.iter().any(|m| m == method::DATA_IMPORT_ABORT));
        assert!(methods.iter().any(|m| m == method::STREAM_READ));
        assert!(methods.iter().any(|m| m == method::STREAM_CLOSE));
        assert!(methods.iter().any(|m| m == method::DDL_BUILD));
        assert!(methods.iter().any(|m| m == method::DDL_BUILD_CREATE_TABLE));
        assert!(methods.iter().any(|m| m == method::DDL_BUILD_ALTER_TABLE));
        assert!(methods.iter().any(|m| m == method::DDL_BUILD_DROP));
    }

    #[test]
    fn driver_manifest_declares_ddl_methods() {
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../driver.json")).unwrap();
        let methods = manifest["methods"].as_array().unwrap();
        assert!(methods.iter().any(|m| m == method::CONN_TEST));
        assert!(methods.iter().any(|m| m == method::SCHEMA_CHECKS));
        assert!(methods.iter().any(|m| m == method::SCHEMA_FUNCTIONS));
        assert!(methods.iter().any(|m| m == method::EXEC_BATCH));
        assert!(methods.iter().any(|m| m == method::TX_BEGIN));
        assert!(methods.iter().any(|m| m == method::TX_COMMIT));
        assert!(methods.iter().any(|m| m == method::TX_ROLLBACK));
        assert!(!methods.iter().any(|m| m == method::TX_SAVEPOINT));
        assert!(!methods.iter().any(|m| m == method::TX_RELEASE));
        assert!(methods.iter().any(|m| m == method::DATA_EXPORT));
        assert!(methods.iter().any(|m| m == method::DATA_IMPORT_BEGIN));
        assert!(methods.iter().any(|m| m == method::DATA_IMPORT_CHUNK));
        assert!(methods.iter().any(|m| m == method::DATA_IMPORT_COMMIT));
        assert!(methods.iter().any(|m| m == method::DATA_IMPORT_ABORT));
        assert!(methods.iter().any(|m| m == method::STREAM_READ));
        assert!(methods.iter().any(|m| m == method::STREAM_CLOSE));
        assert!(methods.iter().any(|m| m == method::DDL_BUILD));
        assert!(methods.iter().any(|m| m == method::DDL_BUILD_CREATE_TABLE));
        assert!(methods.iter().any(|m| m == method::DDL_BUILD_ALTER_TABLE));
        assert!(methods.iter().any(|m| m == method::DDL_BUILD_DROP));
    }

    #[test]
    fn driver_manifest_methods_match_init_declared_methods() {
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../driver.json")).unwrap();
        let manifest_methods = manifest["methods"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect::<Vec<_>>();

        assert_eq!(declared_methods(), manifest_methods.as_slice());
    }

    #[test]
    fn driver_manifest_advertises_supported_schema_groups() {
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../driver.json")).unwrap();
        let capabilities = &manifest["capabilities"];

        assert_eq!(capabilities["supports_functions"], true);
        assert_eq!(capabilities["supports_procedures"], false);
        assert_eq!(capabilities["supports_triggers"], false);
        assert_eq!(capabilities["supports_sequences"], false);
    }

    #[test]
    fn driver_manifest_declares_table_context_actions() {
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../driver.json")).unwrap();
        let actions = manifest["ui"]["form"]["actions"]["actions"]
            .as_array()
            .unwrap();

        assert!(
            actions.iter().any(|action| {
                action["id"] == "OpenTableData"
                    && action["placement"] == "Both"
                    && action["targets"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|target| target["node_type"] == "Table")
            }),
            "DuckDB table nodes should expose open data context/toolbar action: {actions:?}"
        );
        assert!(
            actions.iter().any(|action| {
                action["id"] == "ImportData"
                    && action["placement"] == "ContextMenu"
                    && action["targets"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|target| target["node_type"] == "Table")
            }),
            "DuckDB table nodes should expose import data context action: {actions:?}"
        );
        assert!(
            actions.iter().any(|action| {
                action["id"] == "ExportData"
                    && action["placement"] == "ContextMenu"
                    && action["targets"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|target| target["node_type"] == "Table")
            }),
            "DuckDB table nodes should expose export data context action: {actions:?}"
        );
    }

    #[test]
    fn driver_manifest_declares_schema_aware_actions() {
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../driver.json")).unwrap();
        let actions = manifest["ui"]["form"]["actions"]["actions"]
            .as_array()
            .unwrap();

        for (id, target) in [
            ("RunSqlFile", "Schema"),
            ("DesignTable", "Schema"),
            ("CreateNewQuery", "Schema"),
            ("DumpSqlStructure", "Database"),
            ("DumpSqlStructure", "Schema"),
            ("DumpSqlData", "Database"),
            ("DumpSqlData", "Schema"),
            ("DumpSqlStructureAndData", "Database"),
            ("DumpSqlStructureAndData", "Schema"),
        ] {
            assert!(
                actions.iter().any(|action| {
                    action["id"] == id
                        && action["targets"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|candidate| candidate["node_type"] == target)
                }),
                "DuckDB manifest should expose {id} for {target}: {actions:?}"
            );
        }
    }

    #[test]
    fn ddl_build_create_table_returns_duckdb_sql() {
        let result = handle_ddl_build_create_table(&serde_json::json!({
            "spec": {
                "name": "events",
                "columns": [
                    {"name": "id", "type": "INTEGER", "nullable": false, "is_primary": true},
                    {"name": "payload", "type": "VARCHAR", "default": "'{}'"}
                ],
                "indexes": [
                    {"name": "idx_events_payload", "columns": ["payload"]}
                ]
            }
        }))
        .unwrap();

        assert_eq!(
            result["sql"],
            "CREATE TABLE \"events\" (\"id\" INTEGER NOT NULL PRIMARY KEY, \"payload\" VARCHAR DEFAULT '{}')"
        );
        assert_eq!(
            result["statements"][1],
            "CREATE INDEX \"idx_events_payload\" ON \"events\" (\"payload\")"
        );
    }

    #[test]
    fn ddl_build_dispatches_generic_create_table() {
        let result = handle_ddl_build(&serde_json::json!({
            "op": "create_table",
            "payload": {
                "spec": {
                    "name": "events",
                    "schema": "analytics",
                    "columns": [
                        {"name": "id", "type": "INTEGER", "nullable": false, "is_primary": true}
                    ]
                },
                "options": {"if_not_exists": true}
            }
        }))
        .unwrap();

        assert_eq!(
            result["statements"][0],
            "CREATE TABLE IF NOT EXISTS \"analytics\".\"events\" (\"id\" INTEGER NOT NULL PRIMARY KEY)"
        );
    }

    #[test]
    fn ddl_build_dispatches_generic_drop_view() {
        let result = handle_ddl_build(&serde_json::json!({
            "op": "drop_view",
            "payload": {
                "name": "daily_sales",
                "schema": "analytics",
                "if_exists": true,
                "cascade": true
            }
        }))
        .unwrap();

        assert_eq!(
            result["statements"],
            serde_json::json!(["DROP VIEW IF EXISTS \"analytics\".\"daily_sales\" CASCADE"])
        );
    }

    #[test]
    fn ddl_build_dispatches_generic_schema_and_table_ops() {
        let create_schema = handle_ddl_build(&serde_json::json!({
            "op": "create_schema",
            "payload": {"name": "analytics", "if_not_exists": true}
        }))
        .unwrap();
        assert_eq!(
            create_schema["statements"],
            serde_json::json!(["CREATE SCHEMA IF NOT EXISTS \"analytics\""])
        );

        let drop_schema = handle_ddl_build(&serde_json::json!({
            "op": "drop_schema",
            "payload": {"schema": "analytics", "if_exists": true, "cascade": true}
        }))
        .unwrap();
        assert_eq!(
            drop_schema["statements"],
            serde_json::json!(["DROP SCHEMA IF EXISTS \"analytics\" CASCADE"])
        );

        let rename_table = handle_ddl_build(&serde_json::json!({
            "op": "rename_table",
            "payload": {"schema": "analytics", "name": "events", "new_name": "event_log"}
        }))
        .unwrap();
        assert_eq!(
            rename_table["statements"],
            serde_json::json!(["ALTER TABLE \"analytics\".\"events\" RENAME TO \"event_log\""])
        );

        let truncate_table = handle_ddl_build(&serde_json::json!({
            "op": "truncate_table",
            "payload": {"schema": "analytics", "table": "event_log"}
        }))
        .unwrap();
        assert_eq!(
            truncate_table["statements"],
            serde_json::json!(["TRUNCATE TABLE \"analytics\".\"event_log\""])
        );
    }

    #[test]
    fn ddl_build_dispatches_generic_column_definition() {
        let result = handle_ddl_build(&serde_json::json!({
            "op": "column_definition",
            "payload": {
                "name": "payload",
                "type": "VARCHAR",
                "nullable": false,
                "default": "'{}'"
            }
        }))
        .unwrap();

        assert_eq!(
            result["statements"],
            serde_json::json!(["\"payload\" VARCHAR NOT NULL DEFAULT '{}'"])
        );
    }

    #[test]
    fn ddl_build_drop_quotes_table_and_view_names() {
        let table = handle_ddl_build_drop(&serde_json::json!({
            "kind": "table",
            "name": "odd\"table",
            "if_exists": true
        }))
        .unwrap();
        assert_eq!(table["sql"], "DROP TABLE IF EXISTS \"odd\"\"table\"");

        let view = handle_ddl_build_drop(&serde_json::json!({
            "kind": "view",
            "name": "v_events",
            "if_exists": false
        }))
        .unwrap();
        assert_eq!(view["sql"], "DROP VIEW \"v_events\"");
    }
}
