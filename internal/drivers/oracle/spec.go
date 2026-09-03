package oracle

import (
	"fmt"
	"net"
	"net/url"
	"strconv"
	"strings"

	"navop-db-ipc-drivers/internal/dbipc"
)

func ConfigFromWire(raw map[string]any) (dbipc.Config, error) {
	return dbipc.ConfigFromWire(raw, 1521)
}

func Spec() dbipc.DriverSpec {
	return dbipc.DriverSpec{
		ID:                   "oracle-go",
		Name:                 "Oracle Go",
		SQLDriverName:        "oracle",
		DefaultPort:          1521,
		IdentifierQuoteLeft:  `"`,
		IdentifierQuoteRight: `"`,
		SupportsComments:     true,
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
			Procedures:     oracleProceduresSQL,
			Triggers:       oracleTriggersSQL,
			Sequences:      oracleSequencesSQL,
			ViewDefinition: oracleViewDefinitionSQL,
			DumpDDL:        oracleDumpDDL,
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
		return "", fmt.Errorf("missing required config field service_name or sid")
	}

	values := url.Values{}
	for key, value := range oracleDSNParams(cfg.Extra) {
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

// oracleDSNParams returns the go-ora URL options for a connection.
//
// Only parameters go-ora understands may appear in the DSN: go-ora rejects
// any unknown option with "unknown URL option: <key>" (protocol code -33001).
// Host-managed generic parameters (connect_timeout / read_timeout from the
// connection form, schema preferences, external driver identity) are therefore
// never copied into the DSN. The form's timeout values are mapped to the
// go-ora equivalents so user-configured timeouts still take effect.
func oracleDSNParams(extra map[string]string) map[string]string {
	params := make(map[string]string)
	for key, value := range dbipc.CopyDriverExtra(extra) {
		switch normalizeOptionKey(key) {
		case "connect_timeout":
			if seconds, ok := timeoutSeconds(value); ok {
				params["CONNECT TIMEOUT"] = seconds
			}
		case "read_timeout":
			if seconds, ok := timeoutSeconds(value); ok {
				params["READ TIMEOUT"] = seconds
			}
		case "role":
			// Host-managed Oracle role (default/sysdba/sysoper); maps to the
			// go-ora "DBA PRIVILEGE" URL option. Empty/default adds nothing.
			if privilege := oracleRoleValue(value); privilege != "" {
				params["DBA PRIVILEGE"] = privilege
			}
		case "external_driver_id",
			"default_schema",
			"schema_filter_mode",
			"schema_filter_include",
			"schema_filter_exclude":
			// Host-managed; not go-ora URL options.
		default:
			// Only go-ora URL options may appear in the DSN. Any other key
			// (connection name, remark, or arbitrary host form fields) would
			// be rejected by go-ora with "unknown URL option: <key>".
			if goOraURLOption(key) {
				params[key] = value
			}
		}
	}
	return params
}

// oracleRoleValue maps the host form role value to the DBAPrivilege string
// go-ora understands. Anything other than sysdba/sysoper connects normally.
func oracleRoleValue(raw string) string {
	switch strings.ToUpper(strings.TrimSpace(raw)) {
	case "SYSDBA":
		return "SYSDBA"
	case "SYSOPER":
		return "SYSOPER"
	default:
		return ""
	}
}

// goOraSupportedOptionNormalized lists every URL option go-ora understands
// (the switch in go-ora's connect_config.go), normalized to the lowercase
// underscore spelling used by normalizeOptionKey. Options that make go-ora
// fail on parse (FAILOVER / RETRY TIME) are intentionally excluded.
var goOraSupportedOptionNormalized = map[string]struct{}{
	"cid":                          {},
	"connstr":                      {},
	"server":                       {},
	"service_name":                 {},
	"sid":                          {},
	"instance_name":                {},
	"wallet":                       {},
	"wallet_password":              {},
	"auth_type":                    {},
	"os_user":                      {},
	"os_pass":                      {},
	"os_password":                  {},
	"os_hash":                      {},
	"os_passhash":                  {},
	"os_password_hash":             {},
	"domain":                       {},
	"auth_serv":                    {},
	"encryption":                   {},
	"data_integrity":               {},
	"ssl":                          {},
	"ssl_verify":                   {},
	"dba_privilege":                {},
	"timeout":                      {},
	"read_timeout":                 {},
	"socket_timeout":               {},
	"connect_timeout":              {},
	"connection_timeout":           {},
	"trace_file":                   {},
	"trace_dir":                    {},
	"trace_folder":                 {},
	"trace_directory":              {},
	"use_oob":                      {},
	"enable_oob":                   {},
	"enable_urgent_data_transport": {},
	"prefetch_rows":                {},
	"unix_socket":                  {},
	"proxy_client_name":            {},
	"lob_fetch":                    {},
	"language":                     {},
	"territory":                    {},
	"charset":                      {},
	"client_charset":               {},
	"program":                      {},
	"server_location":              {},
}

// goOraURLOption reports whether key names a URL option go-ora accepts.
func goOraURLOption(key string) bool {
	_, ok := goOraSupportedOptionNormalized[normalizeOptionKey(key)]
	return ok
}

// normalizeOptionKey folds a wire extra-param key to a canonical lowercase
// option name so host forms can use either snake_case or space-separated
// spellings.
func normalizeOptionKey(key string) string {
	normalized := strings.ToLower(strings.TrimSpace(key))
	normalized = strings.ReplaceAll(normalized, " ", "_")
	normalized = strings.ReplaceAll(normalized, "-", "_")
	return normalized
}

// timeoutSeconds converts a form timeout value (seconds) to the integer
// seconds string go-ora expects. Unparseable or negative values are skipped so
// a bad form value never fails the whole connection.
func timeoutSeconds(raw string) (string, bool) {
	value := strings.TrimSpace(raw)
	if value == "" {
		return "", false
	}
	seconds, err := strconv.Atoi(value)
	if err != nil || seconds < 0 {
		return "", false
	}
	return strconv.Itoa(seconds), true
}

func oracleDatabasesSQL(cfg dbipc.Config) string {
	return "SELECT COALESCE(NULLIF(SYS_CONTEXT('USERENV', 'CON_NAME'), ''), SYS_CONTEXT('USERENV', 'DB_NAME')) AS NAME FROM DUAL"
}

func oracleSchemasSQL(cfg dbipc.Config, database string) string {
	return "SELECT USERNAME AS NAME, USERNAME AS OWNER FROM ALL_USERS ORDER BY 1"
}

func oracleObjectsSQL(cfg dbipc.Config, database, schema string, kinds []string) string {
	ownerFilter := ""
	if owner := oracleOwner(database, schema); owner != "" {
		ownerFilter = fmt.Sprintf(" AND o.OWNER = '%s'", upperEscapeSQL(owner))
	}
	return "SELECT o.OBJECT_NAME, CASE o.OBJECT_TYPE WHEN 'TABLE' THEN 'table' WHEN 'VIEW' THEN 'view' WHEN 'MATERIALIZED VIEW' THEN 'materialized_view' WHEN 'SEQUENCE' THEN 'sequence' ELSE LOWER(REPLACE(o.OBJECT_TYPE, ' ', '_')) END AS KIND, NVL(c.COMMENTS, '') AS COMMENTS, o.OWNER AS SCHEMA FROM ALL_OBJECTS o LEFT JOIN ALL_TAB_COMMENTS c ON c.OWNER = o.OWNER AND c.TABLE_NAME = o.OBJECT_NAME WHERE o.OBJECT_TYPE IN (" + oracleObjectTypeList(kinds) + ")" + ownerFilter + " ORDER BY o.OWNER, o.OBJECT_NAME"
}

func oracleColumnsSQL(cfg dbipc.Config, database, schema, table string) string {
	owner, table := oracleOwnerAndTable(database, schema, table)
	ownerFilter := ""
	if owner != "" {
		ownerFilter = fmt.Sprintf(" AND c.OWNER = '%s'", upperEscapeSQL(owner))
	}
	return fmt.Sprintf("SELECT c.COLUMN_ID, c.COLUMN_NAME, c.DATA_TYPE, c.NULLABLE, c.DATA_DEFAULT, NVL(cc.COMMENTS, '') FROM ALL_TAB_COLUMNS c LEFT JOIN ALL_COL_COMMENTS cc ON cc.OWNER = c.OWNER AND cc.TABLE_NAME = c.TABLE_NAME AND cc.COLUMN_NAME = c.COLUMN_NAME WHERE c.TABLE_NAME = '%s'%s ORDER BY c.COLUMN_ID", upperEscapeSQL(table), ownerFilter)
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
	return "SELECT o.OBJECT_NAME, o.OWNER, NVL(p.DATA_TYPE, ''), 'PL/SQL', '', o.STATUS, o.CREATED, o.LAST_DDL_TIME FROM ALL_OBJECTS o LEFT JOIN ALL_PROCEDURES p ON p.OWNER = o.OWNER AND p.OBJECT_NAME = o.OBJECT_NAME WHERE o.OBJECT_TYPE = 'FUNCTION'" + ownerFilter + " ORDER BY o.OWNER, o.OBJECT_NAME"
}

func oracleProceduresSQL(cfg dbipc.Config, database, schema string) string {
	ownerFilter := ""
	if owner := oracleOwner(database, schema); owner != "" {
		ownerFilter = fmt.Sprintf(" AND o.OWNER = '%s'", upperEscapeSQL(owner))
	}
	return "SELECT o.OBJECT_NAME, o.OWNER, '', 'PL/SQL', '', o.STATUS, o.CREATED, o.LAST_DDL_TIME FROM ALL_OBJECTS o WHERE o.OBJECT_TYPE = 'PROCEDURE'" + ownerFilter + " ORDER BY o.OWNER, o.OBJECT_NAME"
}

func oracleTriggersSQL(cfg dbipc.Config, database, schema, table string) string {
	owner, table := oracleOwnerAndTable(database, schema, table)
	ownerFilter := ""
	if owner != "" {
		ownerFilter = fmt.Sprintf(" AND OWNER = '%s'", upperEscapeSQL(owner))
	}
	tableFilter := ""
	if table != "" {
		tableFilter = fmt.Sprintf(" AND TABLE_NAME = '%s'", upperEscapeSQL(table))
	}
	return "SELECT TRIGGER_NAME, TABLE_NAME, TRIGGER_TYPE, TRIGGERING_EVENT, TRIGGER_BODY, STATUS FROM ALL_TRIGGERS WHERE 1 = 1" + ownerFilter + tableFilter + " ORDER BY OWNER, TRIGGER_NAME"
}

func oracleSequencesSQL(cfg dbipc.Config, database, schema string) string {
	ownerFilter := ""
	if owner := oracleOwner(database, schema); owner != "" {
		ownerFilter = fmt.Sprintf(" AND SEQUENCE_OWNER = '%s'", upperEscapeSQL(owner))
	}
	return "SELECT SEQUENCE_NAME, MIN_VALUE, MAX_VALUE, INCREMENT_BY, LAST_NUMBER, CACHE_SIZE, CYCLE_FLAG FROM ALL_SEQUENCES WHERE 1 = 1" + ownerFilter + " ORDER BY SEQUENCE_OWNER, SEQUENCE_NAME"
}

func oracleViewDefinitionSQL(cfg dbipc.Config, database, schema, view string) string {
	owner, view := oracleOwnerAndTable(database, schema, view)
	ownerFilter := ""
	if owner != "" {
		ownerFilter = fmt.Sprintf(" AND OWNER = '%s'", upperEscapeSQL(owner))
	}
	return fmt.Sprintf("SELECT TEXT, 'NO' FROM ALL_VIEWS WHERE VIEW_NAME = '%s'%s", upperEscapeSQL(view), ownerFilter)
}

// oracleDumpDDL asks the server for the official CREATE TABLE text via
// DBMS_METADATA. The schema/owner argument is omitted when unknown so the
// provider uses the connected user's schema.
func oracleDumpDDL(cfg dbipc.Config, database, schema, table string) string {
	owner, table := oracleOwnerAndTable(database, schema, table)
	if owner == "" {
		return fmt.Sprintf("SELECT DBMS_METADATA.GET_DDL('TABLE', '%s') FROM DUAL", upperEscapeSQL(table))
	}
	return fmt.Sprintf("SELECT DBMS_METADATA.GET_DDL('TABLE', '%s', '%s') FROM DUAL", upperEscapeSQL(table), upperEscapeSQL(owner))
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
