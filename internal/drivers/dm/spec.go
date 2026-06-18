package dm

import (
	"fmt"
	"net"
	"strconv"
	"strings"

	"onetcli-db-ipc-drivers/internal/dbipc"
)

func ConfigFromWire(raw map[string]any) (dbipc.Config, error) {
	return dbipc.ConfigFromWire(raw, 5236)
}

func Spec() dbipc.DriverSpec {
	return dbipc.DriverSpec{
		ID:                   "dm",
		Name:                 "Dameng DM",
		SQLDriverName:        "dm",
		DefaultPort:          5236,
		IdentifierQuoteLeft:  `"`,
		IdentifierQuoteRight: `"`,
		BuildDSN:             buildDSN,
		SchemaSQL: dbipc.SchemaSQL{
			Databases:      dmDatabasesSQL,
			Schemas:        dmSchemasSQL,
			Objects:        dmObjectsSQL,
			Columns:        dmColumnsSQL,
			Indexes:        dmIndexesSQL,
			ForeignKeys:    dmForeignKeysSQL,
			Views:          dmViewsSQL,
			Functions:      dmFunctionsSQL,
			ViewDefinition: dmViewDefinitionSQL,
		},
	}
}

func buildDSN(cfg dbipc.Config) (string, error) {
	if err := dbipc.RequireConfig(cfg, "host", "port", "username"); err != nil {
		return "", err
	}
	extra := dbipc.CopyExtra(cfg.Extra)
	if cfg.Database != "" && extra["schema"] == "" {
		extra["schema"] = cfg.Database
	}

	// Dameng's Go driver parses the DSN with string splitting and does not URL-decode
	// credentials. Keep auth text raw so passwords like p@ss are sent as p@ss.
	address := net.JoinHostPort(cfg.Host, strconv.Itoa(cfg.Port))
	dsn := fmt.Sprintf("dm://%s:%s@%s", cfg.Username, cfg.Password, address)
	query := dbipc.QueryString(extra)
	if query == "" {
		if strings.Contains(cfg.Username, "?") || strings.Contains(cfg.Password, "?") {
			return dsn + "?", nil
		}
		return dsn, nil
	}
	return dsn + "?" + query, nil
}

func dmDatabasesSQL(cfg dbipc.Config) string {
	return "SELECT NAME FROM (SELECT USER AS NAME FROM DUAL UNION SELECT USERNAME AS NAME FROM ALL_USERS UNION SELECT OWNER AS NAME FROM ALL_TABLES) WHERE NAME IS NOT NULL ORDER BY NAME"
}

func dmSchemasSQL(cfg dbipc.Config, database string) string {
	return "SELECT USERNAME, USERNAME FROM (SELECT USER AS USERNAME FROM DUAL UNION SELECT USERNAME FROM ALL_USERS UNION SELECT OWNER AS USERNAME FROM ALL_TABLES) WHERE USERNAME IS NOT NULL ORDER BY USERNAME"
}

func dmObjectsSQL(cfg dbipc.Config, database, schema string, kinds []string) string {
	ownerFilter := ""
	if owner := dmOwner(database, schema); owner != "" {
		ownerFilter = fmt.Sprintf(" AND OWNER = '%s'", upperEscapeSQL(owner))
	}
	return "SELECT OBJECT_NAME, KIND, COMMENTS FROM (" +
		"SELECT t.OWNER, t.TABLE_NAME AS OBJECT_NAME, 'table' AS KIND, NVL(c.COMMENTS, '') AS COMMENTS FROM ALL_TABLES t LEFT JOIN ALL_TAB_COMMENTS c ON c.OWNER = t.OWNER AND c.TABLE_NAME = t.TABLE_NAME " +
		"UNION ALL " +
		"SELECT v.OWNER, v.VIEW_NAME AS OBJECT_NAME, 'view' AS KIND, NVL(c.COMMENTS, '') AS COMMENTS FROM ALL_VIEWS v LEFT JOIN ALL_TAB_COMMENTS c ON c.OWNER = v.OWNER AND c.TABLE_NAME = v.VIEW_NAME" +
		") WHERE 1 = 1" + ownerFilter + dmKindFilter(kinds) + " ORDER BY OWNER, OBJECT_NAME"
}

func dmColumnsSQL(cfg dbipc.Config, database, schema, table string) string {
	owner, table := dmOwnerAndTable(database, schema, table)
	ownerFilter := ""
	if owner != "" {
		ownerFilter = fmt.Sprintf(" AND c.OWNER = '%s'", upperEscapeSQL(owner))
	}
	return fmt.Sprintf("SELECT c.COLUMN_ID, c.COLUMN_NAME, c.DATA_TYPE, c.NULLABLE, c.DATA_DEFAULT FROM ALL_TAB_COLUMNS c LEFT JOIN ALL_COL_COMMENTS cc ON cc.OWNER = c.OWNER AND cc.TABLE_NAME = c.TABLE_NAME AND cc.COLUMN_NAME = c.COLUMN_NAME WHERE c.TABLE_NAME = '%s'%s ORDER BY c.COLUMN_ID", upperEscapeSQL(table), ownerFilter)
}

func dmIndexesSQL(cfg dbipc.Config, database, schema, table string) string {
	owner, table := dmOwnerAndTable(database, schema, table)
	ownerFilter := ""
	if owner != "" {
		ownerFilter = fmt.Sprintf(" AND i.TABLE_OWNER = '%s'", upperEscapeSQL(owner))
	}
	return fmt.Sprintf("SELECT i.INDEX_NAME, LISTAGG(c.COLUMN_NAME, ',') WITHIN GROUP (ORDER BY c.COLUMN_POSITION), CASE WHEN i.UNIQUENESS = 'UNIQUE' THEN 'YES' ELSE 'NO' END, CASE WHEN pk.CONSTRAINT_TYPE = 'P' THEN 'YES' ELSE 'NO' END, i.INDEX_TYPE FROM ALL_INDEXES i JOIN ALL_IND_COLUMNS c ON c.INDEX_OWNER = i.OWNER AND c.INDEX_NAME = i.INDEX_NAME LEFT JOIN ALL_CONSTRAINTS pk ON pk.OWNER = i.TABLE_OWNER AND pk.TABLE_NAME = i.TABLE_NAME AND pk.INDEX_NAME = i.INDEX_NAME AND pk.CONSTRAINT_TYPE = 'P' WHERE i.TABLE_NAME = '%s'%s GROUP BY i.INDEX_NAME, i.UNIQUENESS, pk.CONSTRAINT_TYPE, i.INDEX_TYPE ORDER BY i.INDEX_NAME", upperEscapeSQL(table), ownerFilter)
}

func dmForeignKeysSQL(cfg dbipc.Config, database, schema, table string) string {
	owner, table := dmOwnerAndTable(database, schema, table)
	ownerFilter := ""
	if owner != "" {
		ownerFilter = fmt.Sprintf(" AND fk.OWNER = '%s'", upperEscapeSQL(owner))
	}
	return fmt.Sprintf("SELECT fk.CONSTRAINT_NAME, LISTAGG(fkc.COLUMN_NAME, ',') WITHIN GROUP (ORDER BY fkc.POSITION), pk.OWNER, pk.TABLE_NAME, LISTAGG(pkc.COLUMN_NAME, ',') WITHIN GROUP (ORDER BY fkc.POSITION), 'NO ACTION', fk.DELETE_RULE FROM ALL_CONSTRAINTS fk JOIN ALL_CONS_COLUMNS fkc ON fkc.OWNER = fk.OWNER AND fkc.CONSTRAINT_NAME = fk.CONSTRAINT_NAME JOIN ALL_CONSTRAINTS pk ON pk.OWNER = fk.R_OWNER AND pk.CONSTRAINT_NAME = fk.R_CONSTRAINT_NAME JOIN ALL_CONS_COLUMNS pkc ON pkc.OWNER = pk.OWNER AND pkc.CONSTRAINT_NAME = pk.CONSTRAINT_NAME AND pkc.POSITION = fkc.POSITION WHERE fk.CONSTRAINT_TYPE = 'R' AND fk.TABLE_NAME = '%s'%s GROUP BY fk.CONSTRAINT_NAME, pk.OWNER, pk.TABLE_NAME, fk.DELETE_RULE ORDER BY fk.CONSTRAINT_NAME", upperEscapeSQL(table), ownerFilter)
}

func dmViewsSQL(cfg dbipc.Config, database, schema string) string {
	ownerFilter := ""
	if owner := dmOwner(database, schema); owner != "" {
		ownerFilter = fmt.Sprintf(" AND v.OWNER = '%s'", upperEscapeSQL(owner))
	}
	return "SELECT v.VIEW_NAME, v.OWNER, NVL(c.COMMENTS, ''), 'NO', NVL(v.TEXT, '') FROM ALL_VIEWS v LEFT JOIN ALL_TAB_COMMENTS c ON c.OWNER = v.OWNER AND c.TABLE_NAME = v.VIEW_NAME WHERE 1 = 1" + ownerFilter + " ORDER BY v.OWNER, v.VIEW_NAME"
}

func dmFunctionsSQL(cfg dbipc.Config, database, schema string) string {
	ownerFilter := ""
	if owner := dmOwner(database, schema); owner != "" {
		ownerFilter = fmt.Sprintf(" AND o.OWNER = '%s'", upperEscapeSQL(owner))
	}
	return "SELECT o.OBJECT_NAME, o.OWNER, NVL(p.DATA_TYPE, ''), 'SQL', '' FROM ALL_OBJECTS o LEFT JOIN ALL_PROCEDURES p ON p.OWNER = o.OWNER AND p.OBJECT_NAME = o.OBJECT_NAME WHERE o.OBJECT_TYPE = 'FUNCTION'" + ownerFilter + " ORDER BY o.OWNER, o.OBJECT_NAME"
}

func dmViewDefinitionSQL(cfg dbipc.Config, database, schema, view string) string {
	owner, view := dmOwnerAndTable(database, schema, view)
	ownerFilter := ""
	if owner != "" {
		ownerFilter = fmt.Sprintf(" AND OWNER = '%s'", upperEscapeSQL(owner))
	}
	return fmt.Sprintf("SELECT TEXT, 'NO' FROM ALL_VIEWS WHERE VIEW_NAME = '%s'%s", upperEscapeSQL(view), ownerFilter)
}

func dmOwner(database, schema string) string {
	if strings.TrimSpace(schema) != "" {
		return schema
	}
	return database
}

func dmOwnerAndTable(database, schema, table string) (string, string) {
	owner := dmOwner(database, schema)
	name := strings.TrimSpace(table)
	if owner == "" {
		if parts := strings.SplitN(name, ".", 2); len(parts) == 2 {
			owner = parts[0]
			name = parts[1]
		}
	}
	return stripIdentifierQuotes(owner), stripIdentifierQuotes(name)
}

func dmKindFilter(kinds []string) string {
	if len(kinds) == 0 {
		return ""
	}
	seen := map[string]bool{}
	for _, kind := range kinds {
		switch strings.ToLower(strings.TrimSpace(kind)) {
		case "table", "base_table":
			seen["table"] = true
		case "view":
			seen["view"] = true
		}
	}
	if len(seen) == 0 {
		return " AND 1 = 0"
	}
	values := make([]string, 0, len(seen))
	if seen["table"] {
		values = append(values, "'table'")
	}
	if seen["view"] {
		values = append(values, "'view'")
	}
	return " AND KIND IN (" + strings.Join(values, ",") + ")"
}

func stripIdentifierQuotes(value string) string {
	return strings.Trim(strings.TrimSpace(value), `"`)
}

func escapeSQL(value string) string {
	return strings.ReplaceAll(value, "'", "''")
}

func upperEscapeSQL(value string) string {
	return escapeSQL(strings.ToUpper(strings.TrimSpace(value)))
}
