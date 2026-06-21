package oceanbase

import (
	"context"
	"fmt"
	"io"
	"net"
	"strings"
	"time"

	"github.com/go-sql-driver/mysql"

	"onetcli-db-ipc-drivers/internal/dbipc"
	"onetcli-db-ipc-drivers/internal/drivers/oracle"
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
	mysqlCfg.Params = dbipc.CopyExtra(cfg.Extra)
	return mysqlCfg.FormatDSN(), nil
}

func buildMySQLWireOracleTenantDSN(cfg dbipc.Config) (string, error) {
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
	mysqlCfg := mysql.NewConfig()
	mysqlCfg.User = cfg.Username
	mysqlCfg.Passwd = cfg.Password
	mysqlCfg.Net = "tcp"
	mysqlCfg.Addr = net.JoinHostPort(cfg.Host, fmt.Sprint(cfg.Port))
	mysqlCfg.DBName = service
	mysqlCfg.ParseTime = true
	mysqlCfg.Params = dbipc.CopyExtra(cfg.Extra)
	delete(mysqlCfg.Params, "oracle_mysql_wire_driver")
	return mysqlCfg.FormatDSN(), nil
}

func oracleMySQLWireDriverName(cfg dbipc.Config) string {
	if driverName := strings.TrimSpace(cfg.Extra["oracle_mysql_wire_driver"]); driverName != "" {
		return driverName
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
		return false, err
	}
	length := int(header[0]) | int(header[1])<<8 | int(header[2])<<16
	if length <= 0 || length > 4096 {
		return false, nil
	}
	payload := make([]byte, length)
	if _, err := io.ReadFull(conn, payload); err != nil {
		return false, err
	}
	if len(payload) == 0 || payload[0] != 0x0a {
		return false, nil
	}
	return strings.Contains(strings.ToLower(string(payload)), "oceanbase"), nil
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
