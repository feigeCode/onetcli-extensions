package dbipc

import (
	"encoding/json"
	"testing"
)

func TestValidateHostVersion(t *testing.T) {
	tests := []struct {
		name    string
		params  json.RawMessage
		wantErr bool
	}{
		{name: "minimum", params: json.RawMessage(`{"host_version":"0.10.0"}`)},
		{name: "newer stable", params: json.RawMessage(`{"host_version":"0.10.1"}`)},
		{name: "new major", params: json.RawMessage(`{"host_version":"1.0.0"}`)},
		{name: "missing params", params: nil, wantErr: true},
		{name: "missing version", params: json.RawMessage(`{}`), wantErr: true},
		{name: "invalid version", params: json.RawMessage(`{"host_version":"test"}`), wantErr: true},
		{name: "older version", params: json.RawMessage(`{"host_version":"0.9.9"}`), wantErr: true},
		{name: "prerelease of minimum", params: json.RawMessage(`{"host_version":"0.10.0-alpha"}`), wantErr: true},
		{name: "prerelease of newer version", params: json.RawMessage(`{"host_version":"0.10.1-alpha.1"}`), wantErr: true},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			err := ValidateHostVersion(test.params)
			if test.wantErr && err == nil {
				t.Fatal("ValidateHostVersion() succeeded, want error")
			}
			if !test.wantErr && err != nil {
				t.Fatalf("ValidateHostVersion() returned error: %v", err)
			}
		})
	}
}
