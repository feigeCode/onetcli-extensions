package oceanbase

import (
	"crypto/tls"
	"crypto/x509"
	"errors"
	"fmt"
	"io"
	"net"
	"net/url"
	"os"
	"strconv"
	"strings"
	"time"
)

const defaultAddr = "127.0.0.1:2881"

type Config struct {
	Addrs          []string
	Addr           string
	User           string
	Password       string
	Database       string
	Timeout        time.Duration
	Attributes     map[string]string
	CapabilityAdd  uint32
	CapabilityDrop uint32
	Collation      byte
	InitSQL        []string
	Preset         string
	Trace          bool
	TraceWriter    io.Writer
	ProtocolV2         bool
	OB20Magic          uint16
	DisableOB20Checksum bool
	TLSConfig      *tls.Config
	UseCompression bool
}

func ParseDSN(dsn string) (*Config, error) {
	if strings.Contains(dsn, "://") {
		return parseURLDSN(dsn)
	}
	if strings.HasPrefix(dsn, "oceanbase:") || strings.HasPrefix(dsn, "oboracle:") {
		return parseOpaqueDSN(dsn)
	}
	return parseLegacyDSN(dsn)
}

func (c *Config) normalize() error {
	if len(c.Addrs) == 0 && c.Addr != "" {
		c.Addrs = []string{c.Addr}
	} else if len(c.Addrs) > 0 && c.Addr == "" {
		c.Addr = c.Addrs[0]
	}

	if c.Addr == "" {
		c.Addr = defaultAddr
	}
	if len(c.Addrs) == 0 {
		c.Addrs = []string{c.Addr}
	}

	normalizedAddrs := make([]string, len(c.Addrs))
	for i, addr := range c.Addrs {
		host, port, err := net.SplitHostPort(addr)
		if err != nil {
			if strings.Contains(addr, ":") {
				return fmt.Errorf("invalid address %q: %w", addr, err)
			}
			host = addr
			port = "2881"
		}
		if host == "" {
			host = "127.0.0.1"
		}
		if _, err := strconv.Atoi(port); err != nil {
			return fmt.Errorf("invalid port %q: %w", port, err)
		}
		normalizedAddrs[i] = net.JoinHostPort(host, port)
	}
	c.Addrs = normalizedAddrs
	c.Addr = c.Addrs[0]

	if c.User == "" {
		return errors.New("missing user")
	}
	if c.Timeout == 0 {
		if envTimeout := os.Getenv("OB_TIMEOUT"); envTimeout != "" {
			if d, err := parseTimeout(envTimeout); err == nil {
				c.Timeout = d
			}
		}
	}
	if c.Timeout == 0 {
		c.Timeout = 10 * time.Second
	}
	if c.Attributes == nil {
		c.Attributes = map[string]string{}
	}
	if c.Collation == 0 {
		c.Collation = DefaultCollation()
	}
	if c.Preset == "" {
		c.Preset = "default"
	}
	if c.Trace && c.TraceWriter == nil {
		c.TraceWriter = os.Stderr
	}
	return nil
}

func parseURLDSN(dsn string) (*Config, error) {
	u, err := url.Parse(dsn)
	if err != nil {
		return nil, err
	}
	if u.Scheme != "oceanbase" && u.Scheme != "oboracle" {
		return nil, fmt.Errorf("unsupported DSN scheme %q", u.Scheme)
	}

	cfg := &Config{
		Addr:           u.Host,
		Database:       strings.TrimPrefix(u.EscapedPath(), "/"),
		Attributes:     map[string]string{},
		UseCompression: false,
	}
	if u.Scheme == "oboracle" {
		cfg.Preset = "oboracle"
	}
	if user := u.User; user != nil {
		cfg.User = user.Username()
		cfg.Password, _ = user.Password()
	}
	if db, err := url.PathUnescape(cfg.Database); err == nil {
		cfg.Database = db
	}

	if hostPart := u.Host; strings.Contains(hostPart, ",") {
		cfg.Addrs = strings.Split(hostPart, ",")
		cfg.Addr = cfg.Addrs[0]
	}

	if err := applyQuery(cfg, u.Query()); err != nil {
		return nil, err
	}

	return cfg, cfg.normalize()
}

func parseOpaqueDSN(dsn string) (*Config, error) {
	_, rest, _ := strings.Cut(dsn, ":")
	main, rawQuery, _ := strings.Cut(rest, "?")
	userInfo, hostAndPath, ok := strings.Cut(main, "@")
	if !ok {
		return nil, errors.New("opaque dsn must be oceanbase:user:pass@host:port/db")
	}

	rawUser, rawPassword, ok := strings.Cut(userInfo, ":")
	if !ok {
		return nil, errors.New("opaque dsn must include user and password")
	}
	user, err := url.QueryUnescape(rawUser)
	if err != nil {
		return nil, fmt.Errorf("invalid user escape: %w", err)
	}
	password, err := url.QueryUnescape(rawPassword)
	if err != nil {
		return nil, fmt.Errorf("invalid password escape: %w", err)
	}

	addr, rawDB, _ := strings.Cut(hostAndPath, "/")
	database, err := url.QueryUnescape(rawDB)
	if err != nil {
		return nil, fmt.Errorf("invalid database escape: %w", err)
	}
	cfg := &Config{
		Addr:           addr,
		User:           user,
		Password:       password,
		Database:       database,
		Attributes:     map[string]string{},
		UseCompression: false,
	}
	if strings.Contains(addr, ",") {
		cfg.Addrs = strings.Split(addr, ",")
	}
	if strings.HasPrefix(dsn, "oboracle:") {
		cfg.Preset = "oboracle"
	}
	if rawQuery != "" {
		values, err := url.ParseQuery(rawQuery)
		if err != nil {
			return nil, err
		}
		if err := applyQuery(cfg, values); err != nil {
			return nil, err
		}
	}
	return cfg, cfg.normalize()
}

func parseLegacyDSN(dsn string) (*Config, error) {
	cfg := &Config{Addr: defaultAddr, Attributes: map[string]string{}, UseCompression: false}
	before, after, ok := strings.Cut(dsn, "@tcp(")
	if !ok {
		return nil, errors.New("dsn must be oceanbase://user:pass@host:port/db or user:pass@tcp(host:port)/db")
	}
	if user, password, ok := strings.Cut(before, ":"); ok {
		if u, err := url.QueryUnescape(user); err == nil {
			cfg.User = u
		} else {
			cfg.User = user
		}
		if p, err := url.QueryUnescape(password); err == nil {
			cfg.Password = p
		} else {
			cfg.Password = password
		}
	} else {
		if u, err := url.QueryUnescape(before); err == nil {
			cfg.User = u
		} else {
			cfg.User = before
		}
	}

	addr, rest, ok := strings.Cut(after, ")")
	if !ok {
		return nil, errors.New("legacy dsn missing closing )")
	}
	cfg.Addr = addr
	if strings.Contains(addr, ",") {
		cfg.Addrs = strings.Split(addr, ",")
	}
	if strings.HasPrefix(rest, "/") {
		pathAndQuery := strings.TrimPrefix(rest, "/")
		if db, query, ok := strings.Cut(pathAndQuery, "?"); ok {
			cfg.Database = db
			if err := applyLegacyQuery(cfg, query); err != nil {
				return nil, err
			}
		} else {
			cfg.Database = pathAndQuery
		}
	}
	return cfg, cfg.normalize()
}

func applyLegacyQuery(cfg *Config, raw string) error {
	values, err := url.ParseQuery(raw)
	if err != nil {
		return err
	}
	return applyQuery(cfg, values)
}

func applyQuery(cfg *Config, values url.Values) error {
	if timeout := getQueryValue(values, "timeout", "TIMEOUT", "CONNECT TIMEOUT", "connect timeout"); timeout != "" {
		d, err := parseTimeout(timeout)
		if err != nil {
			return fmt.Errorf("invalid timeout: %w", err)
		}
		cfg.Timeout = d
	}
	if trace := getQueryValue(values, "trace"); trace != "" {
		enabled, err := strconv.ParseBool(trace)
		if err != nil {
			return fmt.Errorf("invalid trace: %w", err)
		}
		cfg.Trace = enabled
	}
	if capAdd := getQueryValue(values, "cap.add"); capAdd != "" {
		v, err := parseUint32(capAdd)
		if err != nil {
			return fmt.Errorf("invalid cap.add: %w", err)
		}
		cfg.CapabilityAdd = v
	}
	if capDrop := getQueryValue(values, "cap.drop"); capDrop != "" {
		v, err := parseUint32(capDrop)
		if err != nil {
			return fmt.Errorf("invalid cap.drop: %w", err)
		}
		cfg.CapabilityDrop = v
	}
	if collation := getQueryValue(values, "collation"); collation != "" {
		v, err := strconv.ParseUint(collation, 0, 8)
		if err != nil {
			return fmt.Errorf("invalid collation: %w", err)
		}
		cfg.Collation = byte(v)
	}
	if preset := getQueryValue(values, "preset"); preset != "" {
		cfg.Preset = preset
	}
	if v2 := getQueryValue(values, "ob20", "protocol.v2"); v2 != "" {
		enabled, err := strconv.ParseBool(v2)
		if err != nil {
			return fmt.Errorf("invalid ob20: %w", err)
		}
		cfg.ProtocolV2 = enabled
	}
	if magic := getQueryValue(values, "ob20.magic"); magic != "" {
		v, err := strconv.ParseUint(magic, 0, 16)
		if err != nil {
			return fmt.Errorf("invalid ob20.magic: %w", err)
		}
		cfg.OB20Magic = uint16(v)
	}
	if disableChecksum := getQueryValue(values, "ob20.disableChecksum"); disableChecksum != "" {
		enabled, err := strconv.ParseBool(disableChecksum)
		if err != nil {
			return fmt.Errorf("invalid ob20.disableChecksum: %w", err)
		}
		cfg.DisableOB20Checksum = enabled
	}
	if compress := getQueryValue(values, "useCompression", "compress", "use_compression"); compress != "" {
		enabled, err := strconv.ParseBool(compress)
		if err != nil {
			return fmt.Errorf("invalid compress: %w", err)
		}
		cfg.UseCompression = enabled
	}
	if tlsVal := getQueryValue(values, "tls"); tlsVal != "" {
		switch tlsVal {
		case "true":
			cfg.TLSConfig = &tls.Config{}
		case "skip-verify":
			cfg.TLSConfig = &tls.Config{InsecureSkipVerify: true}
		case "false":
			cfg.TLSConfig = nil
		default:
			return fmt.Errorf("unsupported tls value %q", tlsVal)
		}
	}
	if tlsCA := getQueryValue(values, "tls.ca", "tls_ca"); tlsCA != "" {
		pemData, err := os.ReadFile(tlsCA)
		if err != nil {
			return fmt.Errorf("invalid tls.ca: %w", err)
		}
		rootPool := x509.NewCertPool()
		if !rootPool.AppendCertsFromPEM(pemData) {
			return errors.New("invalid tls.ca: no valid certificate found")
		}
		if cfg.TLSConfig == nil {
			cfg.TLSConfig = &tls.Config{}
		}
		cfg.TLSConfig.RootCAs = rootPool
	}
	if tlsCert := getQueryValue(values, "tls.cert", "tls_cert"); tlsCert != "" {
		tlsKey := getQueryValue(values, "tls.key", "tls_key")
		if tlsKey == "" {
			return errors.New("tls.cert requires tls.key")
		}
		cert, err := tls.LoadX509KeyPair(tlsCert, tlsKey)
		if err != nil {
			return fmt.Errorf("invalid tls.cert/tls.key: %w", err)
		}
		if cfg.TLSConfig == nil {
			cfg.TLSConfig = &tls.Config{}
		}
		cfg.TLSConfig.Certificates = append(cfg.TLSConfig.Certificates, cert)
	}
	cfg.InitSQL = append(cfg.InitSQL, values["init"]...)
	for key, vals := range values {
		if strings.HasPrefix(key, "attr.") && len(vals) > 0 {
			cfg.Attributes[strings.TrimPrefix(key, "attr.")] = vals[len(vals)-1]
		}
	}
	return nil
}

func getQueryValue(values url.Values, names ...string) string {
	for _, name := range names {
		if vals, ok := values[name]; ok && len(vals) > 0 {
			return vals[len(vals)-1]
		}
		for key, vals := range values {
			if strings.EqualFold(key, name) && len(vals) > 0 {
				return vals[len(vals)-1]
			}
		}
	}
	return ""
}

func parseTimeout(s string) (time.Duration, error) {
	if d, err := time.ParseDuration(s); err == nil {
		return d, nil
	}
	seconds, err := strconv.ParseFloat(s, 64)
	if err != nil {
		return 0, err
	}
	return time.Duration(seconds * float64(time.Second)), nil
}

func parseUint32(s string) (uint32, error) {
	v, err := strconv.ParseUint(s, 0, 32)
	return uint32(v), err
}

func DefaultCollation() byte {
	return 45
}
