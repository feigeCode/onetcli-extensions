package dbipc

import (
	"context"
	"fmt"
	"net/url"
	"sort"
	"strconv"
)

type Config struct {
	Host     string
	Port     int
	Username string
	Password string
	Database string
	Service  string
	SID      string
	Protocol string
	Extra    map[string]string
}

type DriverSpec struct {
	ID                   string
	Name                 string
	SQLDriverName        string
	DefaultPort          int
	IdentifierQuoteLeft  string
	IdentifierQuoteRight string
	BuildDSN             func(Config) (string, error)
	ResolveConnection    func(context.Context, Config) (ConnectionSpec, error)
	SchemaSQL            SchemaSQL
}

type ConnectionSpec struct {
	DriverName string
	DSN        string
	SchemaSQL  SchemaSQL
}

type SchemaSQL struct {
	Databases      func(Config) string
	Schemas        func(Config, string) string
	Objects        func(Config, string, string, []string) string
	Columns        func(Config, string, string, string) string
	Indexes        func(Config, string, string, string) string
	ForeignKeys    func(Config, string, string, string) string
	Views          func(Config, string, string) string
	Functions      func(Config, string, string) string
	ViewDefinition func(Config, string, string, string) string
}

func ConfigFromWire(raw map[string]any, defaultPort int) (Config, error) {
	cfg := Config{
		Host:  stringValue(raw, "host"),
		Port:  intValue(raw, "port"),
		Extra: map[string]string{},
	}
	if cfg.Port == 0 {
		cfg.Port = defaultPort
	}
	cfg.Username = stringValue(raw, "username")
	cfg.Password = stringValue(raw, "password")
	cfg.Database = stringValue(raw, "database")
	cfg.Service = stringValue(raw, "service_name")
	cfg.SID = stringValue(raw, "sid")
	cfg.Protocol = stringValue(raw, "protocol")

	if extra, ok := raw["extra_params"].(map[string]any); ok {
		for k, v := range extra {
			if v == nil {
				continue
			}
			cfg.Extra[k] = fmt.Sprint(v)
		}
	}
	return cfg, nil
}

func RequireConfig(cfg Config, fields ...string) error {
	for _, field := range fields {
		switch field {
		case "host":
			if cfg.Host == "" {
				return fmt.Errorf("missing required config field host")
			}
		case "port":
			if cfg.Port == 0 {
				return fmt.Errorf("missing required config field port")
			}
		case "username":
			if cfg.Username == "" {
				return fmt.Errorf("missing required config field username")
			}
		case "database":
			if cfg.Database == "" {
				return fmt.Errorf("missing required config field database")
			}
		default:
			return fmt.Errorf("unknown required config field %s", field)
		}
	}
	return nil
}

func QueryString(extra map[string]string) string {
	if len(extra) == 0 {
		return ""
	}
	values := url.Values{}
	keys := sortedKeys(extra)
	for _, key := range keys {
		values.Set(key, extra[key])
	}
	return values.Encode()
}

func CopyExtra(extra map[string]string) map[string]string {
	out := make(map[string]string, len(extra))
	for k, v := range extra {
		out[k] = v
	}
	return out
}

func sortedKeys(m map[string]string) []string {
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	return keys
}

func stringValue(raw map[string]any, key string) string {
	value, ok := raw[key]
	if !ok || value == nil {
		return ""
	}
	return fmt.Sprint(value)
}

func intValue(raw map[string]any, key string) int {
	value, ok := raw[key]
	if !ok || value == nil {
		return 0
	}
	switch v := value.(type) {
	case int:
		return v
	case int64:
		return int(v)
	case float64:
		return int(v)
	case string:
		n, _ := strconv.Atoi(v)
		return n
	default:
		return 0
	}
}
