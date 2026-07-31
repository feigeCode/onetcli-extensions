package iotdb

import (
	"context"
	"testing"

	"navop-db-ipc-drivers/internal/dbipc"
)

func TestSchemaObjectViewFunctionsReturnsRows(t *testing.T) {
	server := NewServer()
	ctx := context.Background()
	mustOK(t, server.Handle(ctx, message(1, "init", map[string]any{"host_version": "0.10.0"})))
	server.conns[1] = &connection{cfg: dbipc.Config{Database: "root.navop_smoke"}}

	resp := server.Handle(ctx, message(2, "schema/object_view", map[string]any{
		"conn_id": 1,
		"view":    "functions",
	}))

	result := resultMap(t, resp)
	if result["title"] != "Functions" {
		t.Fatalf("title = %#v, want Functions", result["title"])
	}
	rows := result["rows"].([]any)
	if len(rows) == 0 {
		t.Fatalf("rows should not be empty")
	}
}
