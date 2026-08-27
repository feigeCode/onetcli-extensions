package kingbase

import (
	"context"
	"database/sql"
	"strings"
	"testing"
)

func TestSpecBuildsKingbaseConnInfoFromNavopConfig(t *testing.T) {
	cfg, err := ConfigFromWire(map[string]any{
		"host":     "127.0.0.1",
		"port":     float64(54321),
		"username": "system",
		"password": "123456",
		"database": "TEST",
		"extra_params": map[string]any{
			"sslmode":              "disable",
			"connect_timeout":      "10",
			"application_name":     "navop",
			"target_session_attrs": "read-write",
		},
	})
	if err != nil {
		t.Fatalf("ConfigFromWire returned error: %v", err)
	}

	connInfo, err := Spec().BuildDSN(cfg)
	if err != nil {
		t.Fatalf("BuildDSN returned error: %v", err)
	}

	for _, want := range []string{
		"host=127.0.0.1",
		"port=54321",
		"user=system",
		"password=123456",
		"dbname=TEST",
		"sslmode=disable",
		"connect_timeout=10",
		"application_name=navop",
		"target_session_attrs=read-write",
	} {
		if !strings.Contains(connInfo, want) {
			t.Fatalf("connInfo %q does not contain %q", connInfo, want)
		}
	}
}

func TestSpecBuildsKingbaseConnInfoWithDefaultSSLModeAndQuotedValues(t *testing.T) {
	cfg, err := ConfigFromWire(map[string]any{
		"host":     "127.0.0.1",
		"username": "system user",
		"password": "pa ss'\\word",
		"database": "TEST",
	})
	if err != nil {
		t.Fatalf("ConfigFromWire returned error: %v", err)
	}

	connInfo, err := Spec().BuildDSN(cfg)
	if err != nil {
		t.Fatalf("BuildDSN returned error: %v", err)
	}

	for _, want := range []string{
		"port=54321",
		"sslmode=disable",
		"user='system user'",
		"password='pa ss\\'\\\\word'",
	} {
		if !strings.Contains(connInfo, want) {
			t.Fatalf("connInfo %q does not contain %q", connInfo, want)
		}
	}
}

func TestSpecBuildsKingbaseConnInfoWithoutHostManagedSSHOptions(t *testing.T) {
	cfg, err := ConfigFromWire(map[string]any{
		"host":     "127.0.0.1",
		"username": "system",
		"password": "secret",
		"database": "TEST",
		"extra_params": map[string]any{
			"application_name": "navop",
			"ssh_auth_type":    "password",
			" SSH_PORT ":       22,
		},
	})
	if err != nil {
		t.Fatalf("ConfigFromWire returned error: %v", err)
	}

	connInfo, err := Spec().BuildDSN(cfg)
	if err != nil {
		t.Fatalf("BuildDSN returned error: %v", err)
	}

	if !strings.Contains(connInfo, "application_name=navop") {
		t.Fatalf("connInfo %q does not contain application_name=navop", connInfo)
	}
	if strings.Contains(strings.ToLower(connInfo), "ssh_") {
		t.Fatalf("connInfo leaked host-managed ssh options: %q", connInfo)
	}
}

func TestSpecBuildsKingbaseMetadataSQL(t *testing.T) {
	cfg, err := ConfigFromWire(map[string]any{
		"host":     "127.0.0.1",
		"username": "system",
		"database": "TEST",
	})
	if err != nil {
		t.Fatalf("ConfigFromWire returned error: %v", err)
	}
	spec := Spec()

	indexesSQL := spec.SchemaSQL.Indexes(cfg, "", "app", "demo")
	for _, want := range []string{"sys_index", "sys_class", "sys_attribute", "c.relname = 'demo'", "n.nspname = 'app'", "string_agg"} {
		if !strings.Contains(indexesSQL, want) {
			t.Fatalf("indexes SQL %q does not contain %q", indexesSQL, want)
		}
	}

	foreignKeysSQL := spec.SchemaSQL.ForeignKeys(cfg, "", "app", "demo")
	for _, want := range []string{"sys_constraint", "contype = 'f'", "c.relname = 'demo'", "n.nspname = 'app'", "confdeltype", "confupdtype"} {
		if !strings.Contains(foreignKeysSQL, want) {
			t.Fatalf("foreign keys SQL %q does not contain %q", foreignKeysSQL, want)
		}
	}

	viewsSQL := spec.SchemaSQL.Views(cfg, "", "app")
	for _, want := range []string{"sys_class", "pg_get_viewdef", "c.relkind IN ('v','m')", "n.nspname = 'app'", "'YES'"} {
		if !strings.Contains(viewsSQL, want) {
			t.Fatalf("views SQL %q does not contain %q", viewsSQL, want)
		}
	}

	functionsSQL := spec.SchemaSQL.Functions(cfg, "", "app")
	for _, want := range []string{"sys_proc", "sys_namespace", "prokind = 'f'", "n.nspname = 'app'"} {
		if !strings.Contains(functionsSQL, want) {
			t.Fatalf("functions SQL %q does not contain %q", functionsSQL, want)
		}
	}

	viewSQL := spec.SchemaSQL.ViewDefinition(cfg, "", "app", "v_demo")
	for _, want := range []string{"sys_views", "schemaname = 'app'", "viewname = 'v_demo'"} {
		if !strings.Contains(viewSQL, want) {
			t.Fatalf("view definition SQL %q does not contain %q", viewSQL, want)
		}
	}

	dumpDiskSQL := spec.SchemaSQL.DumpDDL(cfg, "", "app", "demo")
	for _, want := range []string{"sys_get_tabledef", "sys_class", "sys_namespace", "c.relname = 'demo'", "n.nspname = 'app'", "c.relkind IN ('r','p','f')"} {
		if !strings.Contains(dumpDiskSQL, want) {
			t.Fatalf("dump DDL SQL %q does not contain %q", dumpDiskSQL, want)
		}
	}

	dumpUnqualifiedSQL := spec.SchemaSQL.DumpDDL(cfg, "", "", "demo")
	if strings.Contains(dumpUnqualifiedSQL, "n.nspname") {
		t.Fatalf("dump DDL SQL without schema still filters by schema: %q", dumpUnqualifiedSQL)
	}
}

func TestSchemaSQLForNamesDumpDDLEscapesValues(t *testing.T) {
	cfg, err := ConfigFromWire(map[string]any{
		"host":     "127.0.0.1",
		"username": "system",
		"database": "TEST",
	})
	if err != nil {
		t.Fatalf("ConfigFromWire returned error: %v", err)
	}

	dumpSQL := schemaSQLForNames(sysCatalogNames()).DumpDDL(cfg, "", "it's", "de'mo")
	if !strings.Contains(dumpSQL, "c.relname = 'de''mo'") {
		t.Fatalf("dump DDL SQL does not escape table name: %q", dumpSQL)
	}
	if !strings.Contains(dumpSQL, "n.nspname = 'it''s'") {
		t.Fatalf("dump DDL SQL does not escape schema name: %q", dumpSQL)
	}
}

func TestSpecBuildsKingbaseObjectsSQLWithProtocolKinds(t *testing.T) {
	cfg, err := ConfigFromWire(map[string]any{
		"host":     "127.0.0.1",
		"username": "system",
		"database": "TEST",
	})
	if err != nil {
		t.Fatalf("ConfigFromWire returned error: %v", err)
	}

	tablesSQL := Spec().SchemaSQL.Objects(cfg, "", "app", []string{"table"})
	for _, want := range []string{
		"WHEN 'p' THEN 'table'",
		"c.relkind IN ('r','p')",
		"n.nspname = 'app'",
	} {
		if !strings.Contains(tablesSQL, want) {
			t.Fatalf("tables SQL %q does not contain %q", tablesSQL, want)
		}
	}
	viewsSQL := Spec().SchemaSQL.Objects(cfg, "", "app", []string{"view", "materialized_view", "sequence"})
	for _, want := range []string{
		"WHEN 'm' THEN 'materialized_view'",
		"WHEN 'S' THEN 'sequence'",
		"c.relkind IN ('v','m','S')",
	} {
		if !strings.Contains(viewsSQL, want) {
			t.Fatalf("objects SQL %q does not contain %q", viewsSQL, want)
		}
	}
}

func TestSchemaSQLForNamesPGUsesPGCatalogs(t *testing.T) {
	cfg, err := ConfigFromWire(map[string]any{
		"host":     "127.0.0.1",
		"username": "system",
		"database": "TEST",
	})
	if err != nil {
		t.Fatalf("ConfigFromWire returned error: %v", err)
	}
	schemaSQL := schemaSQLForNames(pgCatalogNames())

	databasesSQL := schemaSQL.Databases(cfg)
	if !strings.Contains(databasesSQL, "pg_database") {
		t.Fatalf("databases SQL %q does not use pg_database", databasesSQL)
	}

	schemasSQL := schemaSQL.Schemas(cfg, "TEST")
	if !strings.Contains(schemasSQL, "pg_namespace") {
		t.Fatalf("schemas SQL %q does not use pg_namespace", schemasSQL)
	}

	columnsSQL := schemaSQL.Columns(cfg, "", "app", "demo")
	for _, want := range []string{"pg_attribute", "pg_class", "pg_attrdef", "format_type", "pg_get_expr"} {
		if !strings.Contains(columnsSQL, want) {
			t.Fatalf("columns SQL %q does not contain %q", columnsSQL, want)
		}
	}

	indexesSQL := schemaSQL.Indexes(cfg, "", "app", "demo")
	for _, want := range []string{"pg_index", "pg_class", "pg_attribute", "pg_am"} {
		if !strings.Contains(indexesSQL, want) {
			t.Fatalf("indexes SQL %q does not contain %q", indexesSQL, want)
		}
	}

	foreignKeysSQL := schemaSQL.ForeignKeys(cfg, "", "app", "demo")
	for _, want := range []string{"pg_constraint", "pg_class", "pg_attribute"} {
		if !strings.Contains(foreignKeysSQL, want) {
			t.Fatalf("foreign keys SQL %q does not contain %q", foreignKeysSQL, want)
		}
	}

	functionsSQL := schemaSQL.Functions(cfg, "", "app")
	for _, want := range []string{"pg_proc", "pg_namespace", "pg_language"} {
		if !strings.Contains(functionsSQL, want) {
			t.Fatalf("functions SQL %q does not contain %q", functionsSQL, want)
		}
	}

	viewSQL := schemaSQL.ViewDefinition(cfg, "", "app", "v_demo")
	if !strings.Contains(viewSQL, "pg_views") {
		t.Fatalf("view definition SQL %q does not use pg_views", viewSQL)
	}

	dumpSQL := schemaSQL.DumpDDL(cfg, "", "app", "demo")
	for _, want := range []string{"pg_get_tabledef", "pg_class", "pg_namespace", "c.relkind IN ('r','p','f')"} {
		if !strings.Contains(dumpSQL, want) {
			t.Fatalf("dump DDL SQL %q does not contain %q", dumpSQL, want)
		}
	}

	// The pg_* flavor must never reference sys_* relations.
	for _, probe := range []string{
		schemaSQL.Databases(cfg),
		schemaSQL.Schemas(cfg, "TEST"),
		schemaSQL.Objects(cfg, "", "app", nil),
		schemaSQL.Columns(cfg, "", "app", "demo"),
		schemaSQL.Indexes(cfg, "", "app", "demo"),
		schemaSQL.ForeignKeys(cfg, "", "app", "demo"),
		schemaSQL.Views(cfg, "", "app"),
		schemaSQL.Functions(cfg, "", "app"),
		schemaSQL.ViewDefinition(cfg, "", "app", "v_demo"),
		schemaSQL.DumpDDL(cfg, "", "app", "demo"),
	} {
		if strings.Contains(probe, "sys_") {
			t.Fatalf("pg catalog SQL leaked sys_ relation: %q", probe)
		}
	}
}

func TestAdaptSchemaSQLUsesProbeResult(t *testing.T) {
	oldProbe := catalogProbe
	t.Cleanup(func() { catalogProbe = oldProbe })

	cfg, err := ConfigFromWire(map[string]any{
		"host":     "127.0.0.1",
		"username": "system",
		"database": "TEST",
	})
	if err != nil {
		t.Fatalf("ConfigFromWire returned error: %v", err)
	}

	catalogProbe = func(ctx context.Context, db *sql.DB) string { return catalogPGPrefix }
	adapted, err := adaptSchemaSQL(context.Background(), nil, schemaSQLForNames(sysCatalogNames()))
	if err != nil {
		t.Fatalf("adaptSchemaSQL returned error: %v", err)
	}
	if got := adapted.Databases(cfg); !strings.Contains(got, "pg_database") {
		t.Fatalf("adaptSchemaSQL did not switch to pg catalogs, got %q", got)
	}

	catalogProbe = func(ctx context.Context, db *sql.DB) string { return catalogSysPrefix }
	adapted, err = adaptSchemaSQL(context.Background(), nil, schemaSQLForNames(sysCatalogNames()))
	if err != nil {
		t.Fatalf("adaptSchemaSQL returned error: %v", err)
	}
	if got := adapted.Databases(cfg); !strings.Contains(got, "sys_database") {
		t.Fatalf("adaptSchemaSQL should keep sys catalogs, got %q", got)
	}
}
