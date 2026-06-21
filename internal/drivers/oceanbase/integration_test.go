package oceanbase

import (
	"context"
	"database/sql"
	"fmt"
	"os"
	"strconv"
	"strings"
	"testing"
	"time"

	_ "github.com/go-sql-driver/mysql"
)

func TestLocalOceanBaseMySQLIntegration(t *testing.T) {
	if os.Getenv("ONETCLI_OCEANBASE_INTEGRATION") != "1" {
		t.Skip("set ONETCLI_OCEANBASE_INTEGRATION=1 to run against a local OceanBase instance")
	}

	port := 2881
	if raw := strings.TrimSpace(os.Getenv("ONETCLI_OCEANBASE_PORT")); raw != "" {
		parsed, err := strconv.Atoi(raw)
		if err != nil {
			t.Fatalf("ONETCLI_OCEANBASE_PORT = %q: %v", raw, err)
		}
		port = parsed
	}

	host := envOrDefault("ONETCLI_OCEANBASE_HOST", "127.0.0.1")
	username := envOrDefault("ONETCLI_OCEANBASE_USERNAME", "root@test")
	password := os.Getenv("ONETCLI_OCEANBASE_PASSWORD")
	database := envOrDefault("ONETCLI_OCEANBASE_DATABASE", "ai_app")

	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()
	ensureOceanBaseDatabase(t, ctx, host, port, username, password, database)

	cfg, err := ConfigFromWire(map[string]any{
		"host":     host,
		"port":     float64(port),
		"username": username,
		"password": password,
		"database": database,
		"protocol": "mysql",
	})
	if err != nil {
		t.Fatalf("ConfigFromWire returned error: %v", err)
	}

	connSpec, err := Spec().ResolveConnection(context.Background(), cfg)
	if err != nil {
		t.Fatalf("ResolveConnection returned error: %v", err)
	}
	if connSpec.DriverName != "mysql" {
		t.Fatalf("DriverName = %q, want mysql", connSpec.DriverName)
	}

	db, err := sql.Open(connSpec.DriverName, connSpec.DSN)
	if err != nil {
		t.Fatalf("sql.Open returned error: %v", err)
	}
	defer db.Close()
	if err := db.PingContext(ctx); err != nil {
		t.Fatalf("PingContext returned error: %v", err)
	}

	var compatibilityMode string
	if err := db.QueryRowContext(ctx, "SELECT @@ob_compatibility_mode").Scan(&compatibilityMode); err != nil {
		t.Fatalf("compatibility mode query returned error: %v", err)
	}
	if strings.ToUpper(compatibilityMode) != "MYSQL" {
		t.Fatalf("ob_compatibility_mode = %q, want MYSQL", compatibilityMode)
	}

	tableName := fmt.Sprintf("onetcli_ob_smoke_%d", time.Now().UnixNano())
	if _, err := db.ExecContext(ctx, "CREATE TABLE "+tableName+" (id BIGINT PRIMARY KEY, name VARCHAR(64))"); err != nil {
		t.Fatalf("CREATE TABLE returned error: %v", err)
	}
	defer db.ExecContext(context.Background(), "DROP TABLE IF EXISTS "+tableName)
	if _, err := db.ExecContext(ctx, "INSERT INTO "+tableName+" VALUES (?, ?)", int64(1), "ok"); err != nil {
		t.Fatalf("INSERT returned error: %v", err)
	}
	var got string
	if err := db.QueryRowContext(ctx, "SELECT name FROM "+tableName+" WHERE id = ?", int64(1)).Scan(&got); err != nil {
		t.Fatalf("SELECT returned error: %v", err)
	}
	if got != "ok" {
		t.Fatalf("selected name = %q, want ok", got)
	}

	rows, err := db.QueryContext(ctx, connSpec.SchemaSQL.Objects(cfg, cfg.Database, "", []string{"table"}))
	if err != nil {
		t.Fatalf("schema objects query returned error: %v", err)
	}
	defer rows.Close()
	found := false
	for rows.Next() {
		var name, kind, comment string
		if err := rows.Scan(&name, &kind, &comment); err != nil {
			t.Fatalf("schema objects scan returned error: %v", err)
		}
		if name == tableName && kind == "table" {
			found = true
		}
	}
	if err := rows.Err(); err != nil {
		t.Fatalf("schema objects rows returned error: %v", err)
	}
	if !found {
		t.Fatalf("schema objects query did not return created table %s", tableName)
	}
}

func envOrDefault(key, fallback string) string {
	if value := strings.TrimSpace(os.Getenv(key)); value != "" {
		return value
	}
	return fallback
}

func ensureOceanBaseDatabase(t *testing.T, ctx context.Context, host string, port int, username, password, database string) {
	t.Helper()
	cfg, err := ConfigFromWire(map[string]any{
		"host":     host,
		"port":     float64(port),
		"username": username,
		"password": password,
		"database": "oceanbase",
		"protocol": "mysql",
	})
	if err != nil {
		t.Fatalf("bootstrap ConfigFromWire returned error: %v", err)
	}
	connSpec, err := Spec().ResolveConnection(ctx, cfg)
	if err != nil {
		t.Fatalf("bootstrap ResolveConnection returned error: %v", err)
	}
	db, err := sql.Open(connSpec.DriverName, connSpec.DSN)
	if err != nil {
		t.Fatalf("bootstrap sql.Open returned error: %v", err)
	}
	defer db.Close()
	if err := db.PingContext(ctx); err != nil {
		t.Fatalf("bootstrap PingContext returned error: %v", err)
	}
	if _, err := db.ExecContext(ctx, "CREATE DATABASE IF NOT EXISTS "+quoteMySQLIdentifier(database)); err != nil {
		t.Fatalf("CREATE DATABASE returned error: %v", err)
	}
}

func quoteMySQLIdentifier(value string) string {
	return "`" + strings.ReplaceAll(value, "`", "``") + "`"
}
