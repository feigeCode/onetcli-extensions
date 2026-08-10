# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- OB20 flag `OB20FlagNewExtraInfo` is now only set when the server confirms
  `OBCapNewExtraInfo` (1<<15) support during session state change.  Previously it
  was unconditionally set on every OB20 frame, breaking connections through older
  OBProxy versions that reject unknown flags.

## [0.3.0] - 2026-05-25

## [0.2.1] - 2026-05-03

### Added

- TLS support with custom CA certificate (`--tls`, `--tls-ca` flags and `tls.ca` DSN parameter).
- `--mysql-mode` flag for MySQL tenant testing.
- `cmd/obping --full-test`: 16-test comprehensive integration suite for regression testing.
- `isMissingTableError` now handles MySQL error 1051 ("Unknown table").

### Fixed

- TLS `ServerName` now auto-detected from the dial hostname (not resolved IP), enabling CA cert verification through OBProxy.
- Oracle `SYSDATE` replaced with `CURRENT_TIMESTAMP` for cross-mode compatibility.

## [0.1.0] - 2026-04-30

First tagged release for downstream integration (e.g. `go get` / `xixi`).

### Added

- Pure Go `database/sql` driver (`oceanbase` / `oboracle`) for OceanBase **Oracle** tenants over the MySQL-compatible protocol: handshake with Oracle-mode capability and OB client attributes, `mysql_native_password`, `COM_QUERY`, text result sets, streaming rows, OK/ERR parsing.
- DSN parsing: URL form, legacy `tcp()`, opaque `oceanbase:user:pass@host:port/db`, query options (`timeout`, `connect timeout`, `trace`, `preset`, `cap.add` / `cap.drop`, `attr.*`, `init_sql`).
- Client-side `?` parameter interpolation (with quote/comment-aware placeholder counting) for `Query`/`Exec`/`Prepare` paths without server-side binary PS.
- Transactions: implicit Oracle-style session with explicit `commit` / `rollback` from `Tx`.
- Connection pool hooks: `Ping`, `ResetSession`, `IsValid`, `ErrBadConn` mapping for recoverable connection errors.
- `cmd/obping`: scalar/query modes, `-tx-test`, `-exec-test`, `-param-test`, `-pool-test`, `-trace`, preset probing.
- Documentation: `README.md`, `README.zh-CN.md`, `docs/protocol-notes.md`, contributing and security templates.

### Known limitations

- No TLS yet; conservative text-protocol type mapping; no full server-side prepared statements.

[0.1.0]: https://github.com/helingjun/OBConnector-go/releases/tag/v0.1.0
