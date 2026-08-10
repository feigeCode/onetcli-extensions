package dbipc

import (
	"reflect"
	"testing"
)

func TestCopyDriverExtraExcludesHostManagedSSHParams(t *testing.T) {
	extra := map[string]string{
		"application_name": "navop",
		"ssh_auth_type":    "password",
		" SSH_PORT ":       "22",
	}
	original := CopyExtra(extra)

	got := CopyDriverExtra(extra)
	want := map[string]string{
		"application_name": "navop",
	}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("CopyDriverExtra() = %#v, want %#v", got, want)
	}
	if !reflect.DeepEqual(extra, original) {
		t.Fatalf("CopyDriverExtra modified input: got %#v, want %#v", extra, original)
	}
}

func TestCopyDriverExtraHandlesNilAndEmptyMaps(t *testing.T) {
	for name, extra := range map[string]map[string]string{
		"nil":   nil,
		"empty": {},
	} {
		t.Run(name, func(t *testing.T) {
			got := CopyDriverExtra(extra)
			if got == nil {
				t.Fatal("CopyDriverExtra returned nil map")
			}
			if len(got) != 0 {
				t.Fatalf("CopyDriverExtra() = %#v, want empty map", got)
			}
		})
	}
}

func TestConfigFromWirePromotesProtocolFromExtraParams(t *testing.T) {
	cfg, err := ConfigFromWire(map[string]any{
		"host": "127.0.0.1",
		"extra_params": map[string]any{
			"protocol": "oracle",
			"trace":    true,
		},
	}, 2881)
	if err != nil {
		t.Fatalf("ConfigFromWire returned error: %v", err)
	}

	if cfg.Protocol != "oracle" {
		t.Fatalf("Protocol = %q, want oracle", cfg.Protocol)
	}
	if _, exists := cfg.Extra["protocol"]; exists {
		t.Fatalf("protocol leaked into Extra: %#v", cfg.Extra)
	}
	if cfg.Extra["trace"] != "true" {
		t.Fatalf("Extra[trace] = %q, want true", cfg.Extra["trace"])
	}
}

func TestConfigFromWireTopLevelProtocolTakesPrecedence(t *testing.T) {
	cfg, err := ConfigFromWire(map[string]any{
		"protocol": "oracle",
		"extra_params": map[string]any{
			" Protocol ": "mysql",
		},
	}, 2881)
	if err != nil {
		t.Fatalf("ConfigFromWire returned error: %v", err)
	}

	if cfg.Protocol != "oracle" {
		t.Fatalf("Protocol = %q, want top-level oracle", cfg.Protocol)
	}
	if len(cfg.Extra) != 0 {
		t.Fatalf("protocol control field leaked into Extra: %#v", cfg.Extra)
	}
}
