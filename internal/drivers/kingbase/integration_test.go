package kingbase

import (
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"os"
	"strconv"
	"strings"
	"testing"
	"time"

	"navop-db-ipc-drivers/internal/dbipc"
	"navop-db-ipc-drivers/internal/ipc"
)

// TestLocalPostgresIntegration exercises the sys_*/pg_* catalog adaptation
// against a real PostgreSQL-compatible backend (e.g. local Homebrew
// PostgreSQL). KingbaseES V8R6 and newer expose sys_* catalog relations while
// PostgreSQL and older KingbaseES expose pg_*, which is exactly the failure
// mode reported as `关系 "sys_database" 不存在`.
func TestLocalPostgresIntegration(t *testing.T) {
	if os.Getenv("ONETCLI_KINGBASE_INTEGRATION") != "1" {
		t.Skip("set ONETCLI_KINGBASE_INTEGRATION=1 to run against a local PostgreSQL/Kingbase instance")
	}

	port := 5432
	if raw := strings.TrimSpace(os.Getenv("ONETCLI_KINGBASE_PORT")); raw != "" {
		parsed, err := strconv.Atoi(raw)
		if err != nil {
			t.Fatalf("ONETCLI_KINGBASE_PORT = %q: %v", raw, err)
		}
		port = parsed
	}

	host := envOrDefault("ONETCLI_KINGBASE_HOST", "127.0.0.1")
	username := envOrDefault("ONETCLI_KINGBASE_USERNAME", "hufei")
	password := os.Getenv("ONETCLI_KINGBASE_PASSWORD")
	database := envOrDefault("ONETCLI_KINGBASE_DATABASE", "postgres")

	cfg, err := ConfigFromWire(map[string]any{
		"host":     host,
		"port":     float64(port),
		"username": username,
		"password": password,
		"database": database,
	})
	if err != nil {
		t.Fatalf("ConfigFromWire returned error: %v", err)
	}

	dsn, err := Spec().BuildDSN(cfg)
	if err != nil {
		t.Fatalf("BuildDSN returned error: %v", err)
	}

	db, err := sql.Open("kingbase", dsn)
	if err != nil {
		t.Fatalf("sql.Open returned error: %v", err)
	}
	defer db.Close()

	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()
	if err := db.PingContext(ctx); err != nil {
		t.Fatalf("PingContext returned error: %v", err)
	}

	adapted, err := Spec().AdaptSchemaSQL(ctx, db, Spec().SchemaSQL)
	if err != nil {
		t.Fatalf("AdaptSchemaSQL returned error: %v", err)
	}
	databasesSQL := adapted.Databases(cfg)
	if !strings.Contains(databasesSQL, "pg_database") {
		t.Fatalf("adapted databases SQL uses sys_* catalogs, got %q", databasesSQL)
	}

	rows, err := db.QueryContext(ctx, databasesSQL)
	if err != nil {
		t.Fatalf("databases query returned error: %v (SQL: %s)", err, databasesSQL)
	}
	defer rows.Close()
	count := 0
	for rows.Next() {
		count++
	}
	if err := rows.Err(); err != nil {
		t.Fatalf("databases query iteration returned error: %v", err)
	}
	if count == 0 {
		t.Fatalf("databases query returned no rows for %q", databasesSQL)
	}
}

// TestLocalPostgresServerSchemaDatabasesIntegration drives the real IPC server
// the same way the Navop host does: init, conn/open, then schema/databases.
// Before the sys_*/pg_* adaptation this reproduced the reported
// `关系 "sys_database" 不存在` SQLSTATE 42P01 failure on any backend without
// KingbaseES sys_* catalogs.
func TestLocalPostgresServerSchemaDatabasesIntegration(t *testing.T) {
	if os.Getenv("ONETCLI_KINGBASE_INTEGRATION") != "1" {
		t.Skip("set ONETCLI_KINGBASE_INTEGRATION=1 to run against a local PostgreSQL/Kingbase instance")
	}

	port := 5432
	if raw := strings.TrimSpace(os.Getenv("ONETCLI_KINGBASE_PORT")); raw != "" {
		parsed, err := strconv.Atoi(raw)
		if err != nil {
			t.Fatalf("ONETCLI_KINGBASE_PORT = %q: %v", raw, err)
		}
		port = parsed
	}
	host := envOrDefault("ONETCLI_KINGBASE_HOST", "127.0.0.1")
	username := envOrDefault("ONETCLI_KINGBASE_USERNAME", "hufei")
	password := os.Getenv("ONETCLI_KINGBASE_PASSWORD")
	database := envOrDefault("ONETCLI_KINGBASE_DATABASE", "postgres")

	server := dbipc.NewServer(Spec(), nil)

	initResp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`1`),
		Method:  "init",
		Params:  json.RawMessage(`{"host_version":"0.10.0","api_offered":{"database":"1.0"},"instance_id":"integration","config":{}}`),
	})
	if initResp.Error != nil {
		t.Fatalf("init returned error: %#v", initResp.Error)
	}

	openResp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`2`),
		Method:  "conn/open",
		Params:  []byte(`{"driver_id":"kingbase","config":{"host":"` + host + `","port":` + strconv.Itoa(port) + `,"username":"` + username + `","password":"` + password + `","database":"` + database + `"}}`),
	})
	if openResp.Error != nil {
		t.Fatalf("conn/open returned error: %#v (SQLSTATE marker %d)", openResp.Error, openResp.Error.Code)
	}
	var opened struct {
		ConnID uint64 `json:"conn_id"`
	}
	if err := json.Unmarshal(openResp.Result, &opened); err != nil {
		t.Fatalf("conn/open result is not JSON: %v; raw=%s", err, openResp.Result)
	}
	if opened.ConnID == 0 {
		t.Fatalf("conn/open returned conn_id 0")
	}

	dbResp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`3`),
		Method:  "schema/databases",
		Params:  []byte(fmt.Sprintf(`{"conn_id":%d}`, opened.ConnID)),
	})
	if dbResp.Error != nil {
		t.Fatalf("schema/databases returned error: %#v (SQLSTATE marker %d)", dbResp.Error, dbResp.Error.Code)
	}
	var databases []map[string]any
	if err := json.Unmarshal(dbResp.Result, &databases); err != nil {
		t.Fatalf("schema/databases result is not JSON: %v; raw=%s", err, dbResp.Result)
	}
	if len(databases) == 0 {
		t.Fatalf("schema/databases returned no rows")
	}
}

func envOrDefault(key, fallback string) string {
	if v := strings.TrimSpace(os.Getenv(key)); v != "" {
		return v
	}
	return fallback
}
