# driver.json Manifest Reference

## Table of Contents

- [Required Shape](#required-shape)
- [Methods](#methods)
- [Dialect](#dialect)
- [Capabilities](#capabilities)
- [UI Form](#ui-form)
- [Compatibility Fallback](#compatibility-fallback)

## Required Shape

`driver.json` is the host's first contract with an IPC database driver.

Minimal shape:

```json
{
  "id": "postgres-compatible",
  "name": "Postgres Compatible",
  "description": "Example IPC database driver",
  "version": "1.0.0",
  "entry": {
    "command": "./postgres_driver",
    "args": [],
    "working_dir": null
  },
  "transport": {
    "name": "postgres-compatible.sock",
    "connect_timeout_ms": 5000
  },
  "methods": [
    "$/ping",
    "shutdown",
    "conn/test",
    "conn/open",
    "conn/close",
    "query/start",
    "cursor/fetch",
    "cursor/close",
    "schema/databases",
    "schema/schemas",
    "schema/objects",
    "schema/columns"
  ],
  "dialect": {
    "identifier_quote": "\"",
    "compatible_database_type": "PostgreSQL"
  },
  "capabilities": {
    "supports_schema": true,
    "uses_schema_as_database": false
  },
  "ui": {
    "icon": "Database",
    "locales_dir": "locales",
    "default_port": 5432
  }
}
```

The host deserializes this into `IpcDriverManifest` in `crates/db/src/ipc/registry.rs`.

## Methods

Declare only methods the driver handles.

Rules:

- Standard method names must be known by `extension_protocol::method::ALL_METHODS`.
- Private extension methods must use the `x/...` namespace.
- A declared method can still return a typed unsupported error for a database-specific limitation, but it should not be missing from the dispatcher by accident.
- Keep `driver.json.methods`, dispatcher match arms, and tests in sync.

Common starter set:

```json
[
  "$/ping",
  "shutdown",
  "conn/test",
  "conn/open",
  "conn/close",
  "conn/ping",
  "query/start",
  "cursor/fetch",
  "cursor/close",
  "exec/run",
  "schema/databases",
  "schema/schemas",
  "schema/objects",
  "schema/columns"
]
```

Add `ddl/build*`, transactions, import/export, functions, indexes, checks, and views only when implemented.

## Dialect

Supported dialect fields currently include:

| Field | Meaning |
| --- | --- |
| `identifier_quote` | Symmetric quote character, default is `"`. |
| `identifier_quote_left` / `identifier_quote_right` | Asymmetric quotes, for example `[` and `]`. |
| `limit_style` | Host SQL generation limit style. |
| `bool_true` / `bool_false` | Boolean literal strings. |
| `explain_template` | Template such as `EXPLAIN {sql}`. |
| `compatible_database_type` | Host built-in database type used for fallback SQL builders. |
| `supports_schema` | Legacy dialect capability; prefer top-level capabilities when available. |
| `supports_sequences` | Legacy dialect capability. |
| `uses_schema_as_database` | Legacy dialect capability. |

Use `identifier_quote_left/right` when a database uses bracket or paired quoting. Do not make the driver hand-roll host SQL quoting if the dialect field can express it.

## Capabilities

Capabilities drive UI visibility and optional metadata calls. Typical fields include:

- `supports_schema`
- `uses_schema_as_database`
- `supports_sequences`
- `supports_functions`
- `supports_procedures`
- `supports_triggers`
- `supports_table_engine`
- `supports_table_charset`
- `supports_table_collation`
- `supports_auto_increment`
- `supports_tablespace`
- `supports_unsigned`
- `supports_enum_values`
- `show_charset_in_column_detail`
- `show_collation_in_column_detail`
- `table_engines`

Keep capabilities honest. A capability set to true tells the host it can expose workflows and call related methods.

## UI Form

`ui.form` defines connection form fields. Keep field ids aligned with the config shape consumed by `conn/open`.

Common field ids:

- `name`
- `host`
- `port`
- `username`
- `password`
- `database`
- `service_name`
- `sid`
- driver-specific extra params

Use locale keys when the driver ships `locales/`. Package verification should confirm `locales_dir` exists when declared.

## Compatibility Fallback

`dialect.compatible_database_type` is a host hint for external drivers. It lets the host fall back to built-in database plugin methods when the external driver returns not-supported for a wire method such as DDL building.

Example:

```json
{
  "dialect": {
    "identifier_quote": "\"",
    "compatible_database_type": "PostgreSQL"
  }
}
```

Use it when:

- The external database accepts SQL close to an existing built-in type.
- Host-generated DDL is better than the generic external fallback.
- Differences are acceptable for the host workflow being enabled.

Do not use it when:

- The target database has incompatible DDL semantics.
- The driver should implement its own SQL builder.
- Compatibility would hide a correctness bug.

Current intended values are existing host `DatabaseType` variants such as `MySQL`, `PostgreSQL`, `SQLite`, `MSSQL`, `Oracle`, and `ClickHouse`. Treat built-in DuckDB as temporary if the host is planning to remove it.
