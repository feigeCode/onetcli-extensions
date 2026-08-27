package oceanbase

import (
	"context"
	"encoding/binary"
	"fmt"
	"io"
	"net"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/go-sql-driver/mysql"
	obconnector "github.com/helingjun/obconnector-go"
	"navop-db-ipc-drivers/internal/dbipc"
)

const clientConnectWithDB uint32 = 1 << 3

const obConnectorBaseServerCapabilities uint32 = (1 << 9) |
	(1 << 15) |
	(1 << 19) |
	(1 << 20) |
	(1 << 23) |
	(1 << 27)

func TestSpecResolvesMySQLProtocolToMySQLWireDriver(t *testing.T) {
	cfg := ConfigFromWireNoError(t, map[string]any{
		"host":     "127.0.0.1",
		"port":     float64(2881),
		"username": "root@test",
		"password": "p@ss word",
		"database": "app",
		"protocol": "mysql",
	})

	connSpec, err := Spec().ResolveConnection(context.Background(), cfg)
	if err != nil {
		t.Fatalf("ResolveConnection returned error: %v", err)
	}

	if connSpec.DriverName != "mysql" {
		t.Fatalf("driver = %q, want mysql", connSpec.DriverName)
	}
	for _, want := range []string{"root@test:p@ss word@tcp(127.0.0.1:2881)/app", "parseTime=true"} {
		if !strings.Contains(connSpec.DSN, want) {
			t.Fatalf("dsn %q does not contain %q", connSpec.DSN, want)
		}
	}
	if connSpec.SchemaSQL.Databases == nil || !strings.Contains(connSpec.SchemaSQL.Databases(cfg), "INFORMATION_SCHEMA.SCHEMATA") {
		t.Fatalf("mysql protocol did not select MySQL metadata SQL")
	}
}

func TestSpecBuildsMySQLDSNWithoutHostManagedSSHOptions(t *testing.T) {
	cfg := ConfigFromWireNoError(t, map[string]any{
		"host":     "127.0.0.1",
		"username": "root@test",
		"password": "secret",
		"database": "app",
		"protocol": "mysql",
		"extra_params": map[string]any{
			"charset":       "utf8mb4",
			"ssh_auth_type": "password",
			" SSH_PORT ":    22,
		},
	})

	dsn, err := buildMySQLDSN(cfg)
	if err != nil {
		t.Fatalf("buildMySQLDSN returned error: %v", err)
	}

	if !strings.Contains(dsn, "charset=utf8mb4") {
		t.Fatalf("dsn %q does not contain charset=utf8mb4", dsn)
	}
	if strings.Contains(strings.ToLower(dsn), "ssh_") {
		t.Fatalf("dsn leaked host-managed ssh options: %q", dsn)
	}
}

func TestSpecMySQLDSNDropsDriverControlParamsFromWire(t *testing.T) {
	cfg := ConfigFromWireNoError(t, map[string]any{
		"host":     "127.0.0.1",
		"username": "root@test",
		"password": "secret",
		"database": "app",
		"extra_params": map[string]any{
			"PROTOCOL":                 "mysql",
			"oracle_mysql_wire_driver": "oboracle-test",
			"charset":                  "utf8mb4",
		},
	})
	if cfg.Protocol != "mysql" {
		t.Fatalf("protocol = %q, want mysql", cfg.Protocol)
	}

	dsn, err := buildMySQLDSN(cfg)
	if err != nil {
		t.Fatalf("buildMySQLDSN returned error: %v", err)
	}
	parsed, err := mysql.ParseDSN(dsn)
	if err != nil {
		t.Fatalf("mysql.ParseDSN(%q) returned error: %v", dsn, err)
	}
	assertNoDriverControlParams(t, parsed.Params)
	if parsed.Params["charset"] != "utf8mb4" {
		t.Fatalf("charset = %q, want utf8mb4; params = %#v", parsed.Params["charset"], parsed.Params)
	}
}

func TestSpecMySQLDSNDropsDriverControlParamsWhenBypassingWire(t *testing.T) {
	// Guard mysqlDriverExtra directly: even if ConfigFromWire misses a variant
	// (or cfg is built without ConfigFromWire), driver-control params must not
	// become server SET statements.
	cfg := dbipc.Config{
		Host:     "127.0.0.1",
		Port:     2881,
		Username: "root@test",
		Password: "secret",
		Database: "app",
		Extra: map[string]string{
			"protocol":                   "mysql",
			"PROTOCOL":                   "mysql",
			"Oracle_MySQL_Wire_Driver":   "oboracle-test",
			" oracle_mysql_wire_driver ": "dedicated",
			"charset":                    "utf8mb4",
		},
	}

	dsn, err := buildMySQLDSN(cfg)
	if err != nil {
		t.Fatalf("buildMySQLDSN returned error: %v", err)
	}
	parsed, err := mysql.ParseDSN(dsn)
	if err != nil {
		t.Fatalf("mysql.ParseDSN(%q) returned error: %v", dsn, err)
	}
	assertNoDriverControlParams(t, parsed.Params)
	if parsed.Params["charset"] != "utf8mb4" {
		t.Fatalf("charset = %q, want utf8mb4; params = %#v", parsed.Params["charset"], parsed.Params)
	}
}

func assertNoDriverControlParams(t *testing.T, params map[string]string) {
	t.Helper()
	for key := range params {
		normalized := strings.ToLower(strings.TrimSpace(key))
		if normalized == "protocol" || normalized == "oracle_mysql_wire_driver" {
			t.Fatalf("dsn leaked driver-control param %q as server SET statement: %#v", key, params)
		}
	}
}

func TestSpecNormalizesMySQLConnectTimeoutAliasAsSeconds(t *testing.T) {
	cfg := ConfigFromWireNoError(t, map[string]any{
		"host":     "127.0.0.1",
		"username": "root@test",
		"database": "app",
		"protocol": "mysql",
		"extra_params": map[string]any{
			"connect_timeout": "30",
		},
	})

	dsn, err := buildMySQLDSN(cfg)
	if err != nil {
		t.Fatalf("buildMySQLDSN returned error: %v", err)
	}
	parsed, err := mysql.ParseDSN(dsn)
	if err != nil {
		t.Fatalf("mysql.ParseDSN(%q) returned error: %v", dsn, err)
	}
	if parsed.Timeout != 30*time.Second {
		t.Fatalf("timeout = %s, want 30s", parsed.Timeout)
	}
	if _, exists := parsed.Params["connect_timeout"]; exists {
		t.Fatalf("connect_timeout leaked into server params: %#v", parsed.Params)
	}
}

func TestSpecPrefersCanonicalMySQLTimeoutOverAlias(t *testing.T) {
	cfg := ConfigFromWireNoError(t, map[string]any{
		"host":     "127.0.0.1",
		"username": "root@test",
		"database": "app",
		"protocol": "mysql",
		"extra_params": map[string]any{
			"timeout":         "1500ms",
			"connect_timeout": "30",
		},
	})

	dsn, err := buildMySQLDSN(cfg)
	if err != nil {
		t.Fatalf("buildMySQLDSN returned error: %v", err)
	}
	parsed, err := mysql.ParseDSN(dsn)
	if err != nil {
		t.Fatalf("mysql.ParseDSN(%q) returned error: %v", dsn, err)
	}
	if parsed.Timeout != 1500*time.Millisecond {
		t.Fatalf("timeout = %s, want 1.5s", parsed.Timeout)
	}
}

func TestSpecRejectsEmptyCanonicalMySQLTimeoutInsteadOfFallingBackToAlias(t *testing.T) {
	cfg := ConfigFromWireNoError(t, map[string]any{
		"host":     "127.0.0.1",
		"username": "root@test",
		"database": "app",
		"protocol": "mysql",
		"extra_params": map[string]any{
			"timeout":         "",
			"connect_timeout": "30",
		},
	})

	if _, err := buildMySQLDSN(cfg); err == nil {
		t.Fatal("buildMySQLDSN accepted an empty canonical timeout")
	}
}

func TestSpecRejectsEmptyMySQLConnectTimeoutAlias(t *testing.T) {
	cfg := ConfigFromWireNoError(t, map[string]any{
		"host":     "127.0.0.1",
		"username": "root@test",
		"database": "app",
		"protocol": "mysql",
		"extra_params": map[string]any{
			"connect_timeout": "",
		},
	})

	if _, err := buildMySQLDSN(cfg); err == nil {
		t.Fatal("buildMySQLDSN accepted an empty connect_timeout alias")
	}
}

func TestNormalizeMySQLTimeoutBoundaries(t *testing.T) {
	tests := []struct {
		name    string
		input   string
		want    string
		wantErr bool
	}{
		{name: "empty", input: "", wantErr: true},
		{name: "whitespace", input: " \t ", wantErr: true},
		{name: "seconds", input: "30", want: "30s"},
		{name: "fractional seconds", input: "1.5", want: "1.5s"},
		{name: "scientific notation", input: "1e3", want: "16m40s"},
		{name: "duration unit", input: "1500ms", want: "1.5s"},
		{name: "microseconds", input: "1us", want: "1µs"},
		{name: "zero", input: "0", want: "0s"},
		{name: "zero with unit", input: "0s", want: "0s"},
		{name: "positive sub nanosecond", input: "0.0000000001", wantErr: true},
		{name: "positive sub nanosecond with unit", input: "0.0000000001s", wantErr: true},
		{name: "negative sub nanosecond with unit", input: "-0.0000000001s", wantErr: true},
		{name: "negative seconds", input: "-1", wantErr: true},
		{name: "negative duration", input: "-1s", wantErr: true},
		{name: "nan", input: "NaN", wantErr: true},
		{name: "positive infinity", input: "+Inf", wantErr: true},
		{name: "negative infinity", input: "-Inf", wantErr: true},
		{name: "float overflow", input: "1e400", wantErr: true},
		{name: "duration overflow without unit", input: "1e10", wantErr: true},
		{name: "duration overflow with unit", input: "10000000000s", wantErr: true},
		{name: "unknown unit", input: "1d", wantErr: true},
		{name: "scientific notation with unit", input: "1e3ms", wantErr: true},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			got, err := normalizeMySQLTimeout(test.input)
			if (err != nil) != test.wantErr {
				t.Fatalf("normalizeMySQLTimeout(%q) error = %v, wantErr %v", test.input, err, test.wantErr)
			}
			if !test.wantErr && got != test.want {
				t.Fatalf("normalizeMySQLTimeout(%q) = %q, want %q", test.input, got, test.want)
			}
		})
	}
}

func TestSpecResolvesOracleProtocolOverOceanBaseMySQLWireToDedicatedDriver(t *testing.T) {
	oldProbe := probeOceanBaseMySQLWire
	probeOceanBaseMySQLWire = func(ctx context.Context, host string, port int) (bool, error) {
		if host != "ob.example.test" || port != 60014 {
			t.Fatalf("probe target = %s:%d", host, port)
		}
		return true, nil
	}
	defer func() { probeOceanBaseMySQLWire = oldProbe }()

	cfg, err := dbipc.ConfigFromWire(map[string]any{
		"host":         "ob.example.test",
		"port":         float64(60014),
		"username":     "sys@test",
		"password":     "oracle",
		"service_name": "ORCL",
		"database":     "APP",
		"extra_params": map[string]any{
			"protocol": "oracle",
		},
	}, 2881)
	if err != nil {
		t.Fatalf("dbipc.ConfigFromWire returned error: %v", err)
	}

	connSpec, err := Spec().ResolveConnection(context.Background(), cfg)
	if err != nil {
		t.Fatalf("ResolveConnection returned error: %v", err)
	}

	if connSpec.DriverName != "oboracle" {
		t.Fatalf("driver = %q, want oboracle", connSpec.DriverName)
	}
	parsed, err := obconnector.ParseDSN(connSpec.DSN)
	if err != nil {
		t.Fatalf("obconnector.ParseDSN(%q) returned error: %v", connSpec.DSN, err)
	}
	if parsed.User != "sys@test" || parsed.Password != "oracle" {
		t.Fatalf("credentials = %q/%q", parsed.User, parsed.Password)
	}
	if parsed.Addr != "ob.example.test:60014" {
		t.Fatalf("address = %q, want ob.example.test:60014", parsed.Addr)
	}
	if parsed.Database != "" {
		t.Fatalf("database = %q, want empty; generic Database must not be sent in the Oracle-tenant handshake", parsed.Database)
	}
	if parsed.Preset != "oboracle" {
		t.Fatalf("preset = %q, want oboracle", parsed.Preset)
	}
	if connSpec.SchemaSQL.Databases == nil || !strings.Contains(connSpec.SchemaSQL.Databases(cfg), "SYS_CONTEXT('USERENV', 'CON_NAME')") {
		t.Fatalf("oracle mysql-wire protocol did not select Oracle metadata SQL")
	}
	if connSpec.IdentifierQuoteLeft != `"` || connSpec.IdentifierQuoteRight != `"` {
		t.Fatalf("identifier quotes = %q/%q, want Oracle quotes", connSpec.IdentifierQuoteLeft, connSpec.IdentifierQuoteRight)
	}
}

func TestSpecBuildsOracleTenantMySQLWireDSNWithoutHostManagedSSHOptions(t *testing.T) {
	cfg := ConfigFromWireNoError(t, map[string]any{
		"host":         "ob.example.test",
		"port":         float64(60014),
		"username":     "sys@test",
		"password":     "oracle",
		"service_name": "ORCL",
		"protocol":     "oracle",
		"extra_params": map[string]any{
			"charset":                  "utf8mb4",
			"cap.add":                  "0x80",
			"connectionAttributes":     "program_name:navop,tenant:oracle",
			"oracle_mysql_wire_driver": "oboracle-test",
			"ssh_target_host":          "db.internal",
			" SSH_PORT ":               22,
		},
	})

	dsn, err := buildMySQLWireOracleTenantDSN(cfg)
	if err != nil {
		t.Fatalf("buildMySQLWireOracleTenantDSN returned error: %v", err)
	}

	parsed, err := obconnector.ParseDSN(dsn)
	if err != nil {
		t.Fatalf("obconnector.ParseDSN(%q) returned error: %v", dsn, err)
	}
	if parsed.Database != "" {
		t.Fatalf("database = %q, want empty; service_name is only for the TNS path", parsed.Database)
	}
	if parsed.Preset != "oboracle" || parsed.CapabilityAdd != 0x80 {
		t.Fatalf("preset/capability = %q/%#x", parsed.Preset, parsed.CapabilityAdd)
	}
	for name, want := range map[string]string{
		"program_name": "navop",
		"tenant":       "oracle",
	} {
		if parsed.Attributes[name] != want {
			t.Fatalf("attribute %q = %q, want %q", name, parsed.Attributes[name], want)
		}
	}
	for _, unwanted := range []string{"charset", "oracle_mysql_wire_driver", "ssh_", "protocol="} {
		if strings.Contains(strings.ToLower(dsn), unwanted) {
			t.Fatalf("dsn %q contains host-only option %q", dsn, unwanted)
		}
	}
}

func TestSpecResolvesCaseInsensitiveOracleMySQLWireDriverOverride(t *testing.T) {
	cfg := ConfigFromWireNoError(t, map[string]any{
		"host":     "ob.example.test",
		"username": "sys@test",
		"protocol": "oracle",
		"extra_params": map[string]any{
			" Oracle_MySQL_Wire_Driver ": " oboracle-test ",
		},
	})

	if got := oracleMySQLWireDriverName(cfg); got != "oboracle-test" {
		t.Fatalf("driver = %q, want oboracle-test", got)
	}
}

func TestOBConnectorOracleTenantNeverSendsInitialDatabase(t *testing.T) {
	response := captureOBConnectorHandshakeResponse(t, 0x0004, obConnectorBaseServerCapabilities|clientConnectWithDB, obconnector.Config{
		User:          "sys@test",
		Password:      "oracle",
		Database:      "HSRCM_RCMP_15331",
		Preset:        "oboracle",
		CapabilityAdd: clientConnectWithDB,
	})

	parsed := parseOBConnectorHandshakeResponse(t, response)
	if parsed.capabilities&clientConnectWithDB != 0 {
		t.Fatalf("capabilities = %#x, CLIENT_CONNECT_WITH_DB must be disabled for Oracle tenants", parsed.capabilities)
	}
	if parsed.databasePresent {
		t.Fatalf("Oracle tenant handshake contains initial database %q", parsed.database)
	}
}

func TestOBConnectorOraclePresetDisablesInitialDatabaseWithoutStatusBit(t *testing.T) {
	response := captureOBConnectorHandshakeResponse(t, 0, obConnectorBaseServerCapabilities|clientConnectWithDB, obconnector.Config{
		User:          "sys@test",
		Password:      "oracle",
		Database:      "HSRCM_RCMP_15331",
		Preset:        "oboracle",
		CapabilityAdd: clientConnectWithDB,
	})

	parsed := parseOBConnectorHandshakeResponse(t, response)
	if parsed.capabilities&clientConnectWithDB != 0 {
		t.Fatalf("capabilities = %#x, oboracle preset must disable CLIENT_CONNECT_WITH_DB when the proxy omits the Oracle status bit", parsed.capabilities)
	}
	if parsed.databasePresent {
		t.Fatalf("oboracle preset handshake contains initial database %q", parsed.database)
	}
}

func TestBuiltOracleTenantDSNNeverSendsInitialDatabase(t *testing.T) {
	cfg := ConfigFromWireNoError(t, map[string]any{
		"host":     "ob.example.test",
		"port":     float64(60014),
		"username": "sys@test",
		"password": "oracle",
		"database": "HSRCM_RCMP_15331",
		"protocol": "oracle",
		"extra_params": map[string]any{
			"cap.add": "0x8",
		},
	})
	dsn, err := buildMySQLWireOracleTenantDSN(cfg)
	if err != nil {
		t.Fatalf("buildMySQLWireOracleTenantDSN returned error: %v", err)
	}
	connectorCfg, err := obconnector.ParseDSN(dsn)
	if err != nil {
		t.Fatalf("obconnector.ParseDSN(%q) returned error: %v", dsn, err)
	}

	response := captureOBConnectorHandshakeResponse(t, 0, obConnectorBaseServerCapabilities|clientConnectWithDB, *connectorCfg)
	parsed := parseOBConnectorHandshakeResponse(t, response)
	if parsed.capabilities&clientConnectWithDB != 0 {
		t.Fatalf("capabilities = %#x, generated oboracle DSN must not enable CLIENT_CONNECT_WITH_DB", parsed.capabilities)
	}
	if parsed.databasePresent {
		t.Fatalf("generated oboracle DSN handshake contains initial database %q", parsed.database)
	}
}

func TestOBConnectorMySQLTenantStillSendsInitialDatabase(t *testing.T) {
	response := captureOBConnectorHandshakeResponse(t, 0, obConnectorBaseServerCapabilities|clientConnectWithDB, obconnector.Config{
		User:     "root@test",
		Password: "secret",
		Database: "app",
	})

	parsed := parseOBConnectorHandshakeResponse(t, response)
	if parsed.capabilities&clientConnectWithDB == 0 {
		t.Fatalf("capabilities = %#x, CLIENT_CONNECT_WITH_DB must remain enabled for MySQL tenants", parsed.capabilities)
	}
	if !parsed.databasePresent || parsed.database != "app" {
		t.Fatalf("MySQL tenant initial database = %q (present=%v), want app", parsed.database, parsed.databasePresent)
	}
}

func TestOBConnectorCannotEnableInitialDatabaseUnsupportedByServer(t *testing.T) {
	response := captureOBConnectorHandshakeResponse(t, 0, obConnectorBaseServerCapabilities, obconnector.Config{
		User:          "root@test",
		Password:      "secret",
		Database:      "app",
		CapabilityAdd: clientConnectWithDB,
	})

	parsed := parseOBConnectorHandshakeResponse(t, response)
	if parsed.capabilities&clientConnectWithDB != 0 {
		t.Fatalf("capabilities = %#x, CLIENT_CONNECT_WITH_DB must not be invented when the server does not advertise it", parsed.capabilities)
	}
	if parsed.databasePresent {
		t.Fatalf("handshake contains server-unsupported initial database %q", parsed.database)
	}
}

func TestOBConnectorEmptyDatabaseCannotBeEnabledByCapabilityOverride(t *testing.T) {
	response := captureOBConnectorHandshakeResponse(t, 0, obConnectorBaseServerCapabilities|clientConnectWithDB, obconnector.Config{
		User:          "root@test",
		Password:      "secret",
		CapabilityAdd: clientConnectWithDB,
	})

	parsed := parseOBConnectorHandshakeResponse(t, response)
	if parsed.capabilities&clientConnectWithDB != 0 {
		t.Fatalf("capabilities = %#x, empty database must disable CLIENT_CONNECT_WITH_DB", parsed.capabilities)
	}
	if parsed.databasePresent {
		t.Fatalf("empty database handshake unexpectedly contains database %q", parsed.database)
	}
}

func TestProbeTreatsGenericMySQLHandshakeAsMySQLWireCandidate(t *testing.T) {
	address := serveProbePayload(t, mysqlHandshakePacket("8.0.36"))
	host, port := splitProbeAddress(t, address)

	isMySQLWire, err := probeOceanBaseMySQLWireHandshake(context.Background(), host, port)
	if err != nil {
		t.Fatalf("probe returned error: %v", err)
	}
	if !isMySQLWire {
		t.Fatal("generic MySQL handshake was not recognized as a MySQL-wire candidate")
	}
}

func TestProbeTreatsReachableListenerWithoutMySQLHandshakeAsNonMySQLWire(t *testing.T) {
	address := serveProbePayload(t, nil)
	host, port := splitProbeAddress(t, address)

	isMySQLWire, err := probeOceanBaseMySQLWireHandshake(context.Background(), host, port)
	if err != nil {
		t.Fatalf("probe returned error: %v", err)
	}
	if isMySQLWire {
		t.Fatal("listener without a MySQL handshake was classified as MySQL wire")
	}
}

func TestProbeRejectsNonMySQLPayloadWithVersionLikeString(t *testing.T) {
	payload := mysqlHandshakePayload("8.0.36")
	payload[0] = 0x7f
	address := serveProbePayload(t, mysqlPacket(payload))
	host, port := splitProbeAddress(t, address)

	isMySQLWire, err := probeOceanBaseMySQLWireHandshake(context.Background(), host, port)
	if err != nil {
		t.Fatalf("probe returned error: %v", err)
	}
	if isMySQLWire {
		t.Fatal("non-MySQL payload was classified as MySQL wire")
	}
}

func TestSpecResolvesOracleProtocolWithoutMySQLWireToGoOraDriver(t *testing.T) {
	oldProbe := probeOceanBaseMySQLWire
	probeOceanBaseMySQLWire = func(ctx context.Context, host string, port int) (bool, error) {
		return false, nil
	}
	defer func() { probeOceanBaseMySQLWire = oldProbe }()

	cfg := ConfigFromWireNoError(t, map[string]any{
		"host":         "obproxy.example.test",
		"port":         float64(1521),
		"username":     "system",
		"password":     "oracle",
		"service_name": "ORCL",
		"protocol":     "oracle",
	})

	connSpec, err := Spec().ResolveConnection(context.Background(), cfg)
	if err != nil {
		t.Fatalf("ResolveConnection returned error: %v", err)
	}

	if connSpec.DriverName != "oracle" {
		t.Fatalf("driver = %q, want oracle", connSpec.DriverName)
	}
	if !strings.HasPrefix(connSpec.DSN, "oracle://system:oracle@obproxy.example.test:1521/ORCL") {
		t.Fatalf("dsn = %q", connSpec.DSN)
	}
	if connSpec.SchemaSQL.Databases == nil || !strings.Contains(connSpec.SchemaSQL.Databases(cfg), "SYS_CONTEXT('USERENV', 'CON_NAME')") {
		t.Fatalf("oracle protocol did not select Oracle metadata SQL")
	}
	if connSpec.IdentifierQuoteLeft != `"` || connSpec.IdentifierQuoteRight != `"` {
		t.Fatalf("identifier quotes = %q/%q, want Oracle quotes", connSpec.IdentifierQuoteLeft, connSpec.IdentifierQuoteRight)
	}
}

func TestSpecBuildsOceanBaseMySQLMetadataSQL(t *testing.T) {
	cfg := ConfigFromWireNoError(t, map[string]any{
		"host":     "127.0.0.1",
		"username": "root@test",
		"database": "app",
	})
	spec := Spec()

	for name, sqlText := range map[string]string{
		"databases": spec.SchemaSQL.Databases(cfg),
		"schemas":   spec.SchemaSQL.Schemas(cfg, "app"),
		"objects":   spec.SchemaSQL.Objects(cfg, "app", "", []string{"table", "view"}),
		"columns":   spec.SchemaSQL.Columns(cfg, "app", "", "demo"),
		"indexes":   spec.SchemaSQL.Indexes(cfg, "app", "", "demo"),
		"views":     spec.SchemaSQL.Views(cfg, "app", ""),
	} {
		if !strings.Contains(sqlText, "INFORMATION_SCHEMA") {
			t.Fatalf("%s SQL %q does not query INFORMATION_SCHEMA", name, sqlText)
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

func TestSpecBuildsOceanBaseMySQLDumpDDL(t *testing.T) {
	cfg := ConfigFromWireNoError(t, map[string]any{
		"host":     "127.0.0.1",
		"username": "root@test",
		"database": "app",
	})
	spec := Spec()
	if spec.SchemaSQL.DumpDDL == nil {
		t.Fatalf("oceanbase Spec() must provide DumpDDL")
	}

	fromConfigDB := spec.SchemaSQL.DumpDDL(cfg, "", "", "demo")
	if fromConfigDB != "SHOW CREATE TABLE `app`.`demo`" {
		t.Fatalf("config-database DumpDDL SQL = %q", fromConfigDB)
	}

	fromSchema := spec.SchemaSQL.DumpDDL(cfg, "shop", "shop", "demo")
	if fromSchema != "SHOW CREATE TABLE `shop`.`demo`" {
		t.Fatalf("schema DumpDDL SQL = %q", fromSchema)
	}

	escaped := spec.SchemaSQL.DumpDDL(cfg, "", "", "we`ird")
	if !strings.Contains(escaped, "`we``ird`") {
		t.Fatalf("DumpDDL SQL %q does not escape a backtick", escaped)
	}

	quoted := spec.SchemaSQL.DumpDDL(cfg, "", "", "`demo`")
	if !strings.Contains(quoted, "`demo`") {
		t.Fatalf("DumpDDL SQL %q should strip identifier quotes around the table", quoted)
	}
}

func mysqlHandshakePacket(serverVersion string) []byte {
	return mysqlPacket(mysqlHandshakePayload(serverVersion))
}

func mysqlHandshakePayload(serverVersion string) []byte {
	payload := make([]byte, 34)
	payload[0] = 0x0a
	copy(payload[1:], serverVersion)
	payload[1+len(serverVersion)] = 0
	return payload
}

func mysqlPacket(payload []byte) []byte {
	packet := make([]byte, 4+len(payload))
	packet[0] = byte(len(payload))
	packet[1] = byte(len(payload) >> 8)
	packet[2] = byte(len(payload) >> 16)
	copy(packet[4:], payload)
	return packet
}

func serveProbePayload(t *testing.T, payload []byte) string {
	t.Helper()
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("net.Listen returned error: %v", err)
	}
	t.Cleanup(func() { _ = listener.Close() })

	go func() {
		conn, err := listener.Accept()
		if err != nil {
			return
		}
		defer conn.Close()
		if len(payload) > 0 {
			_, _ = conn.Write(payload)
		}
	}()
	return listener.Addr().String()
}

func splitProbeAddress(t *testing.T, address string) (string, int) {
	t.Helper()
	host, rawPort, err := net.SplitHostPort(address)
	if err != nil {
		t.Fatalf("net.SplitHostPort(%q) returned error: %v", address, err)
	}
	port, err := strconv.Atoi(rawPort)
	if err != nil {
		t.Fatalf("parse port %q: %v", rawPort, err)
	}
	return host, port
}

func captureOBConnectorHandshakeResponse(t *testing.T, status uint16, serverCapabilities uint32, cfg obconnector.Config) []byte {
	t.Helper()
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("net.Listen returned error: %v", err)
	}
	t.Cleanup(func() { _ = listener.Close() })

	responseCh := make(chan []byte, 1)
	serverErrCh := make(chan error, 1)
	go func() {
		conn, err := listener.Accept()
		if err != nil {
			serverErrCh <- err
			return
		}
		defer conn.Close()

		if err := writeMySQLPacket(conn, 0, obConnectorHandshakePayload(status, serverCapabilities)); err != nil {
			serverErrCh <- fmt.Errorf("write server handshake: %w", err)
			return
		}
		response, sequence, err := readMySQLPacket(conn)
		if err != nil {
			serverErrCh <- fmt.Errorf("read client handshake: %w", err)
			return
		}
		if sequence != 1 {
			serverErrCh <- fmt.Errorf("client handshake sequence = %d, want 1", sequence)
			return
		}
		responseCh <- response
		if err := writeMySQLPacket(conn, 2, []byte{0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00}); err != nil {
			serverErrCh <- fmt.Errorf("write auth OK: %w", err)
		}
	}()

	cfg.Addr = listener.Addr().String()
	cfg.Addrs = []string{cfg.Addr}
	connector, err := obconnector.NewConnector(cfg)
	if err != nil {
		t.Fatalf("obconnector.NewConnector returned error: %v", err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	driverConn, err := connector.Connect(ctx)
	if err != nil {
		select {
		case serverErr := <-serverErrCh:
			t.Fatalf("connector.Connect returned error: %v (server: %v)", err, serverErr)
		default:
			t.Fatalf("connector.Connect returned error: %v", err)
		}
	}
	defer driverConn.Close()

	select {
	case response := <-responseCh:
		if len(response) < 4 {
			t.Fatalf("client handshake response length = %d, want at least 4", len(response))
		}
		return response
	case serverErr := <-serverErrCh:
		t.Fatalf("fake OceanBase server returned error: %v", serverErr)
	case <-ctx.Done():
		t.Fatalf("timed out waiting for client handshake response: %v", ctx.Err())
	}
	return nil
}

func obConnectorHandshakePayload(status uint16, serverCapabilities uint32) []byte {
	payload := []byte{0x0a}
	payload = append(payload, "5.7.25-OceanBase"...)
	payload = append(payload, 0x00)
	payload = binary.LittleEndian.AppendUint32(payload, 1)
	payload = append(payload, "12345678"...)
	payload = append(payload, 0x00)
	payload = binary.LittleEndian.AppendUint16(payload, uint16(serverCapabilities&0xffff))
	payload = append(payload, 45)
	payload = binary.LittleEndian.AppendUint16(payload, status)
	payload = binary.LittleEndian.AppendUint16(payload, uint16(serverCapabilities>>16))
	payload = append(payload, 21)
	payload = append(payload, make([]byte, 10)...)
	payload = append(payload, "abcdefghijkl"...)
	payload = append(payload, 0x00)
	payload = append(payload, "mysql_native_password"...)
	payload = append(payload, 0x00)
	return payload
}

type obConnectorHandshakeResponse struct {
	capabilities    uint32
	database        string
	databasePresent bool
}

func parseOBConnectorHandshakeResponse(t *testing.T, response []byte) obConnectorHandshakeResponse {
	t.Helper()
	const fixedHeaderLength = 4 + 4 + 1 + 19 + 4
	if len(response) < fixedHeaderLength {
		t.Fatalf("client handshake response length = %d, want at least %d", len(response), fixedHeaderLength)
	}

	parsed := obConnectorHandshakeResponse{
		capabilities: binary.LittleEndian.Uint32(response[:4]),
	}
	pos := fixedHeaderLength
	_, pos = readNullTerminatedHandshakeField(t, response, pos, "username")
	_, pos = readLengthEncodedHandshakeField(t, response, pos, "auth response")
	if parsed.capabilities&clientConnectWithDB != 0 {
		parsed.database, pos = readNullTerminatedHandshakeField(t, response, pos, "database")
		parsed.databasePresent = true
	}
	if pos > len(response) {
		t.Fatalf("parsed client handshake beyond payload: position %d, length %d", pos, len(response))
	}
	return parsed
}

func readNullTerminatedHandshakeField(t *testing.T, payload []byte, pos int, name string) (string, int) {
	t.Helper()
	if pos >= len(payload) {
		t.Fatalf("client handshake missing %s at position %d", name, pos)
	}
	start := pos
	for pos < len(payload) && payload[pos] != 0 {
		pos++
	}
	if pos >= len(payload) {
		t.Fatalf("client handshake %s is not NUL-terminated", name)
	}
	return string(payload[start:pos]), pos + 1
}

func readLengthEncodedHandshakeField(t *testing.T, payload []byte, pos int, name string) ([]byte, int) {
	t.Helper()
	length, pos := readLengthEncodedHandshakeInt(t, payload, pos, name)
	if length > uint64(len(payload)-pos) {
		t.Fatalf("client handshake %s length = %d, remaining payload = %d", name, length, len(payload)-pos)
	}
	end := pos + int(length)
	return payload[pos:end], end
}

func readLengthEncodedHandshakeInt(t *testing.T, payload []byte, pos int, name string) (uint64, int) {
	t.Helper()
	if pos >= len(payload) {
		t.Fatalf("client handshake missing %s length at position %d", name, pos)
	}
	switch first := payload[pos]; first {
	case 0xfc:
		if pos+3 > len(payload) {
			t.Fatalf("client handshake has truncated 2-byte %s length", name)
		}
		return uint64(binary.LittleEndian.Uint16(payload[pos+1 : pos+3])), pos + 3
	case 0xfd:
		if pos+4 > len(payload) {
			t.Fatalf("client handshake has truncated 3-byte %s length", name)
		}
		length := uint64(payload[pos+1]) |
			uint64(payload[pos+2])<<8 |
			uint64(payload[pos+3])<<16
		return length, pos + 4
	case 0xfe:
		if pos+9 > len(payload) {
			t.Fatalf("client handshake has truncated 8-byte %s length", name)
		}
		return binary.LittleEndian.Uint64(payload[pos+1 : pos+9]), pos + 9
	case 0xfb:
		t.Fatalf("client handshake uses NULL length for %s", name)
	default:
		return uint64(first), pos + 1
	}
	return 0, 0
}

func readMySQLPacket(conn net.Conn) ([]byte, byte, error) {
	var header [4]byte
	if _, err := io.ReadFull(conn, header[:]); err != nil {
		return nil, 0, err
	}
	length := int(header[0]) | int(header[1])<<8 | int(header[2])<<16
	payload := make([]byte, length)
	if _, err := io.ReadFull(conn, payload); err != nil {
		return nil, 0, err
	}
	return payload, header[3], nil
}

func writeMySQLPacket(conn net.Conn, sequence byte, payload []byte) error {
	packet := make([]byte, 4+len(payload))
	packet[0] = byte(len(payload))
	packet[1] = byte(len(payload) >> 8)
	packet[2] = byte(len(payload) >> 16)
	packet[3] = sequence
	copy(packet[4:], payload)
	_, err := conn.Write(packet)
	return err
}
