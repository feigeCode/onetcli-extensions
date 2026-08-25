package kingbase

import (
	"context"
	"database/sql"
	"fmt"
	"sort"
	"strings"
	"time"

	"navop-db-ipc-drivers/internal/dbipc"
)

const (
	catalogSysPrefix = "sys"
	catalogPGPrefix  = "pg"

	catalogProbeTimeout = 5 * time.Second
)

// catalogProbe detects the system catalog prefix a connected server exposes
// ("sys" for KingbaseES V8R6 and newer, "pg" for older V8R3 deployments and
// PostgreSQL-compatible backends). It is a package variable so unit tests can
// stub it without a live server.
var catalogProbe = probeCatalogPrefix

// catalogNames maps each system catalog relation referenced by the metadata
// queries to the server-side relation name for a given catalog prefix.
type catalogNames struct {
	database   string // sys_database / pg_database
	namespace  string // sys_namespace / pg_namespace
	class      string // sys_class / pg_class
	attribute  string // sys_attribute / pg_attribute
	attrdef    string // sys_attrdef / pg_attrdef
	index      string // sys_index / pg_index
	constraint string // sys_constraint / pg_constraint
	proc       string // sys_proc / pg_proc
	language   string // sys_language / pg_language
	am         string // sys_am / pg_am
	views      string // sys_views / pg_views
}

func sysCatalogNames() catalogNames {
	return catalogNames{
		database:   "sys_database",
		namespace:  "sys_namespace",
		class:      "sys_class",
		attribute:  "sys_attribute",
		attrdef:    "sys_attrdef",
		index:      "sys_index",
		constraint: "sys_constraint",
		proc:       "sys_proc",
		language:   "sys_language",
		am:         "sys_am",
		views:      "sys_views",
	}
}

func pgCatalogNames() catalogNames {
	return catalogNames{
		database:   "pg_database",
		namespace:  "pg_namespace",
		class:      "pg_class",
		attribute:  "pg_attribute",
		attrdef:    "pg_attrdef",
		index:      "pg_index",
		constraint: "pg_constraint",
		proc:       "pg_proc",
		language:   "pg_language",
		am:         "pg_am",
		views:      "pg_views",
	}
}

func ConfigFromWire(raw map[string]any) (dbipc.Config, error) {
	return dbipc.ConfigFromWire(raw, 54321)
}

func Spec() dbipc.DriverSpec {
	return dbipc.DriverSpec{
		ID:                   "kingbase",
		Name:                 "KingbaseES",
		SQLDriverName:        "kingbase",
		DefaultPort:          54321,
		IdentifierQuoteLeft:  `"`,
		IdentifierQuoteRight: `"`,
		SupportsComments:     true,
		BuildDSN:             buildDSN,
		AdaptSchemaSQL:       adaptSchemaSQL,
		SchemaSQL:            schemaSQLForNames(sysCatalogNames()),
	}
}

// adaptSchemaSQL adjusts the metadata SQL to the catalog naming the connected
// server actually exposes. Older KingbaseES instances (and PostgreSQL servers
// reached through a Kingbase-compatible protocol) do not have the sys_*
// catalog relations this driver uses by default, so they need the pg_*
// equivalents.
func adaptSchemaSQL(ctx context.Context, db *sql.DB, schemaSQL dbipc.SchemaSQL) (dbipc.SchemaSQL, error) {
	probeCtx, cancel := context.WithTimeout(ctx, catalogProbeTimeout)
	defer cancel()
	if catalogProbe(probeCtx, db) == catalogPGPrefix {
		return schemaSQLForNames(pgCatalogNames()), nil
	}
	return schemaSQL, nil
}

// probeCatalogPrefix asks the connected server which system catalog naming it
// uses. The sys_* probe is authoritative for KingbaseES V8R6 and newer; when
// it fails we fall back to checking pg_* so older KingbaseES and PostgreSQL
// backends keep working.
func probeCatalogPrefix(ctx context.Context, db *sql.DB) string {
	var one int
	if err := db.QueryRowContext(ctx, "SELECT 1 FROM sys_database LIMIT 1").Scan(&one); err == nil {
		return catalogSysPrefix
	}
	if err := db.QueryRowContext(ctx, "SELECT 1 FROM pg_database LIMIT 1").Scan(&one); err == nil {
		return catalogPGPrefix
	}
	return catalogSysPrefix
}

func buildDSN(cfg dbipc.Config) (string, error) {
	if err := dbipc.RequireConfig(cfg, "host", "port", "username", "database"); err != nil {
		return "", err
	}
	pairs := map[string]string{
		"host":     cfg.Host,
		"port":     fmt.Sprint(cfg.Port),
		"user":     cfg.Username,
		"password": cfg.Password,
		"dbname":   cfg.Database,
		"sslmode":  "disable",
	}
	for k, v := range dbipc.CopyDriverExtra(cfg.Extra) {
		pairs[k] = v
	}
	keys := make([]string, 0, len(pairs))
	for k := range pairs {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	parts := make([]string, 0, len(keys))
	for _, key := range keys {
		parts = append(parts, key+"="+escapeConnInfo(pairs[key]))
	}
	return strings.Join(parts, " "), nil
}

func schemaSQLForNames(c catalogNames) dbipc.SchemaSQL {
	return dbipc.SchemaSQL{
		Databases: func(cfg dbipc.Config) string {
			return "SELECT datname FROM " + c.database + " WHERE datallowconn ORDER BY datname"
		},
		Schemas: func(cfg dbipc.Config, database string) string {
			return "SELECT nspname, pg_get_userbyid(nspowner) FROM " + c.namespace + " WHERE nspname NOT LIKE 'pg_%' AND nspname <> 'information_schema' ORDER BY nspname"
		},
		Objects: func(cfg dbipc.Config, database, schema string, kinds []string) string {
			schemaFilter := ""
			if schema != "" {
				schemaFilter = fmt.Sprintf(" AND n.nspname = '%s'", escapeSQL(schema))
			}
			return "SELECT c.relname, CASE c.relkind WHEN 'r' THEN 'table' WHEN 'p' THEN 'table' WHEN 'v' THEN 'view' WHEN 'm' THEN 'materialized_view' WHEN 'S' THEN 'sequence' ELSE 'table' END, COALESCE(obj_description(c.oid), ''), n.nspname FROM " + c.class + " c JOIN " + c.namespace + " n ON n.oid = c.relnamespace WHERE c.relkind IN (" + kingbaseRelkindList(kinds) + ")" + schemaFilter + " ORDER BY n.nspname, c.relname"
		},
		Columns: func(cfg dbipc.Config, database, schema, table string) string {
			schemaFilter := ""
			if schema != "" {
				schemaFilter = fmt.Sprintf(" AND n.nspname = '%s'", escapeSQL(schema))
			}
			return fmt.Sprintf("SELECT a.attnum, a.attname, format_type(a.atttypid, a.atttypmod), CASE WHEN a.attnotnull THEN 'NO' ELSE 'YES' END, pg_get_expr(d.adbin, d.adrelid), COALESCE(col_description(a.attrelid, a.attnum), '') FROM %s a JOIN %s c ON c.oid = a.attrelid JOIN %s n ON n.oid = c.relnamespace LEFT JOIN %s d ON d.adrelid = a.attrelid AND d.adnum = a.attnum WHERE c.relname = '%s'%s AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum", c.attribute, c.class, c.namespace, c.attrdef, escapeSQL(table), schemaFilter)
		},
		Indexes: func(cfg dbipc.Config, database, schema, table string) string {
			schemaFilter := ""
			if schema != "" {
				schemaFilter = fmt.Sprintf(" AND n.nspname = '%s'", escapeSQL(schema))
			}
			return fmt.Sprintf("SELECT ic.relname, string_agg(a.attname, ',' ORDER BY a.attnum), CASE WHEN i.indisunique THEN 'YES' ELSE 'NO' END, CASE WHEN i.indisprimary THEN 'YES' ELSE 'NO' END, am.amname FROM %s i JOIN %s c ON c.oid = i.indrelid JOIN %s n ON n.oid = c.relnamespace JOIN %s ic ON ic.oid = i.indexrelid LEFT JOIN %s am ON am.oid = ic.relam JOIN %s a ON a.attrelid = c.oid AND a.attnum = ANY(i.indkey) WHERE c.relname = '%s'%s GROUP BY ic.relname, i.indisunique, i.indisprimary, am.amname ORDER BY ic.relname", c.index, c.class, c.namespace, c.class, c.am, c.attribute, escapeSQL(table), schemaFilter)
		},
		ForeignKeys: func(cfg dbipc.Config, database, schema, table string) string {
			schemaFilter := ""
			if schema != "" {
				schemaFilter = fmt.Sprintf(" AND n.nspname = '%s'", escapeSQL(schema))
			}
			return fmt.Sprintf("SELECT con.conname, string_agg(a.attname, ',' ORDER BY keys.ord), rn.nspname, rc.relname, string_agg(ra.attname, ',' ORDER BY keys.ord), CASE con.confupdtype WHEN 'c' THEN 'CASCADE' WHEN 'r' THEN 'RESTRICT' WHEN 'n' THEN 'SET NULL' WHEN 'd' THEN 'SET DEFAULT' ELSE 'NO ACTION' END, CASE con.confdeltype WHEN 'c' THEN 'CASCADE' WHEN 'r' THEN 'RESTRICT' WHEN 'n' THEN 'SET NULL' WHEN 'd' THEN 'SET DEFAULT' ELSE 'NO ACTION' END FROM %s con JOIN %s c ON c.oid = con.conrelid JOIN %s n ON n.oid = c.relnamespace JOIN %s rc ON rc.oid = con.confrelid JOIN %s rn ON rn.oid = rc.relnamespace JOIN LATERAL unnest(con.conkey, con.confkey) WITH ORDINALITY AS keys(attnum, ref_attnum, ord) ON true JOIN %s a ON a.attrelid = c.oid AND a.attnum = keys.attnum JOIN %s ra ON ra.attrelid = rc.oid AND ra.attnum = keys.ref_attnum WHERE con.contype = 'f' AND c.relname = '%s'%s GROUP BY con.conname, rn.nspname, rc.relname, con.confupdtype, con.confdeltype ORDER BY con.conname", c.constraint, c.class, c.namespace, c.class, c.namespace, c.attribute, c.attribute, escapeSQL(table), schemaFilter)
		},
		Views: func(cfg dbipc.Config, database, schema string) string {
			schemaFilter := ""
			if schema != "" {
				schemaFilter = fmt.Sprintf(" AND n.nspname = '%s'", escapeSQL(schema))
			}
			return "SELECT c.relname, n.nspname, COALESCE(obj_description(c.oid), ''), CASE WHEN c.relkind = 'm' THEN 'YES' ELSE 'NO' END, COALESCE(pg_get_viewdef(c.oid), '') FROM " + c.class + " c JOIN " + c.namespace + " n ON n.oid = c.relnamespace WHERE c.relkind IN ('v','m')" + schemaFilter + " ORDER BY n.nspname, c.relname"
		},
		Functions: func(cfg dbipc.Config, database, schema string) string {
			schemaFilter := ""
			if schema != "" {
				schemaFilter = fmt.Sprintf(" AND n.nspname = '%s'", escapeSQL(schema))
			}
			return "SELECT p.proname, n.nspname, pg_get_function_result(p.oid), l.lanname, COALESCE(obj_description(p.oid), '') FROM " + c.proc + " p JOIN " + c.namespace + " n ON n.oid = p.pronamespace LEFT JOIN " + c.language + " l ON l.oid = p.prolang WHERE p.prokind = 'f'" + schemaFilter + " ORDER BY n.nspname, p.proname"
		},
		ViewDefinition: func(cfg dbipc.Config, database, schema, view string) string {
			schemaFilter := ""
			if schema != "" {
				schemaFilter = fmt.Sprintf(" AND schemaname = '%s'", escapeSQL(schema))
			}
			return fmt.Sprintf("SELECT definition, 'NO' FROM %s WHERE viewname = '%s'%s", c.views, escapeSQL(view), schemaFilter)
		},
	}
}

func kingbaseRelkindList(kinds []string) string {
	if len(kinds) == 0 {
		return "'r','p','v','m','S'"
	}
	seen := map[string]bool{}
	for _, kind := range kinds {
		switch strings.ToLower(strings.TrimSpace(kind)) {
		case "table", "base_table":
			seen["r"] = true
			seen["p"] = true
		case "view":
			seen["v"] = true
		case "materialized_view":
			seen["m"] = true
		case "sequence":
			seen["S"] = true
		}
	}
	if len(seen) == 0 {
		return "''"
	}
	order := []string{"r", "p", "v", "m", "S"}
	values := make([]string, 0, len(seen))
	for _, relkind := range order {
		if seen[relkind] {
			values = append(values, "'"+relkind+"'")
		}
	}
	return strings.Join(values, ",")
}

func escapeConnInfo(value string) string {
	if value == "" {
		return "''"
	}

	needsQuote := false
	for _, r := range value {
		switch r {
		case ' ', '\t', '\n', '\r', '\v', '\f', '\'', '\\':
			needsQuote = true
		}
		if needsQuote {
			break
		}
	}
	if !needsQuote {
		return value
	}

	var b strings.Builder
	b.Grow(len(value) + 2)
	b.WriteByte('\'')
	for _, r := range value {
		if r == '\\' || r == '\'' {
			b.WriteByte('\\')
		}
		b.WriteRune(r)
	}
	b.WriteByte('\'')
	return b.String()
}

func escapeSQL(value string) string {
	return strings.ReplaceAll(value, "'", "''")
}
