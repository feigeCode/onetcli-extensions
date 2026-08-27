package dm

import (
	"strings"
	"testing"

	"navop-db-ipc-drivers/internal/dbipc"
)

func TestSpecBuildsDamengDSNFromNavopConfig(t *testing.T) {
	cfg, err := ConfigFromWire(map[string]any{
		"host":     "127.0.0.1",
		"port":     float64(5236),
		"username": "SYSDBA",
		"password": "sysDBA*00",
		"database": "SYSDBA",
		"extra_params": map[string]any{
			"autoCommit": "true",
			"schema":     "APP",
		},
	})
	if err != nil {
		t.Fatalf("ConfigFromWire returned error: %v", err)
	}

	dsn, err := Spec().BuildDSN(cfg)
	if err != nil {
		t.Fatalf("BuildDSN returned error: %v", err)
	}

	if !strings.HasPrefix(dsn, "dm://SYSDBA:sysDBA*00@127.0.0.1:5236?") {
		t.Fatalf("dsn prefix = %q", dsn)
	}
	for _, want := range []string{"autoCommit=true", "schema=APP"} {
		if !strings.Contains(dsn, want) {
			t.Fatalf("dsn %q does not contain %q", dsn, want)
		}
	}
}

func TestSpecBuildsDamengDSNWithDatabaseAsSchemaAndRawCredentials(t *testing.T) {
	cfg, err := ConfigFromWire(map[string]any{
		"host":     "2001:db8::10",
		"username": "SYS?DBA",
		"password": "p@ss?word",
		"database": "app",
	})
	if err != nil {
		t.Fatalf("ConfigFromWire returned error: %v", err)
	}

	dsn, err := Spec().BuildDSN(cfg)
	if err != nil {
		t.Fatalf("BuildDSN returned error: %v", err)
	}

	want := "dm://SYS?DBA:p@ss?word@[2001:db8::10]:5236?schema=app"
	if dsn != want {
		t.Fatalf("dsn = %q, want %q", dsn, want)
	}
}

func TestSpecBuildsDamengDSNWithoutHostManagedSSHOptions(t *testing.T) {
	cfg, err := ConfigFromWire(map[string]any{
		"host":     "127.0.0.1",
		"username": "SYSDBA",
		"password": "secret",
		"extra_params": map[string]any{
			"connect_timeout": "10",
			"ssh_auth_type":   "password",
			" SSH_PORT ":      22,
		},
	})
	if err != nil {
		t.Fatalf("ConfigFromWire returned error: %v", err)
	}

	dsn, err := Spec().BuildDSN(cfg)
	if err != nil {
		t.Fatalf("BuildDSN returned error: %v", err)
	}

	if !strings.Contains(dsn, "connect_timeout=10") {
		t.Fatalf("dsn %q does not contain connect_timeout=10", dsn)
	}
	if strings.Contains(strings.ToLower(dsn), "ssh_") {
		t.Fatalf("dsn leaked host-managed ssh options: %q", dsn)
	}
}

func TestSpecBuildsDamengMetadataSQLWithOwnerFilters(t *testing.T) {
	cfg := ConfigFromWireNoError(t, map[string]any{
		"host":     "127.0.0.1",
		"username": "SYSDBA",
	})
	spec := Spec()

	databasesSQL := spec.SchemaSQL.Databases(cfg)
	for _, want := range []string{"USER AS NAME FROM DUAL", "USERNAME AS NAME FROM ALL_USERS", "OWNER AS NAME FROM ALL_TABLES"} {
		if !strings.Contains(databasesSQL, want) {
			t.Fatalf("databases SQL %q does not contain %q", databasesSQL, want)
		}
	}

	objectsSQL := spec.SchemaSQL.Objects(cfg, "", "app's", nil)
	for _, want := range []string{"ALL_TABLES", "ALL_VIEWS", "ALL_TAB_COMMENTS", "OWNER = 'APP''S'"} {
		if !strings.Contains(objectsSQL, want) {
			t.Fatalf("objects SQL %q does not contain %q", objectsSQL, want)
		}
	}

	columnsSQL := spec.SchemaSQL.Columns(cfg, "", "app", "demo")
	for _, want := range []string{"ALL_TAB_COLUMNS", "ALL_COL_COMMENTS", "TABLE_NAME = 'DEMO'", "OWNER = 'APP'"} {
		if !strings.Contains(columnsSQL, want) {
			t.Fatalf("columns SQL %q does not contain %q", columnsSQL, want)
		}
	}

	indexesSQL := spec.SchemaSQL.Indexes(cfg, "", "app", "demo")
	for _, want := range []string{"ALL_INDEXES", "ALL_IND_COLUMNS", "TABLE_NAME = 'DEMO'", "TABLE_OWNER = 'APP'", "LISTAGG"} {
		if !strings.Contains(indexesSQL, want) {
			t.Fatalf("indexes SQL %q does not contain %q", indexesSQL, want)
		}
	}

	foreignKeysSQL := spec.SchemaSQL.ForeignKeys(cfg, "", "app", "demo")
	for _, want := range []string{"ALL_CONSTRAINTS", "ALL_CONS_COLUMNS", "CONSTRAINT_TYPE = 'R'", "TABLE_NAME = 'DEMO'", "OWNER = 'APP'", "DELETE_RULE"} {
		if !strings.Contains(foreignKeysSQL, want) {
			t.Fatalf("foreign keys SQL %q does not contain %q", foreignKeysSQL, want)
		}
	}

	viewsSQL := spec.SchemaSQL.Views(cfg, "", "app")
	for _, want := range []string{"ALL_VIEWS", "ALL_TAB_COMMENTS", "OWNER = 'APP'", "'NO'"} {
		if !strings.Contains(viewsSQL, want) {
			t.Fatalf("views SQL %q does not contain %q", viewsSQL, want)
		}
	}

	functionsSQL := spec.SchemaSQL.Functions(cfg, "", "app")
	for _, want := range []string{"ALL_OBJECTS", "ALL_PROCEDURES", "OBJECT_TYPE = 'FUNCTION'", "OWNER = 'APP'"} {
		if !strings.Contains(functionsSQL, want) {
			t.Fatalf("functions SQL %q does not contain %q", functionsSQL, want)
		}
	}

	viewSQL := spec.SchemaSQL.ViewDefinition(cfg, "", "app", "v_demo")
	for _, want := range []string{"ALL_VIEWS", "TEXT", "VIEW_NAME = 'V_DEMO'", "OWNER = 'APP'"} {
		if !strings.Contains(viewSQL, want) {
			t.Fatalf("view definition SQL %q does not contain %q", viewSQL, want)
		}
	}
}

func TestSpecBuildsDamengColumnsSQLFromQualifiedTable(t *testing.T) {
	cfg := ConfigFromWireNoError(t, map[string]any{
		"host":     "127.0.0.1",
		"username": "SYSDBA",
	})

	columnsSQL := Spec().SchemaSQL.Columns(cfg, "", "", "app.demo")
	for _, want := range []string{"TABLE_NAME = 'DEMO'", "OWNER = 'APP'"} {
		if !strings.Contains(columnsSQL, want) {
			t.Fatalf("columns SQL %q does not contain %q", columnsSQL, want)
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

func TestSpecBuildsDamengDumpDDL(t *testing.T) {
	cfg := ConfigFromWireNoError(t, map[string]any{
		"host":     "127.0.0.1",
		"username": "SYSDBA",
	})
	spec := Spec()
	if spec.SchemaSQL.DumpDDL == nil {
		t.Fatalf("dm Spec() must provide DumpDDL")
	}

	withoutOwner := spec.SchemaSQL.DumpDDL(cfg, "", "", "demo")
	if withoutOwner != "SELECT DBMS_METADATA.GET_DDL('TABLE', 'DEMO') FROM DUAL" {
		t.Fatalf("ownerless DumpDDL SQL = %q", withoutOwner)
	}

	withOwner := spec.SchemaSQL.DumpDDL(cfg, "APP", "APP", "demo")
	if withOwner != "SELECT DBMS_METADATA.GET_DDL('TABLE', 'DEMO', 'APP') FROM DUAL" {
		t.Fatalf("owner DumpDDL SQL = %q", withOwner)
	}

	qualified := spec.SchemaSQL.DumpDDL(cfg, "", "", "app.demo")
	if qualified != "SELECT DBMS_METADATA.GET_DDL('TABLE', 'DEMO', 'APP') FROM DUAL" {
		t.Fatalf("qualified DumpDDL SQL = %q", qualified)
	}

	escaped := spec.SchemaSQL.DumpDDL(cfg, "", "", "o'brien")
	if !strings.Contains(escaped, "'O''BRIEN'") {
		t.Fatalf("DumpDDL SQL %q does not escape a single quote", escaped)
	}
}
