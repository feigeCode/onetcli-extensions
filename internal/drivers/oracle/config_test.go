package oracle

import (
	"strings"
	"testing"

	"onetcli-db-ipc-drivers/internal/dbipc"
)

func TestSpecBuildsGoOraV2DSNFromOnetcliConfig(t *testing.T) {
	cfg, err := ConfigFromWire(map[string]any{
		"host":         "db.example.test",
		"port":         float64(1522),
		"username":     "app/user",
		"password":     "p@ss?word",
		"service_name": "orclpdb1",
		"extra_params": map[string]any{
			"TRACE FILE": "trace.log",
			"SERVER":     "dedicated",
		},
	})
	if err != nil {
		t.Fatalf("ConfigFromWire returned error: %v", err)
	}

	dsn, err := Spec().BuildDSN(cfg)
	if err != nil {
		t.Fatalf("BuildDSN returned error: %v", err)
	}

	if !strings.HasPrefix(dsn, "oracle://app%2Fuser:p%40ss%3Fword@db.example.test:1522/orclpdb1?") {
		t.Fatalf("dsn prefix = %q", dsn)
	}
	for _, want := range []string{"SERVER=dedicated", "TRACE+FILE=trace.log"} {
		if !strings.Contains(dsn, want) {
			t.Fatalf("dsn %q does not contain %q", dsn, want)
		}
	}
}

func TestSpecBuildsGoOraV2DSNFromSIDWhenServiceIsMissing(t *testing.T) {
	cfg, err := ConfigFromWire(map[string]any{
		"host":     "127.0.0.1",
		"username": "system",
		"password": "oracle",
		"sid":      "XE",
	})
	if err != nil {
		t.Fatalf("ConfigFromWire returned error: %v", err)
	}

	dsn, err := Spec().BuildDSN(cfg)
	if err != nil {
		t.Fatalf("BuildDSN returned error: %v", err)
	}

	want := "oracle://system:oracle@127.0.0.1:1521/XE"
	if dsn != want {
		t.Fatalf("dsn = %q, want %q", dsn, want)
	}
}

func TestSpecBuildsOracleMetadataSQLWithOwnerFilters(t *testing.T) {
	cfg := ConfigFromWireNoError(t, map[string]any{
		"host":         "127.0.0.1",
		"username":     "system",
		"password":     "oracle",
		"service_name": "orclpdb1",
	})
	spec := Spec()

	databasesSQL := spec.SchemaSQL.Databases(cfg)
	for _, want := range []string{"SYS_CONTEXT('USERENV', 'CON_NAME')", "FROM DUAL"} {
		if !strings.Contains(databasesSQL, want) {
			t.Fatalf("databases SQL %q does not contain %q", databasesSQL, want)
		}
	}

	schemasSQL := spec.SchemaSQL.Schemas(cfg, "ORCLPDB1")
	for _, want := range []string{"ALL_USERS", "USERNAME"} {
		if !strings.Contains(schemasSQL, want) {
			t.Fatalf("schemas SQL %q does not contain %q", schemasSQL, want)
		}
	}

	objectsSQL := spec.SchemaSQL.Objects(cfg, "", "app's", []string{"table", "view"})
	for _, want := range []string{"ALL_OBJECTS", "ALL_TAB_COMMENTS", "OWNER = 'APP''S'", "OBJECT_TYPE IN ('TABLE','VIEW')"} {
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
	for _, want := range []string{"ALL_CONSTRAINTS", "ALL_CONS_COLUMNS", "CONSTRAINT_TYPE = 'R'", "TABLE_NAME = 'DEMO'", "OWNER = 'APP'"} {
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

func TestSpecBuildsOracleColumnsSQLFromQualifiedTable(t *testing.T) {
	cfg := ConfigFromWireNoError(t, map[string]any{
		"host":         "127.0.0.1",
		"username":     "system",
		"password":     "oracle",
		"service_name": "orclpdb1",
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
