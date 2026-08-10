package oceanbase

import (
	"context"
	"net"
	"strconv"
	"strings"
	"testing"

	obconnector "github.com/helingjun/obconnector-go"
	"navop-db-ipc-drivers/internal/dbipc"
)

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
	if parsed.Addr != "ob.example.test:60014" || parsed.Database != "APP" {
		t.Fatalf("target = %q/%q", parsed.Addr, parsed.Database)
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
