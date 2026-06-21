package oceanbase

import (
	"context"
	"strings"
	"testing"

	"onetcli-db-ipc-drivers/internal/dbipc"
)

func TestSpecResolvesMySQLProtocolToMySQLWireDriver(t *testing.T) {
	cfg := ConfigFromWireNoError(t, map[string]any{
		"host":     "127.0.0.1",
		"port":     float64(2881),
		"username": "root@test",
		"password": "p@ss word",
		"database": "app",
		"protocol": "mysql",
	})

	connSpec, err := Spec().ResolveConnection(context.Background(), cfg)
	if err != nil {
		t.Fatalf("ResolveConnection returned error: %v", err)
	}

	if connSpec.DriverName != "mysql" {
		t.Fatalf("driver = %q, want mysql", connSpec.DriverName)
	}
	for _, want := range []string{"root@test:p@ss word@tcp(127.0.0.1:2881)/app", "parseTime=true"} {
		if !strings.Contains(connSpec.DSN, want) {
			t.Fatalf("dsn %q does not contain %q", connSpec.DSN, want)
		}
	}
	if connSpec.SchemaSQL.Databases == nil || !strings.Contains(connSpec.SchemaSQL.Databases(cfg), "INFORMATION_SCHEMA.SCHEMATA") {
		t.Fatalf("mysql protocol did not select MySQL metadata SQL")
	}
}

func TestSpecResolvesOracleProtocolOverOceanBaseMySQLWireToDedicatedDriver(t *testing.T) {
	oldProbe := probeOceanBaseMySQLWire
	probeOceanBaseMySQLWire = func(ctx context.Context, host string, port int) (bool, error) {
		if host != "ob.example.test" || port != 60014 {
			t.Fatalf("probe target = %s:%d", host, port)
		}
		return true, nil
	}
	defer func() { probeOceanBaseMySQLWire = oldProbe }()

	cfg := ConfigFromWireNoError(t, map[string]any{
		"host":         "ob.example.test",
		"port":         float64(60014),
		"username":     "sys@test",
		"password":     "oracle",
		"service_name": "ORCL",
		"protocol":     "oracle",
		"extra_params": map[string]any{
			"oracle_mysql_wire_driver": "oboracle-test",
		},
	})

	connSpec, err := Spec().ResolveConnection(context.Background(), cfg)
	if err != nil {
		t.Fatalf("ResolveConnection returned error: %v", err)
	}

	if connSpec.DriverName != "oboracle-test" {
		t.Fatalf("driver = %q, want oboracle-test", connSpec.DriverName)
	}
	if !strings.Contains(connSpec.DSN, "sys@test:oracle@tcp(ob.example.test:60014)/ORCL") {
		t.Fatalf("dsn = %q", connSpec.DSN)
	}
	if connSpec.SchemaSQL.Databases == nil || !strings.Contains(connSpec.SchemaSQL.Databases(cfg), "SYS_CONTEXT('USERENV', 'CON_NAME')") {
		t.Fatalf("oracle mysql-wire protocol did not select Oracle metadata SQL")
	}
	if connSpec.IdentifierQuoteLeft != `"` || connSpec.IdentifierQuoteRight != `"` {
		t.Fatalf("identifier quotes = %q/%q, want Oracle quotes", connSpec.IdentifierQuoteLeft, connSpec.IdentifierQuoteRight)
	}
}

func TestSpecResolvesOracleProtocolWithoutMySQLWireToGoOraDriver(t *testing.T) {
	oldProbe := probeOceanBaseMySQLWire
	probeOceanBaseMySQLWire = func(ctx context.Context, host string, port int) (bool, error) {
		return false, nil
	}
	defer func() { probeOceanBaseMySQLWire = oldProbe }()

	cfg := ConfigFromWireNoError(t, map[string]any{
		"host":         "obproxy.example.test",
		"port":         float64(1521),
		"username":     "system",
		"password":     "oracle",
		"service_name": "ORCL",
		"protocol":     "oracle",
	})

	connSpec, err := Spec().ResolveConnection(context.Background(), cfg)
	if err != nil {
		t.Fatalf("ResolveConnection returned error: %v", err)
	}

	if connSpec.DriverName != "oracle" {
		t.Fatalf("driver = %q, want oracle", connSpec.DriverName)
	}
	if !strings.HasPrefix(connSpec.DSN, "oracle://system:oracle@obproxy.example.test:1521/ORCL") {
		t.Fatalf("dsn = %q", connSpec.DSN)
	}
	if connSpec.SchemaSQL.Databases == nil || !strings.Contains(connSpec.SchemaSQL.Databases(cfg), "SYS_CONTEXT('USERENV', 'CON_NAME')") {
		t.Fatalf("oracle protocol did not select Oracle metadata SQL")
	}
	if connSpec.IdentifierQuoteLeft != `"` || connSpec.IdentifierQuoteRight != `"` {
		t.Fatalf("identifier quotes = %q/%q, want Oracle quotes", connSpec.IdentifierQuoteLeft, connSpec.IdentifierQuoteRight)
	}
}

func TestSpecBuildsOceanBaseMySQLMetadataSQL(t *testing.T) {
	cfg := ConfigFromWireNoError(t, map[string]any{
		"host":     "127.0.0.1",
		"username": "root@test",
		"database": "app",
	})
	spec := Spec()

	for name, sqlText := range map[string]string{
		"databases": spec.SchemaSQL.Databases(cfg),
		"schemas":   spec.SchemaSQL.Schemas(cfg, "app"),
		"objects":   spec.SchemaSQL.Objects(cfg, "app", "", []string{"table", "view"}),
		"columns":   spec.SchemaSQL.Columns(cfg, "app", "", "demo"),
		"indexes":   spec.SchemaSQL.Indexes(cfg, "app", "", "demo"),
		"views":     spec.SchemaSQL.Views(cfg, "app", ""),
	} {
		if !strings.Contains(sqlText, "INFORMATION_SCHEMA") {
			t.Fatalf("%s SQL %q does not query INFORMATION_SCHEMA", name, sqlText)
		}
	}
}

func ConfigFromWireNoError(t *testing.T, raw map[string]any) dbipc.Config {
	t.Helper()
	cfg, err := ConfigFromWire(raw)
	if err != nil {
		t.Fatalf("ConfigFromWire returned error: %v", err)
	}
	return cfg
}
