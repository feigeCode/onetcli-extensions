package ipc

import "testing"

func TestWindowsPipePathUsesLocalNamedPipeNamespace(t *testing.T) {
	got := windowsPipePath("onetcli-ext-123")
	want := `\\.\pipe\onetcli-ext-123`
	if got != want {
		t.Fatalf("windowsPipePath() = %q, want %q", got, want)
	}
}
