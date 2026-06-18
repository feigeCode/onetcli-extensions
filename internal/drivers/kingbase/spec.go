package kingbase

import (
	"fmt"
	"sort"
	"strings"

	"onetcli-db-ipc-drivers/internal/dbipc"
)

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
		BuildDSN:             buildDSN,
		SchemaSQL: dbipc.SchemaSQL{
			Databases:      kingbaseDatabasesSQL,
			Schemas:        kingbaseSchemasSQL,
			Objects:        kingbaseObjectsSQL,
			Columns:        kingbaseColumnsSQL,
			Indexes:        kingbaseIndexesSQL,
			ForeignKeys:    kingbaseForeignKeysSQL,
			Views:          kingbaseViewsSQL,
			Functions:      kingbaseFunctionsSQL,
			ViewDefinition: kingbaseViewDefinitionSQL,
		},
	}
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
	for k, v := range cfg.Extra {
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

func kingbaseDatabasesSQL(cfg dbipc.Config) string {
	return "SELECT datname FROM sys_database WHERE datallowconn ORDER BY datname"
}

func kingbaseSchemasSQL(cfg dbipc.Config, database string) string {
	return "SELECT nspname, pg_get_userbyid(nspowner) FROM sys_namespace WHERE nspname NOT LIKE 'pg_%' AND nspname <> 'information_schema' ORDER BY nspname"
}

func kingbaseObjectsSQL(cfg dbipc.Config, database, schema string, kinds []string) string {
	schemaFilter := ""
	if schema != "" {
		schemaFilter = fmt.Sprintf(" AND n.nspname = '%s'", escapeSQL(schema))
	}
	return "SELECT c.relname, CASE c.relkind WHEN 'r' THEN 'table' WHEN 'p' THEN 'table' WHEN 'v' THEN 'view' WHEN 'm' THEN 'materialized_view' WHEN 'S' THEN 'sequence' ELSE 'table' END, COALESCE(obj_description(c.oid), '') FROM sys_class c JOIN sys_namespace n ON n.oid = c.relnamespace WHERE c.relkind IN (" + kingbaseRelkindList(kinds) + ")" + schemaFilter + " ORDER BY n.nspname, c.relname"
}

func kingbaseColumnsSQL(cfg dbipc.Config, database, schema, table string) string {
	schemaFilter := ""
	if schema != "" {
		schemaFilter = fmt.Sprintf(" AND n.nspname = '%s'", escapeSQL(schema))
	}
	return fmt.Sprintf("SELECT a.attnum, a.attname, format_type(a.atttypid, a.atttypmod), CASE WHEN a.attnotnull THEN 'NO' ELSE 'YES' END, pg_get_expr(d.adbin, d.adrelid) FROM sys_attribute a JOIN sys_class c ON c.oid = a.attrelid JOIN sys_namespace n ON n.oid = c.relnamespace LEFT JOIN sys_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum WHERE c.relname = '%s'%s AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum", escapeSQL(table), schemaFilter)
}

func kingbaseIndexesSQL(cfg dbipc.Config, database, schema, table string) string {
	schemaFilter := ""
	if schema != "" {
		schemaFilter = fmt.Sprintf(" AND n.nspname = '%s'", escapeSQL(schema))
	}
	return fmt.Sprintf("SELECT ic.relname, string_agg(a.attname, ',' ORDER BY a.attnum), CASE WHEN i.indisunique THEN 'YES' ELSE 'NO' END, CASE WHEN i.indisprimary THEN 'YES' ELSE 'NO' END, am.amname FROM sys_index i JOIN sys_class c ON c.oid = i.indrelid JOIN sys_namespace n ON n.oid = c.relnamespace JOIN sys_class ic ON ic.oid = i.indexrelid LEFT JOIN sys_am am ON am.oid = ic.relam JOIN sys_attribute a ON a.attrelid = c.oid AND a.attnum = ANY(i.indkey) WHERE c.relname = '%s'%s GROUP BY ic.relname, i.indisunique, i.indisprimary, am.amname ORDER BY ic.relname", escapeSQL(table), schemaFilter)
}

func kingbaseForeignKeysSQL(cfg dbipc.Config, database, schema, table string) string {
	schemaFilter := ""
	if schema != "" {
		schemaFilter = fmt.Sprintf(" AND n.nspname = '%s'", escapeSQL(schema))
	}
	return fmt.Sprintf("SELECT con.conname, string_agg(a.attname, ',' ORDER BY keys.ord), rn.nspname, rc.relname, string_agg(ra.attname, ',' ORDER BY keys.ord), CASE con.confupdtype WHEN 'c' THEN 'CASCADE' WHEN 'r' THEN 'RESTRICT' WHEN 'n' THEN 'SET NULL' WHEN 'd' THEN 'SET DEFAULT' ELSE 'NO ACTION' END, CASE con.confdeltype WHEN 'c' THEN 'CASCADE' WHEN 'r' THEN 'RESTRICT' WHEN 'n' THEN 'SET NULL' WHEN 'd' THEN 'SET DEFAULT' ELSE 'NO ACTION' END FROM sys_constraint con JOIN sys_class c ON c.oid = con.conrelid JOIN sys_namespace n ON n.oid = c.relnamespace JOIN sys_class rc ON rc.oid = con.confrelid JOIN sys_namespace rn ON rn.oid = rc.relnamespace JOIN LATERAL unnest(con.conkey, con.confkey) WITH ORDINALITY AS keys(attnum, ref_attnum, ord) ON true JOIN sys_attribute a ON a.attrelid = c.oid AND a.attnum = keys.attnum JOIN sys_attribute ra ON ra.attrelid = rc.oid AND ra.attnum = keys.ref_attnum WHERE con.contype = 'f' AND c.relname = '%s'%s GROUP BY con.conname, rn.nspname, rc.relname, con.confupdtype, con.confdeltype ORDER BY con.conname", escapeSQL(table), schemaFilter)
}

func kingbaseViewsSQL(cfg dbipc.Config, database, schema string) string {
	schemaFilter := ""
	if schema != "" {
		schemaFilter = fmt.Sprintf(" AND n.nspname = '%s'", escapeSQL(schema))
	}
	return "SELECT c.relname, n.nspname, COALESCE(obj_description(c.oid), ''), CASE WHEN c.relkind = 'm' THEN 'YES' ELSE 'NO' END, COALESCE(pg_get_viewdef(c.oid), '') FROM sys_class c JOIN sys_namespace n ON n.oid = c.relnamespace WHERE c.relkind IN ('v','m')" + schemaFilter + " ORDER BY n.nspname, c.relname"
}

func kingbaseFunctionsSQL(cfg dbipc.Config, database, schema string) string {
	schemaFilter := ""
	if schema != "" {
		schemaFilter = fmt.Sprintf(" AND n.nspname = '%s'", escapeSQL(schema))
	}
	return "SELECT p.proname, n.nspname, pg_get_function_result(p.oid), l.lanname, COALESCE(obj_description(p.oid), '') FROM sys_proc p JOIN sys_namespace n ON n.oid = p.pronamespace LEFT JOIN sys_language l ON l.oid = p.prolang WHERE p.prokind = 'f'" + schemaFilter + " ORDER BY n.nspname, p.proname"
}

func kingbaseViewDefinitionSQL(cfg dbipc.Config, database, schema, view string) string {
	schemaFilter := ""
	if schema != "" {
		schemaFilter = fmt.Sprintf(" AND schemaname = '%s'", escapeSQL(schema))
	}
	return fmt.Sprintf("SELECT definition, 'NO' FROM sys_views WHERE viewname = '%s'%s", escapeSQL(view), schemaFilter)
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
