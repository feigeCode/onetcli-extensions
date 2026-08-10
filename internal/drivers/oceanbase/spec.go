package oceanbase

import (
	"bytes"
	"context"
	"fmt"
	"io"
	"math"
	"net"
	"net/url"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/go-sql-driver/mysql"

	"navop-db-ipc-drivers/internal/dbipc"
	"navop-db-ipc-drivers/internal/drivers/oracle"
)

const (
	protocolMySQL  = "mysql"
	protocolOracle = "oracle"
)

var probeOceanBaseMySQLWire = probeOceanBaseMySQLWireHandshake

func ConfigFromWire(raw map[string]any) (dbipc.Config, error) {
	cfg, err := dbipc.ConfigFromWire(raw, 2881)
	if err != nil {
		return cfg, err
	}
	if strings.TrimSpace(cfg.Protocol) == "" {
		cfg.Protocol = protocolMySQL
	}
	return cfg, nil
}

func Spec() dbipc.DriverSpec {
	return dbipc.DriverSpec{
		ID:                   "oceanbase",
		Name:                 "OceanBase",
		SQLDriverName:        "mysql",
		DefaultPort:          2881,
		IdentifierQuoteLeft:  "`",
		IdentifierQuoteRight: "`",
		BuildDSN:             buildMySQLDSN,
		ResolveConnection:    resolveConnection,
		SchemaSQL: dbipc.SchemaSQL{
			Databases:      oceanbaseMySQLDatabasesSQL,
			Schemas:        oceanbaseMySQLSchemasSQL,
			Objects:        oceanbaseMySQLObjectsSQL,
			Columns:        oceanbaseMySQLColumnsSQL,
			Indexes:        oceanbaseMySQLIndexesSQL,
			ForeignKeys:    oceanbaseMySQLForeignKeysSQL,
			Views:          oceanbaseMySQLViewsSQL,
			Functions:      oceanbaseMySQLFunctionsSQL,
			ViewDefinition: oceanbaseMySQLViewDefinitionSQL,
		},
	}
}

func resolveConnection(ctx context.Context, cfg dbipc.Config) (dbipc.ConnectionSpec, error) {
	switch normalizeProtocol(cfg.Protocol) {
	case protocolMySQL:
		dsn, err := buildMySQLDSN(cfg)
		if err != nil {
			return dbipc.ConnectionSpec{}, err
		}
		return dbipc.ConnectionSpec{DriverName: "mysql", DSN: dsn, SchemaSQL: Spec().SchemaSQL}, nil
	case protocolOracle:
		oracleSpec := oracle.Spec()
		isOBMySQLWire, err := probeOceanBaseMySQLWire(ctx, cfg.Host, cfg.Port)
		if err != nil {
			return dbipc.ConnectionSpec{}, err
		}
		if isOBMySQLWire {
			dsn, err := buildMySQLWireOracleTenantDSN(cfg)
			if err != nil {
				return dbipc.ConnectionSpec{}, err
			}
			return dbipc.ConnectionSpec{
				DriverName:           oracleMySQLWireDriverName(cfg),
				DSN:                  dsn,
				IdentifierQuoteLeft:  oracleSpec.IdentifierQuoteLeft,
				IdentifierQuoteRight: oracleSpec.IdentifierQuoteRight,
				SchemaSQL:            oracleSpec.SchemaSQL,
			}, nil
		}
		dsn, err := oracleSpec.BuildDSN(cfg)
		if err != nil {
			return dbipc.ConnectionSpec{}, err
		}
		return dbipc.ConnectionSpec{
			DriverName:           "oracle",
			DSN:                  dsn,
			IdentifierQuoteLeft:  oracleSpec.IdentifierQuoteLeft,
			IdentifierQuoteRight: oracleSpec.IdentifierQuoteRight,
			SchemaSQL:            oracleSpec.SchemaSQL,
		}, nil
	default:
		return dbipc.ConnectionSpec{}, fmt.Errorf("unsupported OceanBase protocol %q", cfg.Protocol)
	}
}

func buildMySQLDSN(cfg dbipc.Config) (string, error) {
	if err := dbipc.RequireConfig(cfg, "host", "port", "username", "database"); err != nil {
		return "", err
	}
	mysqlCfg := mysql.NewConfig()
	mysqlCfg.User = cfg.Username
	mysqlCfg.Passwd = cfg.Password
	mysqlCfg.Net = "tcp"
	mysqlCfg.Addr = net.JoinHostPort(cfg.Host, fmt.Sprint(cfg.Port))
	mysqlCfg.DBName = cfg.Database
	mysqlCfg.ParseTime = true
	params, err := mysqlDriverExtra(cfg.Extra)
	if err != nil {
		return "", err
	}
	mysqlCfg.Params = params
	return mysqlCfg.FormatDSN(), nil
}

func mysqlDriverExtra(extra map[string]string) (map[string]string, error) {
	params := dbipc.CopyDriverExtra(extra)
	keys := make([]string, 0, len(params))
	for key := range params {
		keys = append(keys, key)
	}
	sort.Strings(keys)

	var timeoutValue string
	var timeoutFound bool
	var timeoutIsCanonical bool
	for _, key := range keys {
		normalizedKey := strings.ToLower(strings.TrimSpace(key))
		switch normalizedKey {
		case "timeout":
			if !timeoutIsCanonical || key == "timeout" {
				timeoutValue = params[key]
				timeoutFound = true
				timeoutIsCanonical = true
			}
			delete(params, key)
		case "connect_timeout", "connecttimeout", "connect timeout":
			if !timeoutIsCanonical && !timeoutFound {
				timeoutValue = params[key]
				timeoutFound = true
			}
			delete(params, key)
		}
	}
	if !timeoutFound {
		return params, nil
	}

	timeout, err := normalizeMySQLTimeout(timeoutValue)
	if err != nil {
		return nil, err
	}
	params["timeout"] = timeout
	return params, nil
}

func normalizeMySQLTimeout(raw string) (string, error) {
	value := strings.TrimSpace(raw)
	if value == "" {
		return "", fmt.Errorf("invalid connect timeout %q", raw)
	}
	if duration, err := time.ParseDuration(value); err == nil {
		if duration < 0 {
			return "", fmt.Errorf("invalid connect timeout %q: must not be negative", raw)
		}
		if duration == 0 && strings.ContainsAny(value, "123456789") {
			return "", fmt.Errorf("invalid connect timeout %q: below minimum duration", raw)
		}
		return duration.String(), nil
	}
	seconds, err := strconv.ParseFloat(value, 64)
	if err != nil || math.IsNaN(seconds) || math.IsInf(seconds, 0) || seconds < 0 {
		return "", fmt.Errorf("invalid connect timeout %q", raw)
	}
	if seconds >= float64(math.MaxInt64)/float64(time.Second) {
		return "", fmt.Errorf("invalid connect timeout %q: exceeds maximum duration", raw)
	}
	duration := time.Duration(seconds * float64(time.Second))
	if seconds > 0 && duration == 0 {
		return "", fmt.Errorf("invalid connect timeout %q: below minimum duration", raw)
	}
	return duration.String(), nil
}

func buildMySQLWireOracleTenantDSN(cfg dbipc.Config) (string, error) {
	if err := dbipc.RequireConfig(cfg, "host", "port", "username"); err != nil {
		return "", err
	}

	dsnURL := url.URL{
		Scheme: "oboracle",
		Host:   net.JoinHostPort(cfg.Host, fmt.Sprint(cfg.Port)),
		User:   url.UserPassword(cfg.Username, cfg.Password),
	}
	// OceanBase Oracle tenants are selected by the login identity. Sending the
	// generic Database field in the MySQL-wire handshake enables
	// CLIENT_CONNECT_WITH_DB and makes OceanBase reject schema-like values with
	// error 1049 (Unknown database). Keep Database available to metadata code,
	// but do not use it as the initial handshake database.

	params := url.Values{}
	params.Set("preset", "oboracle")
	mergeOracleOBClientParams(params, dbipc.CopyDriverExtra(cfg.Extra))
	if strings.TrimSpace(params.Get("preset")) == "" {
		params.Set("preset", "oboracle")
	}
	dsnURL.RawQuery = params.Encode()
	return dsnURL.String(), nil
}

func mergeOracleOBClientParams(params url.Values, extra map[string]string) {
	for rawName, value := range extra {
		name := strings.TrimSpace(rawName)
		lowerName := strings.ToLower(name)
		switch {
		case lowerName == "protocol", lowerName == "oracle_mysql_wire_driver":
			continue
		case lowerName == "connectionattributes":
			for attributeName, attributeValue := range parseConnectionAttributes(value) {
				params.Set("attr."+attributeName, attributeValue)
			}
		case strings.HasPrefix(lowerName, "attr."):
			attributeName := strings.TrimSpace(name[len("attr."):])
			if attributeName != "" {
				params.Set("attr."+attributeName, value)
			}
		default:
			switch lowerName {
			case "timeout", "connecttimeout", "connect timeout", "connect_timeout":
				params.Set("timeout", value)
			case "trace":
				params.Set("trace", value)
			case "preset":
				params.Set("preset", value)
			case "cap.add":
				params.Set("cap.add", value)
			case "cap.drop":
				params.Set("cap.drop", value)
			case "collation":
				params.Set("collation", value)
			case "ob20", "protocol.v2":
				params.Set(lowerName, value)
			case "ob20.magic":
				params.Set("ob20.magic", value)
			case "ob20.disablechecksum":
				params.Set("ob20.disableChecksum", value)
			case "compress", "usecompression", "use_compression":
				params.Set("useCompression", value)
			case "tls":
				params.Set("tls", value)
			case "tls.ca", "tls_ca":
				params.Set("tls.ca", value)
			case "tls.cert", "tls_cert":
				params.Set("tls.cert", value)
			case "tls.key", "tls_key":
				params.Set("tls.key", value)
			case "init":
				params.Add("init", value)
			}
		}
	}
}

func parseConnectionAttributes(raw string) map[string]string {
	attributes := map[string]string{}
	for _, item := range strings.Split(raw, ",") {
		name, value, ok := strings.Cut(strings.TrimSpace(item), ":")
		name = strings.TrimSpace(name)
		if !ok || name == "" {
			continue
		}
		attributes[name] = strings.TrimSpace(value)
	}
	return attributes
}

func oracleMySQLWireDriverName(cfg dbipc.Config) string {
	if driverName := strings.TrimSpace(cfg.Extra["oracle_mysql_wire_driver"]); driverName != "" {
		return driverName
	}
	keys := make([]string, 0, len(cfg.Extra))
	for key := range cfg.Extra {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	for _, key := range keys {
		if !strings.EqualFold(strings.TrimSpace(key), "oracle_mysql_wire_driver") {
			continue
		}
		if driverName := strings.TrimSpace(cfg.Extra[key]); driverName != "" {
			return driverName
		}
	}
	return "oboracle"
}

func normalizeProtocol(protocol string) string {
	switch strings.ToLower(strings.TrimSpace(protocol)) {
	case "", "mysql":
		return protocolMySQL
	case "oracle":
		return protocolOracle
	default:
		return strings.ToLower(strings.TrimSpace(protocol))
	}
}

func probeOceanBaseMySQLWireHandshake(ctx context.Context, host string, port int) (bool, error) {
	dialer := net.Dialer{Timeout: 1500 * time.Millisecond}
	conn, err := dialer.DialContext(ctx, "tcp", net.JoinHostPort(host, fmt.Sprint(port)))
	if err != nil {
		return false, err
	}
	defer conn.Close()
	_ = conn.SetReadDeadline(time.Now().Add(1500 * time.Millisecond))

	header := make([]byte, 4)
	if _, err := io.ReadFull(conn, header); err != nil {
		// Oracle TNS listeners do not send a MySQL initial handshake. Once TCP
		// is reachable, a read timeout/EOF means this is not a MySQL-wire
		// candidate and the caller should fall back to go-ora.
		return false, nil
	}
	if header[3] != 0 {
		return false, nil
	}
	length := int(header[0]) | int(header[1])<<8 | int(header[2])<<16
	if length < 34 || length > 65536 {
		return false, nil
	}
	payload := make([]byte, length)
	if _, err := io.ReadFull(conn, payload); err != nil {
		return false, nil
	}
	// obconnector-go currently accepts the MySQL protocol 10 initial
	// handshake and requires at least 34 payload bytes. Keep the probe aligned
	// with what the selected driver can actually parse, while allowing a
	// generic server_version string that does not mention OceanBase.
	if payload[0] != 0x0a {
		return false, nil
	}
	versionEnd := bytes.IndexByte(payload[1:], 0)
	if versionEnd < 0 {
		return false, nil
	}
	// OceanBase may expose a generic server_version string. A structurally
	// valid MySQL initial handshake is sufficient to select the
	// OBClient-compatible driver; a TNS listener does not emit this packet.
	return strings.TrimSpace(string(payload[1:1+versionEnd])) != "", nil
}

func oceanbaseMySQLDatabasesSQL(cfg dbipc.Config) string {
	return "SELECT SCHEMA_NAME FROM INFORMATION_SCHEMA.SCHEMATA ORDER BY SCHEMA_NAME"
}

func oceanbaseMySQLSchemasSQL(cfg dbipc.Config, database string) string {
	return "SELECT SCHEMA_NAME, DEFAULT_CHARACTER_SET_NAME FROM INFORMATION_SCHEMA.SCHEMATA ORDER BY SCHEMA_NAME"
}

func oceanbaseMySQLObjectsSQL(cfg dbipc.Config, database, schema string, kinds []string) string {
	db := mysqlCatalog(database, schema, cfg.Database)
	return "SELECT TABLE_NAME, CASE TABLE_TYPE WHEN 'BASE TABLE' THEN 'table' WHEN 'VIEW' THEN 'view' ELSE LOWER(REPLACE(TABLE_TYPE, ' ', '_')) END, TABLE_COMMENT FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA = '" + escapeSQL(db) + "'" + mysqlKindFilter(kinds) + " ORDER BY TABLE_NAME"
}

func oceanbaseMySQLColumnsSQL(cfg dbipc.Config, database, schema, table string) string {
	db := mysqlCatalog(database, schema, cfg.Database)
	return fmt.Sprintf("SELECT ORDINAL_POSITION, COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_DEFAULT FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_SCHEMA = '%s' AND TABLE_NAME = '%s' ORDER BY ORDINAL_POSITION", escapeSQL(db), escapeSQL(table))
}

func oceanbaseMySQLIndexesSQL(cfg dbipc.Config, database, schema, table string) string {
	db := mysqlCatalog(database, schema, cfg.Database)
	return fmt.Sprintf("SELECT INDEX_NAME, GROUP_CONCAT(COLUMN_NAME ORDER BY SEQ_IN_INDEX SEPARATOR ','), CASE WHEN NON_UNIQUE = 0 THEN 'YES' ELSE 'NO' END, CASE WHEN INDEX_NAME = 'PRIMARY' THEN 'YES' ELSE 'NO' END, INDEX_TYPE FROM INFORMATION_SCHEMA.STATISTICS WHERE TABLE_SCHEMA = '%s' AND TABLE_NAME = '%s' GROUP BY INDEX_NAME, NON_UNIQUE, INDEX_TYPE ORDER BY INDEX_NAME", escapeSQL(db), escapeSQL(table))
}

func oceanbaseMySQLForeignKeysSQL(cfg dbipc.Config, database, schema, table string) string {
	db := mysqlCatalog(database, schema, cfg.Database)
	return fmt.Sprintf("SELECT CONSTRAINT_NAME, GROUP_CONCAT(COLUMN_NAME ORDER BY ORDINAL_POSITION SEPARATOR ','), REFERENCED_TABLE_SCHEMA, REFERENCED_TABLE_NAME, GROUP_CONCAT(REFERENCED_COLUMN_NAME ORDER BY ORDINAL_POSITION SEPARATOR ','), 'NO ACTION', 'NO ACTION' FROM INFORMATION_SCHEMA.KEY_COLUMN_USAGE WHERE TABLE_SCHEMA = '%s' AND TABLE_NAME = '%s' AND REFERENCED_TABLE_NAME IS NOT NULL GROUP BY CONSTRAINT_NAME, REFERENCED_TABLE_SCHEMA, REFERENCED_TABLE_NAME ORDER BY CONSTRAINT_NAME", escapeSQL(db), escapeSQL(table))
}

func oceanbaseMySQLViewsSQL(cfg dbipc.Config, database, schema string) string {
	db := mysqlCatalog(database, schema, cfg.Database)
	return fmt.Sprintf("SELECT TABLE_NAME, TABLE_SCHEMA, TABLE_COMMENT, 'NO', '' FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA = '%s' AND TABLE_TYPE = 'VIEW' ORDER BY TABLE_NAME", escapeSQL(db))
}

func oceanbaseMySQLFunctionsSQL(cfg dbipc.Config, database, schema string) string {
	db := mysqlCatalog(database, schema, cfg.Database)
	return fmt.Sprintf("SELECT ROUTINE_NAME, ROUTINE_SCHEMA, DTD_IDENTIFIER, ROUTINE_TYPE, '' FROM INFORMATION_SCHEMA.ROUTINES WHERE ROUTINE_SCHEMA = '%s' AND ROUTINE_TYPE = 'FUNCTION' ORDER BY ROUTINE_NAME", escapeSQL(db))
}

func oceanbaseMySQLViewDefinitionSQL(cfg dbipc.Config, database, schema, view string) string {
	db := mysqlCatalog(database, schema, cfg.Database)
	return fmt.Sprintf("SELECT VIEW_DEFINITION, 'NO' FROM INFORMATION_SCHEMA.VIEWS WHERE TABLE_SCHEMA = '%s' AND TABLE_NAME = '%s'", escapeSQL(db), escapeSQL(view))
}

func mysqlCatalog(database, schema, fallback string) string {
	if strings.TrimSpace(schema) != "" {
		return schema
	}
	if strings.TrimSpace(database) != "" {
		return database
	}
	return fallback
}

func mysqlKindFilter(kinds []string) string {
	if len(kinds) == 0 {
		return ""
	}
	seen := map[string]bool{}
	for _, kind := range kinds {
		switch strings.ToLower(strings.TrimSpace(kind)) {
		case "table", "base_table":
			seen["'BASE TABLE'"] = true
		case "view":
			seen["'VIEW'"] = true
		}
	}
	if len(seen) == 0 {
		return " AND 1 = 0"
	}
	values := make([]string, 0, len(seen))
	for _, value := range []string{"'BASE TABLE'", "'VIEW'"} {
		if seen[value] {
			values = append(values, value)
		}
	}
	return " AND TABLE_TYPE IN (" + strings.Join(values, ",") + ")"
}

func escapeSQL(value string) string {
	return strings.ReplaceAll(value, "'", "''")
}
