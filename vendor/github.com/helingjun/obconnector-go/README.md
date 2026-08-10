# obconnector-go

[中文文档](README.zh-CN.md)

Clean-room OceanBase driver experiments for Go's `database/sql`.

The first milestone is a pure-Go PoC that can connect to an OceanBase Oracle tenant and run:

```sql
select 1 from dual
```

This repository intentionally does not depend on OBCI, Oracle Instant Client, LibOBClient, cgo, or dynamic OceanBase client libraries.

## Current State

- Registers `database/sql` driver names `oceanbase` and `oboracle`.
- Implements a minimal MySQL-compatible wire path with OceanBase Oracle tenant handshake extensions.
- Supports `mysql_native_password`, `COM_QUERY`, streaming text rows, server errors, basic transactions, `Prepare` compatibility, client-side `?` parameter interpolation, connection-pool lifecycle hooks, and basic column type metadata.
- Provides `cmd/obping` for connection experiments and regression testing.
- Records protocol observations and open questions in `docs/protocol-notes.md`.

The implementation is not production-ready. It is designed to support packet captures and connection experiments without copying LGPL/MariaDB/OBConnector-C source code.

## obping — Connection & Regression Testing Tool

`cmd/obping` is a CLI tool for testing connections and running integration tests against OceanBase tenants (both Oracle and MySQL mode).

### Quick Start

```bash
go run ./cmd/obping \
  -host 127.0.0.1 \
  -port 2881 \
  -user '<user@tenant#cluster>' \
  -password '<password>' \
  -trace
```

Or pass a DSN directly:

```bash
go run ./cmd/obping \
  -dsn 'oceanbase://<user>:<password>@<host>:<port>/?timeout=5s'
```

### All Flags

| Flag | Default | Description |
|------|---------|-------------|
| `-dsn` | `""` | Full DSN (overrides individual flags) |
| `-host` | `127.0.0.1` | OceanBase host or OBProxy host |
| `-port` | `2881` | OceanBase port or OBProxy port |
| `-user` | `""` | OceanBase user, often `user@tenant#cluster` |
| `-password` | `""` | OceanBase password |
| `-database` | `""` | Database/schema name |
| `-timeout` | `10s` | Connect and query timeout |
| `-tls` | `false` | Enable TLS encryption |
| `-tls-ca` | `""` | Path to CA certificate file for TLS |
| `-oracle-mode` | `false` | Force Oracle mode |
| `-mysql-mode` | `false` | Force MySQL mode |
| `-trace` | `false` | Print handshake and query trace to stderr |
| `-preset` | `""` | Client identity preset (`default`, `oboracle`, `obclient`, ...) |
| `-ob20` | `false` | Enable OB 2.0 protocol encapsulation |
| `-probe-presets` | `false` | Try all presets until one succeeds |
| `-query` | `"select 1 from dual"` | Query to execute |
| `-max-rows` | `20` | Maximum rows to print |
| `-attr` | — | Connection attribute `key=value` (repeatable) |
| `-init` | — | SQL to run after auth (repeatable) |
| `-cap-add` | `""` | Capability bits to force on (`0x200000`) |
| `-cap-drop` | `""` | Capability bits to force off (`0x100000`) |
| `-collation` | `""` | Handshake collation ID (`45`) |
| `-exec-table` | `""` | Table name for exec-test |

#### Test Flags

| Flag | Description |
|------|-------------|
| `-tx-test` | Basic `BEGIN / COMMIT / ROLLBACK` transaction test |
| `-exec-test` | DDL/DML smoke test (create, insert, update, delete) |
| `-param-test` | Parameterized `?` query/exec test |
| `-pool-test` | `database/sql` connection pool lifecycle test |
| `-bulk-test` | BulkInsert helper test |
| `-full-test` | **Comprehensive integration test (all of the above combined)** |

### DSN

URL-style DSN (recommended):

```text
oceanbase://<user>:<password>@<host>:<port>/<database>?<params>
```

Opaque DSN (legacy):

```text
oceanbase:<user>:<password>@<host>:<port>/<database>?<params>
```

#### DSN Parameters

| Parameter | Values | Description |
|-----------|--------|-------------|
| `timeout` | Duration (`5s`, `10s`) | Connection and read/write timeout |
| `trace` | `true`/`false` | Print handshake and query details |
| `tls` | `true`/`skip-verify`/`false` | Enable TLS |
| `tls.ca` | File path | Path to CA certificate for TLS |
| `oracleMode` | `true`/`false`/`auto` | Force Oracle/MySQL mode or auto-detect |
| `ob20` | `true`/`false` | Enable OB 2.0 protocol encapsulation |
| `preset` | `default`/`oboracle`/`obclient`/... | Client identity preset |
| `collation` | `uint8` | Handshake collation byte |
| `cap.add` | `uint32` | Force capability bits on |
| `cap.drop` | `uint32` | Force capability bits off |
| `attr.<key>` | String | Connection attribute |
| `init` | SQL | SQL to run after authentication (repeatable) |

### Custom Query

```bash
go run ./cmd/obping \
  -dsn 'oceanbase://<user>:<password>@<host>:<port>/?timeout=5s' \
  -query 'select 1 as one from dual' \
  -max-rows 20
```

### DDL/DML Test

```bash
go run ./cmd/obping \
  -dsn 'oceanbase://<user>:<password>@<host>:<port>/?timeout=5s' \
  -exec-test
```

### Parameterized Query Test

```bash
go run ./cmd/obping \
  -dsn 'oceanbase://<user>:<password>@<host>:<port>/?timeout=5s' \
  -param-test
```

### Connection Pool Test

```bash
go run ./cmd/obping \
  -dsn 'oceanbase://<user>:<password>@<host>:<port>/?timeout=5s' \
  -pool-test \
  -trace
```

### Oracle Tenant — Full Integration Test

```bash
go run ./cmd/obping \
  -host <host> \
  -port 1521 \
  -user '<user>@<tenant>#<cluster>' \
  -password '<password>' \
  -oracle-mode \
  -full-test
```

The `-oracle-mode` flag forces Oracle mode when the server version string does not contain "oracle" (common with OBProxy).

### MySQL Tenant — Full Integration Test (TLS)

```bash
go run ./cmd/obping \
  -host <host> \
  -port 3306 \
  -user '<user>' \
  -password '<password>' \
  -database <database> \
  -tls \
  -tls-ca /path/to/ca.pem \
  -mysql-mode \
  -full-test
```

The `-tls` flag enables TLS, and `-tls-ca` specifies the CA certificate path for verifying the server certificate.

### Full-Test Reference

The `-full-test` flag runs 16 test categories in sequence:

| # | Test | Description |
|---|------|-------------|
| 1 | Ping | Basic connectivity check |
| 2 | Numeric | `SELECT 1.5 + 2.3 FROM DUAL` |
| 3 | String | `SELECT UPPER('hello world') FROM DUAL` |
| 4 | Timestamp | `SELECT CURRENT_TIMESTAMP FROM DUAL` |
| 5 | DDL | `CREATE TABLE` with INTEGER/VARCHAR/TIMESTAMP columns |
| 6 | INSERT | `INSERT` 3 rows with `?` placeholders |
| 7 | SELECT | Multi-row `SELECT ... ORDER BY` |
| 8 | Param Query | `SELECT ... WHERE id = ?` (text protocol interpolation) |
| 9 | Prepared `?` | Server-side prepared statement with `?` (Oracle mode only) |
| 10 | Prepared `:1` | Server-side prepared statement with `:1` (Oracle mode only) |
| 11 | UPDATE | `UPDATE ... WHERE id = ?` |
| 12 | DELETE | `DELETE ... WHERE id = ?` |
| 13 | NULL | `INSERT/SELECT` NULL values |
| 14 | Rollback | Transaction `UPDATE + ROLLBACK`, verify no change |
| 15 | Commit | Transaction `INSERT + COMMIT`, verify persistence |
| 16 | Cleanup | DROP test table |

> **Note:** Tests 9-10 (prepared statements) use `COM_STMT_EXECUTE` which is not supported by some OBProxy versions (notably MySQL-mode OBProxy). In Oracle mode through OBProxy, prepared statements work correctly. Failures in tests 9-10 are non-fatal — the connection is isolated and cleaned up before continuing.

### Using obping for Regression Testing

The `-full-test` flag is designed for regression testing. After any code changes, run:

**Oracle tenant:**
```bash
obping \
  -host <host> -port 1521 \
  -user '<user>' -password '<password>' \
  -oracle-mode \
  -full-test
```

**MySQL tenant:**
```bash
obping \
  -host <host> -port 3306 \
  -user '<user>' -password '<password>' \
  -database <database> \
  -tls -tls-ca /path/to/ca.pem \
  -mysql-mode \
  -full-test
```

Expected result: `=== ALL TESTS PASSED ===`

### Security

> **Never commit real database credentials, hostnames, or certificates to the repository.**
> Always use placeholder values (`<host>`, `<user>`, `<password>`, etc.) in documentation, issues, and PRs.

## Driver Usage in Go Code

```go
package main

import (
	"database/sql"
	_ "github.com/helingjun/obconnector-go"
)

func main() {
	db, err := sql.Open("oceanbase", "oceanbase://<user>:<password>@<host>:<port>/?timeout=5s")
	if err != nil {
		panic(err)
	}
	defer db.Close()

	var one string
	if err := db.QueryRow("select 1 from dual").Scan(&one); err != nil {
		panic(err)
	}
}
```

## Parameterized Query Support

Current `?` parameter support is a client-side interpolation layer, not server-side prepared statements.

Supported parameter types:

- `nil`
- `string`
- `int64`
- `float64`
- `bool`
- `[]byte`
- `time.Time`

The interpolation logic correctly handles string literals, quoted identifiers, line comments, and block comments.

## License

Apache-2.0. See `LICENSE`.
