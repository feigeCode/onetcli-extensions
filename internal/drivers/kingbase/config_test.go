package kingbase

import (
	"strings"
	"testing"
)

func TestSpecBuildsKingbaseConnInfoFromOnetcliConfig(t *testing.T) {
	cfg, err := ConfigFromWire(map[string]any{
		"host":     "127.0.0.1",
		"port":     float64(54321),
		"username": "system",
		"password": "123456",
		"database": "TEST",
		"extra_params": map[string]any{
			"sslmode":              "disable",
			"connect_timeout":      "10",
			"application_name":     "onetcli",
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
		"application_name=onetcli",
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
