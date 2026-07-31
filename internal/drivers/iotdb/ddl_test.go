package iotdb

import (
	"context"
	"testing"
)

func TestDdlBuildCreateDatabaseBuildsStorageGroup(t *testing.T) {
	server := NewServer()
	ctx := context.Background()
	mustOK(t, server.Handle(ctx, message(1, "init", map[string]any{"host_version": "0.10.0"})))

	resp := server.Handle(ctx, message(2, "ddl/build", map[string]any{
		"op": "create_database",
		"payload": map[string]any{
			"database_name": "root.navop_smoke",
		},
	}))

	result := resultMap(t, resp)
	statements := result["statements"].([]any)
	if len(statements) != 1 || statements[0] != "SET STORAGE GROUP TO root.navop_smoke" {
		t.Fatalf("statements = %#v, want storage group creation", statements)
	}
}

func TestDdlBuildDropDatabaseBuildsStorageGroupDelete(t *testing.T) {
	server := NewServer()
	ctx := context.Background()
	mustOK(t, server.Handle(ctx, message(1, "init", map[string]any{"host_version": "0.10.0"})))

	resp := server.Handle(ctx, message(2, "ddl/build", map[string]any{
		"op": "drop_database",
		"payload": map[string]any{
			"name": "root.navop_smoke",
		},
	}))

	result := resultMap(t, resp)
	statements := result["statements"].([]any)
	if len(statements) != 1 || statements[0] != "DELETE STORAGE GROUP root.navop_smoke" {
		t.Fatalf("statements = %#v, want storage group deletion", statements)
	}
}
