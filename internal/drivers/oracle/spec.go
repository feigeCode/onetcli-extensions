package oracle

import (
	"fmt"
	"net"
	"net/url"
	"strconv"
	"strings"

	"onetcli-db-ipc-drivers/internal/dbipc"
)

func ConfigFromWire(raw map[string]any) (dbipc.Config, error) {
	return dbipc.ConfigFromWire(raw, 1521)
}

func Spec() dbipc.DriverSpec {
	return dbipc.DriverSpec{
		ID:                   "oracle",
		Name:                 "Oracle",
		SQLDriverName:        "oracle",
		DefaultPort:          1521,
		IdentifierQuoteLeft:  `"`,
		IdentifierQuoteRight: `"`,
		BuildDSN:             buildDSN,
		SchemaSQL: dbipc.SchemaSQL{
			Databases:      oracleDatabasesSQL,
			Schemas:        oracleSchemasSQL,
			Objects:        oracleObjectsSQL,
			Columns:        oracleColumnsSQL,
			Indexes:        oracleIndexesSQL,
			ForeignKeys:    oracleForeignKeysSQL,
			Views:          oracleViewsSQL,
			Functions:      oracleFunctionsSQL,
			ViewDefinition: oracleViewDefinitionSQL,
		},
	}
}

func buildDSN(cfg dbipc.Config) (string, error) {
	if err := dbipc.RequireConfig(cfg, "host", "port", "username"); err != nil {
		return "", err
	}
	service := strings.TrimSpace(cfg.Service)
	if service == "" {
		service = strings.TrimSpace(cfg.SID)
	}
	if service == "" {
		service = strings.TrimSpace(cfg.Database)
	}
	if service == "" {
		return "", fmt.Errorf("missing required config field service_name, sid, or database")
	}

	values := url.Values{}
	for key, value := range cfg.Extra {
		values.Set(key, value)
	}
	rawURL := url.URL{
		Scheme:   "oracle",
		User:     url.UserPassword(cfg.Username, cfg.Password),
		Host:     net.JoinHostPort(cfg.Host, strconv.Itoa(cfg.Port)),
		Path:     "/" + service,
		RawQuery: values.Encode(),
	}
	return rawURL.String(), nil
}

func oracleDatabasesSQL(cfg dbipc.Config) string {
	return "SELECT COALESCE(NULLIF(SYS_CONTEXT('USERENV', 'CON_NAME'), ''), SYS_CONTEXT('USERENV', 'DB_NAME')) AS NAME FROM DUAL"
}

func oracleSchemasSQL(cfg dbipc.Config, database string) string {
	return "SELECT USERNAME, USERNAME FROM ALL_USERS ORDER BY USERNAME"
}

func oracleObjectsSQL(cfg dbipc.Config, database, schema string, kinds []string) string {
	ownerFilter := ""
	if owner := oracleOwner(database, schema); owner != "" {
		ownerFilter = fmt.Sprintf(" AND o.OWNER = '%s'", upperEscapeSQL(owner))
	}
	return "SELECT o.OBJECT_NAME, CASE o.OBJECT_TYPE WHEN 'TABLE' THEN 'table' WHEN 'VIEW' THEN 'view' WHEN 'MATERIALIZED VIEW' THEN 'materialized_view' WHEN 'SEQUENCE' THEN 'sequence' ELSE LOWER(REPLACE(o.OBJECT_TYPE, ' ', '_')) END AS KIND, NVL(c.COMMENTS, '') AS COMMENTS FROM ALL_OBJECTS o LEFT JOIN ALL_TAB_COMMENTS c ON c.OWNER = o.OWNER AND c.TABLE_NAME = o.OBJECT_NAME WHERE o.OBJECT_TYPE IN (" + oracleObjectTypeList(kinds) + ")" + ownerFilter + " ORDER BY o.OWNER, o.OBJECT_NAME"
}

func oracleColumnsSQL(cfg dbipc.Config, database, schema, table string) string {
	owner, table := oracleOwnerAndTable(database, schema, table)
	ownerFilter := ""
	if owner != "" {
		ownerFilter = fmt.Sprintf(" AND c.OWNER = '%s'", upperEscapeSQL(owner))
	}
	return fmt.Sprintf("SELECT c.COLUMN_ID, c.COLUMN_NAME, c.DATA_TYPE, c.NULLABLE, c.DATA_DEFAULT FROM ALL_TAB_COLUMNS c LEFT JOIN ALL_COL_COMMENTS cc ON cc.OWNER = c.OWNER AND cc.TABLE_NAME = c.TABLE_NAME AND cc.COLUMN_NAME = c.COLUMN_NAME WHERE c.TABLE_NAME = '%s'%s ORDER BY c.COLUMN_ID", upperEscapeSQL(table), ownerFilter)
}

func oracleIndexesSQL(cfg dbipc.Config, database, schema, table string) string {
	owner, table := oracleOwnerAndTable(database, schema, table)
	ownerFilter := ""
	if owner != "" {
		ownerFilter = fmt.Sprintf(" AND i.TABLE_OWNER = '%s'", upperEscapeSQL(owner))
	}
	return fmt.Sprintf("SELECT i.INDEX_NAME, LISTAGG(c.COLUMN_NAME, ',') WITHIN GROUP (ORDER BY c.COLUMN_POSITION), CASE WHEN i.UNIQUENESS = 'UNIQUE' THEN 'YES' ELSE 'NO' END, CASE WHEN pk.CONSTRAINT_TYPE = 'P' THEN 'YES' ELSE 'NO' END, i.INDEX_TYPE FROM ALL_INDEXES i JOIN ALL_IND_COLUMNS c ON c.INDEX_OWNER = i.OWNER AND c.INDEX_NAME = i.INDEX_NAME LEFT JOIN ALL_CONSTRAINTS pk ON pk.OWNER = i.TABLE_OWNER AND pk.TABLE_NAME = i.TABLE_NAME AND pk.INDEX_NAME = i.INDEX_NAME AND pk.CONSTRAINT_TYPE = 'P' WHERE i.TABLE_NAME = '%s'%s GROUP BY i.INDEX_NAME, i.UNIQUENESS, pk.CONSTRAINT_TYPE, i.INDEX_TYPE ORDER BY i.INDEX_NAME", upperEscapeSQL(table), ownerFilter)
}

func oracleForeignKeysSQL(cfg dbipc.Config, database, schema, table string) string {
	owner, table := oracleOwnerAndTable(database, schema, table)
	ownerFilter := ""
	if owner != "" {
		ownerFilter = fmt.Sprintf(" AND fk.OWNER = '%s'", upperEscapeSQL(owner))
	}
	return fmt.Sprintf("SELECT fk.CONSTRAINT_NAME, LISTAGG(fkc.COLUMN_NAME, ',') WITHIN GROUP (ORDER BY fkc.POSITION), pk.OWNER, pk.TABLE_NAME, LISTAGG(pkc.COLUMN_NAME, ',') WITHIN GROUP (ORDER BY fkc.POSITION), 'NO ACTION', fk.DELETE_RULE FROM ALL_CONSTRAINTS fk JOIN ALL_CONS_COLUMNS fkc ON fkc.OWNER = fk.OWNER AND fkc.CONSTRAINT_NAME = fk.CONSTRAINT_NAME JOIN ALL_CONSTRAINTS pk ON pk.OWNER = fk.R_OWNER AND pk.CONSTRAINT_NAME = fk.R_CONSTRAINT_NAME JOIN ALL_CONS_COLUMNS pkc ON pkc.OWNER = pk.OWNER AND pkc.CONSTRAINT_NAME = pk.CONSTRAINT_NAME AND pkc.POSITION = fkc.POSITION WHERE fk.CONSTRAINT_TYPE = 'R' AND fk.TABLE_NAME = '%s'%s GROUP BY fk.CONSTRAINT_NAME, pk.OWNER, pk.TABLE_NAME, fk.DELETE_RULE ORDER BY fk.CONSTRAINT_NAME", upperEscapeSQL(table), ownerFilter)
}

func oracleViewsSQL(cfg dbipc.Config, database, schema string) string {
	ownerFilter := ""
	if owner := oracleOwner(database, schema); owner != "" {
		ownerFilter = fmt.Sprintf(" AND v.OWNER = '%s'", upperEscapeSQL(owner))
	}
	return "SELECT v.VIEW_NAME, v.OWNER, NVL(c.COMMENTS, ''), 'NO', NVL(v.TEXT, '') FROM ALL_VIEWS v LEFT JOIN ALL_TAB_COMMENTS c ON c.OWNER = v.OWNER AND c.TABLE_NAME = v.VIEW_NAME WHERE 1 = 1" + ownerFilter + " ORDER BY v.OWNER, v.VIEW_NAME"
}

func oracleFunctionsSQL(cfg dbipc.Config, database, schema string) string {
	ownerFilter := ""
	if owner := oracleOwner(database, schema); owner != "" {
		ownerFilter = fmt.Sprintf(" AND o.OWNER = '%s'", upperEscapeSQL(owner))
	}
	return "SELECT o.OBJECT_NAME, o.OWNER, NVL(p.DATA_TYPE, ''), 'SQL', '' FROM ALL_OBJECTS o LEFT JOIN ALL_PROCEDURES p ON p.OWNER = o.OWNER AND p.OBJECT_NAME = o.OBJECT_NAME WHERE o.OBJECT_TYPE = 'FUNCTION'" + ownerFilter + " ORDER BY o.OWNER, o.OBJECT_NAME"
}

func oracleViewDefinitionSQL(cfg dbipc.Config, database, schema, view string) string {
	owner, view := oracleOwnerAndTable(database, schema, view)
	ownerFilter := ""
	if owner != "" {
		ownerFilter = fmt.Sprintf(" AND OWNER = '%s'", upperEscapeSQL(owner))
	}
	return fmt.Sprintf("SELECT TEXT, 'NO' FROM ALL_VIEWS WHERE VIEW_NAME = '%s'%s", upperEscapeSQL(view), ownerFilter)
}

func oracleOwner(database, schema string) string {
	if strings.TrimSpace(schema) != "" {
		return schema
	}
	return database
}

func oracleOwnerAndTable(database, schema, table string) (string, string) {
	owner := oracleOwner(database, schema)
	name := strings.TrimSpace(table)
	if owner == "" {
		if parts := strings.SplitN(name, ".", 2); len(parts) == 2 {
			owner = parts[0]
			name = parts[1]
		}
	}
	return stripIdentifierQuotes(owner), stripIdentifierQuotes(name)
}

func oracleObjectTypeList(kinds []string) string {
	if len(kinds) == 0 {
		return "'TABLE','VIEW','MATERIALIZED VIEW','SEQUENCE'"
	}
	seen := map[string]bool{}
	for _, kind := range kinds {
		switch strings.ToLower(strings.TrimSpace(kind)) {
		case "table", "base_table":
			seen["TABLE"] = true
		case "view":
			seen["VIEW"] = true
		case "materialized_view":
			seen["MATERIALIZED VIEW"] = true
		case "sequence":
			seen["SEQUENCE"] = true
		}
	}
	if len(seen) == 0 {
		return "''"
	}
	order := []string{"TABLE", "VIEW", "MATERIALIZED VIEW", "SEQUENCE"}
	values := make([]string, 0, len(seen))
	for _, typ := range order {
		if seen[typ] {
			values = append(values, "'"+typ+"'")
		}
	}
	return strings.Join(values, ",")
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
