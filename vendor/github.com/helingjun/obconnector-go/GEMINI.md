# OBConnector-go: Project Context & Guidelines

`obconnector-go` is a pure-Go, clean-room implementation of an OceanBase database driver for Go's `database/sql` package. It specifically targets support for both OceanBase MySQL and Oracle tenants without requiring C libraries (cgo-free).

## Project Overview

- **Purpose:** Provide a lightweight, pure-Go driver for OceanBase that correctly handles Oracle tenant handshake extensions.
- **Key Technologies:** Go 1.22+, `database/sql/driver`.
- **Architecture:**
    - `driver.go`: Implements the `sql.Register` logic and `driver.Driver` interface.
    - `conn.go`, `stmt.go`, `rows.go`, `tx.go`: Core `database/sql/driver` implementations.
    - `internal/protocol/`: Low-level packet and wire protocol handling.
    - `config.go`: DSN parsing logic supporting `oceanbase://` (URL), `oceanbase:` (Opaque), and legacy MySQL formats.
    - `cmd/obping/`: A CLI tool for connectivity experiments and protocol validation.

## Building and Running

### Prerequisites
- Go 1.22 or higher.

### Key Commands
- **Build the driver:** `go build ./...`
- **Run tests:** `go test -v ./...`
- **Run the ping tool:**
  ```bash
  go run ./cmd/obping -dsn 'oceanbase://user:password@127.0.0.1:2881/db?timeout=5s'
  ```
- **Trace protocol:** Add `-trace` to the ping tool or `trace=true` to the DSN to see handshake and query details.

## Development Conventions

### Clean-Room Implementation
This project follows a strict clean-room approach. **DO NOT** copy source code from other drivers such as `go-sql-driver/mysql`, MariaDB Connector/C, or OBConnector-C. Refer to `docs/protocol-notes.md` for the clean-room boundary rules.

### Protocol Documentation
All new protocol discoveries, packet shapes, and experiment results must be recorded in `docs/protocol-notes.md`. This file serves as the working notebook for the project.

### DSN Formats
The driver supports three DSN formats:
1. **URL:** `oceanbase://user:password@host:port/db?param=value`
2. **Opaque:** `oceanbase:user:password@host:port/db?param=value`
3. **Legacy:** `user:password@tcp(host:port)/db?param=value`

### Testing
- **Unit Tests:** Always add corresponding `*_test.go` files for new features or bug fixes.
- **Validation:** Use `cmd/obping` with flags like `-tx-test`, `-exec-test`, and `-param-test` to validate driver behavior against a live OceanBase instance.

## Key Files
- `driver.go`: Entry point for driver registration.
- `config.go`: Configuration and DSN parsing.
- `docs/protocol-notes.md`: Protocol implementation details and clean-room rules.
- `cmd/obping/main.go`: Primary tool for testing and experimentation.
