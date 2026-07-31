package iotdb

import (
	"context"
	"encoding/json"
	"os"
	"testing"

	"navop-db-ipc-drivers/internal/ipc"
)

func TestLocalIoTDBQueryTimestamps(t *testing.T) {
	if os.Getenv("ONETCLI_IOTDB_INTEGRATION") != "1" {
		t.Skip("set ONETCLI_IOTDB_INTEGRATION=1 to run local IoTDB integration test")
	}

	server := NewServer()
	ctx := context.Background()
	mustOK(t, server.Handle(ctx, message(1, "init", map[string]any{"host_version": "0.10.0"})))

	open := server.Handle(ctx, message(2, "conn/open", map[string]any{
		"driver_id": "iotdb",
		"config": map[string]any{
			"host":     "127.0.0.1",
			"port":     6667,
			"username": "root",
			"password": "root",
			"database": "root.navop_go_switch",
		},
	}))
	connID := uint64(resultMap(t, open)["conn_id"].(float64))

	// The storage group may not exist on the first run.
	_ = server.Handle(ctx, message(3, "exec/run", map[string]any{
		"conn_id": connID,
		"sql":     "DROP DATABASE root.navop_go_switch",
	}))
	mustOK(t, server.Handle(ctx, message(4, "exec/run", map[string]any{
		"conn_id": connID,
		"sql":     "CREATE DATABASE root.navop_go_switch",
	})))
	mustOK(t, server.Handle(ctx, message(5, "exec/run", map[string]any{
		"conn_id": connID,
		"sql":     "CREATE TIMESERIES root.navop_go_switch.d1.temperature WITH DATATYPE=FLOAT, ENCODING=PLAIN, COMPRESSOR=SNAPPY",
	})))
	mustOK(t, server.Handle(ctx, message(6, "exec/run", map[string]any{
		"conn_id": connID,
		"sql":     "CREATE TIMESERIES root.navop_go_switch.d1.status WITH DATATYPE=BOOLEAN, ENCODING=PLAIN, COMPRESSOR=SNAPPY",
	})))
	mustOK(t, server.Handle(ctx, message(7, "exec/run", map[string]any{
		"conn_id": connID,
		"sql":     "INSERT INTO d1 (Time, temperature, status) VALUES ('1', '1.0', 'true')",
	})))
	mustOK(t, server.Handle(ctx, message(8, "exec/run", map[string]any{
		"conn_id": connID,
		"sql":     "INSERT INTO d1 (Time, temperature, status) VALUES ('2', '2.0', 'false')",
	})))

	start := server.Handle(ctx, message(9, "query/start", map[string]any{
		"conn_id": connID,
		"sql":     "SELECT temperature, status FROM d1",
	}))
	cursorID := resultMap(t, start)["cursor_id"].(string)
	fetch := server.Handle(ctx, message(10, "cursor/fetch", map[string]any{
		"cursor_id": cursorID,
		"n":         10,
	}))
	rows := resultMap(t, fetch)["rows"].([]any)
	if len(rows) != 2 {
		t.Fatalf("rows len = %d, want 2; rows=%#v", len(rows), rows)
	}
	got := []int{timeCell(t, rows[0]), timeCell(t, rows[1])}
	want := []int{1, 2}
	if got[0] != want[0] || got[1] != want[1] {
		t.Fatalf("Time values = %#v, want %#v; rows=%#v", got, want, rows)
	}

	mustOK(t, server.Handle(ctx, message(11, "conn/close", map[string]any{"conn_id": connID})))
}

func message(id int, method string, params any) ipc.Message {
	var raw json.RawMessage
	if params == nil {
		raw = json.RawMessage(`null`)
	} else {
		raw, _ = json.Marshal(params)
	}
	idRaw, _ := json.Marshal(id)
	return ipc.Message{JSONRPC: ipc.JSONRPCVersion, ID: idRaw, Method: method, Params: raw}
}

func mustOK(t *testing.T, msg ipc.Message) {
	t.Helper()
	if msg.Error != nil {
		t.Fatalf("unexpected error: code=%d message=%s", msg.Error.Code, msg.Error.Message)
	}
}

func resultMap(t *testing.T, msg ipc.Message) map[string]any {
	t.Helper()
	mustOK(t, msg)
	var out map[string]any
	if err := json.Unmarshal(msg.Result, &out); err != nil {
		t.Fatalf("unmarshal result: %v; raw=%s", err, msg.Result)
	}
	return out
}

func timeCell(t *testing.T, row any) int {
	t.Helper()
	cells := row.([]any)
	cell := cells[0].(map[string]any)
	return int(cell["value"].(float64))
}
