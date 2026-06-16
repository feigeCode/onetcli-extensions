# IPC Database Protocol Reference

## Table of Contents

- [Wire Model](#wire-model)
- [Method Families](#method-families)
- [Routing Pattern](#routing-pattern)
- [Schema Metadata](#schema-metadata)
- [DDL Builders](#ddl-builders)
- [Errors and Fallbacks](#errors-and-fallbacks)

## Wire Model

onetcli IPC database drivers speak JSON-RPC-like request/response messages over the host-managed local transport declared in `driver.json`.

Authoritative Rust sources in the host repo:

- `crates/extension-protocol/src/method.rs`
- `crates/extension-protocol/src/conn.rs`
- `crates/extension-protocol/src/schema.rs`
- `crates/extension-protocol/src/query.rs`
- `crates/extension-protocol/src/ddl.rs`
- `crates/extension-driver/src/runtime.rs`

Rust drivers should use `extension_protocol::method` constants instead of literal method strings. Non-Rust drivers should mirror the same strings exactly and keep a test that validates manifest method names.

## Method Families

| Family | Methods | Notes |
| --- | --- | --- |
| Protocol meta | `$/ping`, `$/cancelRequest`, `shutdown` | Process and request lifecycle. |
| Connection | `conn/test`, `conn/open`, `conn/close`, `conn/ping`, `conn/use` | `conn/open` returns `conn_id`; connection-scoped methods receive it. |
| Query/cursor | `query/start`, `cursor/fetch`, `cursor/cancel`, `cursor/close` | Use server-side or emulated cursors. Preserve cancellation behavior. |
| Exec | `exec/run`, `exec/batch` | Non-query SQL execution and batch scripts. |
| Transaction | `tx/begin`, `tx/commit`, `tx/rollback`, optional savepoint methods | Return typed unsupported errors if unavailable. |
| Schema | `schema/databases`, `schema/schemas`, `schema/objects`, `schema/columns`, `schema/views`, `schema/indexes`, `schema/checks`, etc. | Used by tree, details, completion, and data grid. |
| DDL | `ddl/build`, `ddl/build_create_table`, `ddl/build_alter_table`, `ddl/build_drop` | SQL generation only. |
| Data pipe | `data/export`, `data/import_begin`, `data/import_chunk`, `data/import_commit`, `data/import_abort`, `stream/read`, `stream/close` | Used for larger transfer workflows. |
| Host API | `host/*` | Extension-to-host calls when supported by runtime. |

## Routing Pattern

Use two routing layers:

1. Driver/control plane:
   - `init`
   - `conn/test`
   - `conn/open`
   - pure `ddl/build*` when no live database state is required
2. Opened connection:
   - `conn/ping`, `conn/use`
   - query/cursor/exec/tx
   - schema metadata
   - import/export/stream
   - `ddl/build*` as an accepted duplicate path when host injects `conn_id`

For Rust drivers using `extension-driver`, implement:

```rust
impl Driver for MyDriver {
    fn call_connless(&self, method_name: &str, params: &Value) -> Result<Value, ProtocolError> {
        match method_name {
            method::CONN_TEST => handle_conn_test(params),
            method::DDL_BUILD => handle_ddl_build(params),
            other => Err(method_not_found(other)),
        }
    }
}

impl DriverConnection for MyConnection {
    fn call(&mut self, method_name: &str, params: &Value) -> Result<Value, ProtocolError> {
        match method_name {
            method::SCHEMA_OBJECTS => handle_schema_objects(&mut self.state, params),
            method::QUERY_START => handle_query_start(&mut self.state, params),
            method::DDL_BUILD => handle_ddl_build(params),
            other => Err(method_not_found(other)),
        }
    }
}
```

Non-Rust runtimes should implement the same split in their JSON-RPC dispatcher.

## Schema Metadata

Return structured objects matching `extension-protocol/src/schema.rs`.

Important params:

- `schema/databases`: `{ "conn_id": number }`
- `schema/schemas`: `{ "conn_id": number, "database": string }`
- `schema/objects`: `{ "conn_id": number, "database"?: string, "schema"?: string, "kinds": [...] }`
- `schema/columns`: `{ "conn_id": number, "database"?: string, "schema"?: string, "table": string }`

Important result fields:

- Database: `name`, optional `charset`, `collation`, `owner`, `size_bytes`, `comment`, `extra`
- Schema: `name`, optional `owner`, `comment`, `extra`
- Object: `name`, `kind`, optional `row_count_estimate`, `size_bytes`, timestamps, `comment`, `extra`
- Column: `ordinal`, `name`, `type`, `raw_type`, `nullable`, `default`, primary/unique flags, numeric/string sizing, `comment`, `extra`

Catalog rules:

- Query the database's catalog metadata as the source of truth.
- If a database exposes `current_database()` or equivalent, use it for the current/default catalog returned by `schema/databases`.
- If the host sends legacy/default aliases such as `main`, treat them as equivalent filters only when the backend semantics justify it.
- Include `information_schema`, `pg_catalog`, and other system schemas when they are real visible schemas and the request asks for schema/object listings.
- Always qualify data-grid and metadata SQL with the returned catalog/schema names. A bug pattern is returning `character_sets` but later querying `"default_catalog"."character_sets"` instead of `information_schema.character_sets`.

## DDL Builders

DDL builders translate declarative specs into SQL strings. They must not execute SQL.

Core params and results:

- `ddl/build`: `{ "conn_id"?: number, "op": "create_table" | "...", "payload": object }` -> `{ "statements": string[], "warnings": string[] }`
- `ddl/build_create_table`: `{ "conn_id"?: number, "spec": TableSpec, "options": CreateTableOptions }` -> `{ "sql": string, "statements": string[] }`
- `ddl/build_alter_table`: `{ "conn_id"?: number, "from_spec": TableSpec, "to_spec": TableSpec, "column_renames": [], "options": AlterTableOptions }` -> `{ "statements": string[], "rollback_statements": string[], "warnings": string[] }`
- `ddl/build_drop`: `{ "kind": ObjectKind, "name": string, "schema"?: string, "database"?: string, "if_exists": bool, "cascade": bool }` -> `{ "sql": string }`

`TableSpec` includes `name`, optional `schema` and `database`, `columns`, `primary_key`, `indexes`, `foreign_keys`, `comment`, and driver-specific `options`.

Keep generic `ddl/build` in sync with specialized methods when both are declared. It is acceptable to implement specialized methods first and route generic ops to the same builder.

## Errors and Fallbacks

Use typed protocol errors:

- Invalid params: malformed JSON or unsupported config shape.
- Connection errors: database connection failed.
- Method not found / not supported: method intentionally unavailable.
- Query errors: SQL/backend execution failure.

Host fallback behavior depends on not-supported errors. If a driver does not implement a method and `driver.json` declares a compatible built-in database type, the host may use built-in SQL builders for DDL generation.

Do not hide unsupported behavior behind successful empty responses unless the protocol explicitly treats empty as meaningful.
