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
