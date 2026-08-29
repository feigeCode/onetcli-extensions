package oracle

import (
	"strings"
	"testing"

	"navop-db-ipc-drivers/internal/dbipc"
)

func TestSpecExposesOracleGoExternalIDWhileUsingGoOraSQLDriver(t *testing.T) {
	spec := Spec()

	if spec.ID != "oracle-go" {
		t.Fatalf("spec.ID = %q, want oracle-go", spec.ID)
	}
	if spec.Name != "Oracle Go" {
		t.Fatalf("spec.Name = %q, want Oracle Go", spec.Name)
	}
	if spec.SQLDriverName != "oracle" {
		t.Fatalf("spec.SQLDriverName = %q, want oracle", spec.SQLDriverName)
	}
}

func TestSpecBuildsGoOraV2DSNFromNavopConfig(t *testing.T) {
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

func TestSpecBuildsGoOraV2DSNWithoutHostManagedSSHOptions(t *testing.T) {
	cfg, err := ConfigFromWire(map[string]any{
		"host":         "10.2.4.53",
		"port":         float64(1521),
		"username":     "COMI_SERVER2112",
		"password":     "oracle",
		"service_name": "ORCL",
		"extra_params": map[string]any{
			"SERVER":             "dedicated",
			"ssh_tunnel_enabled": false,
			"ssh_port":           22,
			" SSH_AUTH_TYPE ":    "password",
		},
	})
	if err != nil {
		t.Fatalf("ConfigFromWire returned error: %v", err)
	}

	dsn, err := Spec().BuildDSN(cfg)
	if err != nil {
		t.Fatalf("BuildDSN returned error: %v", err)
	}

	if strings.Contains(strings.ToLower(dsn), "ssh_") {
		t.Fatalf("dsn leaked host-managed ssh options: %q", dsn)
	}
	if !strings.Contains(dsn, "SERVER=dedicated") {
		t.Fatalf("dsn %q does not contain SERVER=dedicated", dsn)
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

func TestSpecDoesNotUseGenericDatabaseAsOracleService(t *testing.T) {
	cfg, err := ConfigFromWire(map[string]any{
		"host":     "127.0.0.1",
		"username": "system",
		"password": "oracle",
		"database": "ORCL",
	})
	if err != nil {
		t.Fatalf("ConfigFromWire returned error: %v", err)
	}

	_, err = Spec().BuildDSN(cfg)
	if err == nil {
		t.Fatalf("BuildDSN returned nil error for database-only Oracle config")
	}
	if !strings.Contains(err.Error(), "service_name or sid") {
		t.Fatalf("BuildDSN error = %q, want service_name or sid", err.Error())
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

func TestSpecBuildsOracleSchemasSQLWithDistinctColumnAliases(t *testing.T) {
	cfg := ConfigFromWireNoError(t, map[string]any{
		"host":         "127.0.0.1",
		"username":     "system",
		"password":     "oracle",
		"service_name": "orclpdb1",
	})

	schemasSQL := Spec().SchemaSQL.Schemas(cfg, "ORCLPDB1")
	for _, want := range []string{"USERNAME AS NAME", "USERNAME AS OWNER", "ORDER BY 1"} {
		if !strings.Contains(schemasSQL, want) {
			t.Fatalf("schemas SQL %q does not contain %q", schemasSQL, want)
		}
	}
	if strings.Contains(schemasSQL, "SELECT USERNAME, USERNAME") {
		t.Fatalf("schemas SQL %q uses duplicate unaliased USERNAME columns", schemasSQL)
	}
}

func TestSpecBuildsOracleRoutineTriggerAndSequenceSQL(t *testing.T) {
	cfg := ConfigFromWireNoError(t, map[string]any{
		"host":         "127.0.0.1",
		"username":     "system",
		"password":     "oracle",
		"service_name": "orclpdb1",
	})
	spec := Spec()

	proceduresSQL := spec.SchemaSQL.Procedures(cfg, "", "app")
	for _, want := range []string{"ALL_OBJECTS", "OBJECT_TYPE = 'PROCEDURE'", "OWNER = 'APP'"} {
		if !strings.Contains(proceduresSQL, want) {
			t.Fatalf("procedures SQL %q does not contain %q", proceduresSQL, want)
		}
	}

	triggersSQL := spec.SchemaSQL.Triggers(cfg, "", "app", "demo")
	for _, want := range []string{"ALL_TRIGGERS", "OWNER = 'APP'", "TABLE_NAME = 'DEMO'"} {
		if !strings.Contains(triggersSQL, want) {
			t.Fatalf("triggers SQL %q does not contain %q", triggersSQL, want)
		}
	}

	sequencesSQL := spec.SchemaSQL.Sequences(cfg, "", "app")
	for _, want := range []string{"ALL_SEQUENCES", "SEQUENCE_OWNER = 'APP'", "INCREMENT_BY"} {
		if !strings.Contains(sequencesSQL, want) {
			t.Fatalf("sequences SQL %q does not contain %q", sequencesSQL, want)
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

func TestSpecBuildsOracleDumpDDL(t *testing.T) {
	cfg := ConfigFromWireNoError(t, map[string]any{
		"host":         "127.0.0.1",
		"username":     "app",
		"service_name": "orclpdb1",
	})
	spec := Spec()
	if spec.SchemaSQL.DumpDDL == nil {
		t.Fatalf("oracle Spec() must provide DumpDDL")
	}

	withoutOwner := spec.SchemaSQL.DumpDDL(cfg, "", "", "demo")
	if withoutOwner != "SELECT DBMS_METADATA.GET_DDL('TABLE', 'DEMO') FROM DUAL" {
		t.Fatalf("ownerless DumpDDL SQL = %q", withoutOwner)
	}

	withOwner := spec.SchemaSQL.DumpDDL(cfg, "app", "app", "demo")
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

func TestSpecMapsFormTimeoutsToGoOraOptions(t *testing.T) {
	cfg, err := ConfigFromWire(map[string]any{
		"host":         "127.0.0.1",
		"username":     "system",
		"password":     "oracle",
		"service_name": "orclpdb1",
		"extra_params": map[string]any{
			"connect_timeout": "30",
			"read_timeout":    "28800",
		},
	})
	if err != nil {
		t.Fatalf("ConfigFromWire returned error: %v", err)
	}

	dsn, err := Spec().BuildDSN(cfg)
	if err != nil {
		t.Fatalf("BuildDSN returned error: %v", err)
	}

	if !strings.Contains(dsn, "CONNECT+TIMEOUT=30") {
		t.Fatalf("dsn %q does not map connect_timeout to CONNECT TIMEOUT", dsn)
	}
	if !strings.Contains(dsn, "READ+TIMEOUT=28800") {
		t.Fatalf("dsn %q does not map read_timeout to READ TIMEOUT", dsn)
	}
	if strings.Contains(dsn, "connect_timeout=") {
		t.Fatalf("dsn %q leaks the unsupported connect_timeout URL option", dsn)
	}
	if strings.Contains(dsn, "read_timeout=") {
		t.Fatalf("dsn %q leaks the unsupported read_timeout URL option", dsn)
	}
}

func TestSpecDropsHostManagedParamsFromDSN(t *testing.T) {
	cfg, err := ConfigFromWire(map[string]any{
		"host":         "127.0.0.1",
		"username":     "system",
		"password":     "oracle",
		"service_name": "orclpdb1",
		"extra_params": map[string]any{
			"external_driver_id":    "oracle-go",
			"default_schema":        "APP",
			"schema_filter_mode":    "auto",
			"schema_filter_include": "",
			"schema_filter_exclude": "",
			"ssh_target_host":       "db.internal",
			"SERVER":                "dedicated",
		},
	})
	if err != nil {
		t.Fatalf("ConfigFromWire returned error: %v", err)
	}

	dsn, err := Spec().BuildDSN(cfg)
	if err != nil {
		t.Fatalf("BuildDSN returned error: %v", err)
	}

	for _, leaked := range []string{
		"external_driver_id",
		"default_schema",
		"schema_filter",
		"ssh_",
	} {
		if strings.Contains(strings.ToLower(dsn), leaked) {
			t.Fatalf("dsn %q leaked host-managed param %q", dsn, leaked)
		}
	}
	if !strings.Contains(dsn, "SERVER=dedicated") {
		t.Fatalf("dsn %q dropped a driver-supported option", dsn)
	}
}

func TestSpecSkipsUnparseableTimeoutWithoutFailing(t *testing.T) {
	cfg, err := ConfigFromWire(map[string]any{
		"host":         "127.0.0.1",
		"username":     "system",
		"password":     "oracle",
		"service_name": "orclpdb1",
		"extra_params": map[string]any{
			"connect_timeout": "abc",
		},
	})
	if err != nil {
		t.Fatalf("ConfigFromWire returned error: %v", err)
	}

	dsn, err := Spec().BuildDSN(cfg)
	if err != nil {
		t.Fatalf("BuildDSN returned error: %v", err)
	}
	if strings.Contains(dsn, "TIMEOUT") {
		t.Fatalf("dsn %q should skip the unparseable timeout", dsn)
	}
	if strings.Contains(dsn, "connect_timeout") {
		t.Fatalf("dsn %q leaked the unsupported connect_timeout URL option", dsn)
	}
}
