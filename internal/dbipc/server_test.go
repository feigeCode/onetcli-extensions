package dbipc

import (
	"context"
	"database/sql"
	"database/sql/driver"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"sync/atomic"
	"testing"

	"onetcli-db-ipc-drivers/internal/ipc"
)

func TestServerRejectsBusinessMethodBeforeInit(t *testing.T) {
	server := NewServer(testSpec(), nil)
	resp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`1`),
		Method:  "conn/open",
		Params:  json.RawMessage(`{"config":{}}`),
	})

	if resp.Error == nil {
		t.Fatalf("expected protocol error before init, got result %s", resp.Result)
	}
	if resp.Error.Code != ErrNotInitialized {
		t.Fatalf("error code = %d, want %d", resp.Error.Code, ErrNotInitialized)
	}
}

func TestServerInitReturnsDriverCapabilities(t *testing.T) {
	server := NewServer(testSpec(), nil)
	resp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`1`),
		Method:  "init",
		Params:  json.RawMessage(`{"host_version":"test","api_offered":{"database":"1.0"},"instance_id":"unit","config":{}}`),
	})

	if resp.Error != nil {
		t.Fatalf("init returned error: %#v", resp.Error)
	}

	var result map[string]any
	if err := json.Unmarshal(resp.Result, &result); err != nil {
		t.Fatalf("result is not JSON object: %v", err)
	}
	if result["extension_version"] == "" {
		t.Fatalf("missing extension_version in %#v", result)
	}
	if !containsString(result["drivers_ready"], "testdb") {
		t.Fatalf("drivers_ready = %#v", result["drivers_ready"])
	}
	if !containsString(result["features"], "schema_introspection") {
		t.Fatalf("features = %#v", result["features"])
	}
	if !containsString(result["methods"], "schema/checks") {
		t.Fatalf("methods = %#v", result["methods"])
	}
	if !containsString(result["methods"], "schema/foreign_keys") {
		t.Fatalf("methods = %#v", result["methods"])
	}
	for _, method := range []string{
		"tx/begin",
		"tx/commit",
		"tx/rollback",
		"tx/savepoint",
		"tx/release",
		"ddl/build",
		"ddl/build_create_table",
		"ddl/build_alter_table",
		"ddl/build_drop",
		"data/export",
		"data/import_begin",
		"data/import_chunk",
		"data/import_commit",
		"data/import_abort",
		"stream/read",
		"stream/close",
	} {
		if !containsString(result["methods"], method) {
			t.Fatalf("methods missing %s: %#v", method, result["methods"])
		}
	}
}

func TestServerAllowsBusinessMethodsAfterInit(t *testing.T) {
	server := NewServer(testSpec(), nil)
	initResp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`1`),
		Method:  "init",
		Params:  json.RawMessage(`{}`),
	})
	if initResp.Error != nil {
		t.Fatalf("init returned error: %#v", initResp.Error)
	}

	resp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`2`),
		Method:  "conn/test",
		Params:  json.RawMessage(`{"driver_id":"testdb","config":{}}`),
	})

	if resp.Error == nil {
		t.Fatalf("expected conn/test error from unavailable sql driver, got result %s", resp.Result)
	}
	if resp.Error.Code == ErrNotInitialized {
		t.Fatalf("conn/test still failed as not initialized after init: %#v", resp.Error)
	}
}

func TestServerHandlesConnectionUseAsNoop(t *testing.T) {
	server := NewServer(testSpec(), nil)
	server.initialized = true
	server.conns[7] = &connectionState{}

	resp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`1`),
		Method:  "conn/use",
		Params:  json.RawMessage(`{"conn_id":7,"database":"main","schema":"public"}`),
	})

	if resp.Error != nil {
		t.Fatalf("conn/use returned error: %#v", resp.Error)
	}
	if string(resp.Result) != "null" {
		t.Fatalf("conn/use result = %s, want null", resp.Result)
	}
}

func TestQueryStartKeepsRowsStreamingUntilCursorFetch(t *testing.T) {
	driverName, state := registerStreamingDriver(t, [][]driver.Value{
		{int64(1), "first"},
		{int64(2), "second"},
	})
	server := NewServer(testSpecWithSQLDriver(driverName), nil)
	server.initialized = true

	connID := openTestConn(t, server)
	startResp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`2`),
		Method:  "query/start",
		Params:  []byte(fmt.Sprintf(`{"conn_id":%d,"sql":"SELECT id, name FROM demo"}`, connID)),
	})

	if startResp.Error != nil {
		t.Fatalf("query/start returned error: %#v", startResp.Error)
	}
	if got := atomic.LoadInt32(&state.nextCalls); got != 0 {
		t.Fatalf("query/start consumed %d row(s); rows must be fetched by cursor/fetch", got)
	}

	var started struct {
		CursorID         string       `json:"cursor_id"`
		Columns          []columnSpec `json:"columns"`
		RowCountKnown    bool         `json:"row_count_known"`
		RowCountEstimate *uint64      `json:"row_count_estimate,omitempty"`
	}
	decodeResult(t, startResp, &started)
	if started.CursorID == "" {
		t.Fatal("query/start did not return cursor_id")
	}
	if started.RowCountKnown {
		t.Fatal("streaming cursor must not report row_count_known before rows are fetched")
	}
	if started.RowCountEstimate != nil {
		t.Fatalf("streaming cursor returned row_count_estimate = %d, want nil", *started.RowCountEstimate)
	}
	if len(started.Columns) != 2 || started.Columns[0].Name != "id" || started.Columns[1].Name != "name" {
		t.Fatalf("columns = %#v", started.Columns)
	}

	fetchResp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`3`),
		Method:  "cursor/fetch",
		Params:  []byte(fmt.Sprintf(`{"cursor_id":%q,"n":1}`, started.CursorID)),
	})
	if fetchResp.Error != nil {
		t.Fatalf("cursor/fetch returned error: %#v", fetchResp.Error)
	}
	var fetched struct {
		Rows [][]cellValue `json:"rows"`
		Done bool          `json:"done"`
	}
	decodeResult(t, fetchResp, &fetched)
	if fetched.Done {
		t.Fatal("first cursor/fetch reported done with one row still available")
	}
	if len(fetched.Rows) != 1 || fetched.Rows[0][0]["value"] != float64(1) || fetched.Rows[0][1]["value"] != "first" {
		t.Fatalf("rows = %#v", fetched.Rows)
	}
	if got := atomic.LoadInt32(&state.nextCalls); got != 1 {
		t.Fatalf("cursor/fetch consumed %d row(s), want 1", got)
	}
}

func TestCursorFetchReturnsEmptyArrayWhenNoRows(t *testing.T) {
	driverName, _ := registerStreamingDriver(t, nil)
	server := NewServer(testSpecWithSQLDriver(driverName), nil)
	server.initialized = true

	connID := openTestConn(t, server)
	startResp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`2`),
		Method:  "query/start",
		Params:  []byte(fmt.Sprintf(`{"conn_id":%d,"sql":"SELECT id, name FROM empty_demo"}`, connID)),
	})
	if startResp.Error != nil {
		t.Fatalf("query/start returned error: %#v", startResp.Error)
	}
	var started struct {
		CursorID string `json:"cursor_id"`
	}
	decodeResult(t, startResp, &started)

	fetchResp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`3`),
		Method:  "cursor/fetch",
		Params:  []byte(fmt.Sprintf(`{"cursor_id":%q,"n":100}`, started.CursorID)),
	})
	if fetchResp.Error != nil {
		t.Fatalf("cursor/fetch returned error: %#v", fetchResp.Error)
	}
	var raw map[string]json.RawMessage
	decodeResult(t, fetchResp, &raw)
	if string(raw["rows"]) != "[]" {
		t.Fatalf("cursor/fetch rows raw JSON = %s, want []", raw["rows"])
	}
	var fetched struct {
		Rows [][]cellValue `json:"rows"`
		Done bool          `json:"done"`
	}
	decodeResult(t, fetchResp, &fetched)
	if len(fetched.Rows) != 0 || !fetched.Done {
		t.Fatalf("fetch result = %#v, want empty rows and done", fetched)
	}
}

func TestConnCloseClosesCursorsForConnection(t *testing.T) {
	driverName, state := registerStreamingDriver(t, [][]driver.Value{{int64(1)}})
	server := NewServer(testSpecWithSQLDriver(driverName), nil)
	server.initialized = true

	connID := openTestConn(t, server)
	startResp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`2`),
		Method:  "query/start",
		Params:  []byte(fmt.Sprintf(`{"conn_id":%d,"sql":"SELECT id FROM demo"}`, connID)),
	})
	if startResp.Error != nil {
		t.Fatalf("query/start returned error: %#v", startResp.Error)
	}
	var started struct {
		CursorID string `json:"cursor_id"`
	}
	decodeResult(t, startResp, &started)

	closeResp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`3`),
		Method:  "conn/close",
		Params:  []byte(fmt.Sprintf(`{"conn_id":%d}`, connID)),
	})
	if closeResp.Error != nil {
		t.Fatalf("conn/close returned error: %#v", closeResp.Error)
	}
	if got := atomic.LoadInt32(&state.closeCalls); got != 1 {
		t.Fatalf("Rows.Close calls = %d, want 1", got)
	}
	if _, ok := server.cursors[started.CursorID]; ok {
		t.Fatalf("conn/close left cursor %q in server state", started.CursorID)
	}
}

func TestServerHandlesCursorCancelWithoutDroppingCursorMetadata(t *testing.T) {
	driverName, state := registerStreamingDriver(t, [][]driver.Value{{int64(1)}})
	server := NewServer(testSpecWithSQLDriver(driverName), nil)
	server.initialized = true

	connID := openTestConn(t, server)
	startResp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`2`),
		Method:  "query/start",
		Params:  []byte(fmt.Sprintf(`{"conn_id":%d,"sql":"SELECT id FROM demo"}`, connID)),
	})
	if startResp.Error != nil {
		t.Fatalf("query/start returned error: %#v", startResp.Error)
	}
	var started struct {
		CursorID string `json:"cursor_id"`
	}
	decodeResult(t, startResp, &started)

	resp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`3`),
		Method:  "cursor/cancel",
		Params:  []byte(fmt.Sprintf(`{"cursor_id":%q}`, started.CursorID)),
	})

	if resp.Error != nil {
		t.Fatalf("cursor/cancel returned error: %#v", resp.Error)
	}
	if got := atomic.LoadInt32(&state.closeCalls); got != 1 {
		t.Fatalf("Rows.Close calls after cancel = %d, want 1", got)
	}

	fetchResp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`4`),
		Method:  "cursor/fetch",
		Params:  []byte(fmt.Sprintf(`{"cursor_id":%q,"n":1}`, started.CursorID)),
	})
	if fetchResp.Error != nil {
		t.Fatalf("cursor/fetch after cancel returned error: %#v", fetchResp.Error)
	}
	var fetched struct {
		Rows [][]cellValue `json:"rows"`
		Done bool          `json:"done"`
	}
	decodeResult(t, fetchResp, &fetched)
	if len(fetched.Rows) != 0 || !fetched.Done {
		t.Fatalf("fetch after cancel = %#v, want no rows and done", fetched)
	}

	closeResp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`5`),
		Method:  "cursor/close",
		Params:  []byte(fmt.Sprintf(`{"cursor_id":%q}`, started.CursorID)),
	})
	if closeResp.Error != nil {
		t.Fatalf("cursor/close after cancel returned error: %#v", closeResp.Error)
	}
}

func TestQueryAndExecForwardWireParamsToSQLDriver(t *testing.T) {
	driverName, state := registerStreamingDriver(t, [][]driver.Value{{int64(1)}})
	server := NewServer(testSpecWithSQLDriver(driverName), nil)
	server.initialized = true

	connID := openTestConn(t, server)
	queryResp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`2`),
		Method:  "query/start",
		Params: []byte(fmt.Sprintf(`{
			"conn_id": %d,
			"sql": "SELECT * FROM demo WHERE id = ? AND name = ? AND deleted_at IS ?",
			"params": [
				{"type":"i64","value":42},
				{"type":"text","value":"alice"},
				{"type":"null"}
			]
		}`, connID)),
	})
	if queryResp.Error != nil {
		t.Fatalf("query/start returned error: %#v", queryResp.Error)
	}
	if got := namedValuesToValues(state.lastQueryArgs); len(got) != 3 || got[0] != int64(42) || got[1] != "alice" || got[2] != nil {
		t.Fatalf("query args = %#v", got)
	}

	execResp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`3`),
		Method:  "exec/run",
		Params: []byte(fmt.Sprintf(`{
			"conn_id": %d,
			"sql": "UPDATE demo SET active = ?",
			"params": [{"type":"bool","value":true}]
		}`, connID)),
	})
	if execResp.Error != nil {
		t.Fatalf("exec/run returned error: %#v", execResp.Error)
	}
	if got := namedValuesToValues(state.lastExecArgs); len(got) != 1 || got[0] != true {
		t.Fatalf("exec args = %#v", got)
	}
}

func TestExecBatchRunsStatementsAndStopsOnError(t *testing.T) {
	driverName, state := registerStreamingDriver(t, nil)
	state.execErrAt = 2
	server := NewServer(testSpecWithSQLDriver(driverName), nil)
	server.initialized = true

	connID := openTestConn(t, server)
	resp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`2`),
		Method:  "exec/batch",
		Params: []byte(fmt.Sprintf(`{
			"conn_id": %d,
			"statements": ["UPDATE demo SET n = 1", "BAD SQL", "UPDATE demo SET n = 3"],
			"stop_on_error": true
		}`, connID)),
	})
	if resp.Error != nil {
		t.Fatalf("exec/batch returned error: %#v", resp.Error)
	}
	var result struct {
		Results []map[string]any `json:"results"`
		Errors  []map[string]any `json:"errors"`
	}
	decodeResult(t, resp, &result)
	if len(result.Results) != 1 {
		t.Fatalf("results = %#v, want one successful statement before error", result.Results)
	}
	if len(result.Errors) != 1 || result.Errors[0]["message"] != "batch exec failure" {
		t.Fatalf("errors = %#v, want recorded batch exec failure", result.Errors)
	}
	if got := atomic.LoadInt32(&state.execCalls); got != 2 {
		t.Fatalf("exec calls = %d, want stop after second statement", got)
	}
}

func TestTransactionMethodsRouteQueryExecAndLifecycle(t *testing.T) {
	driverName, state := registerStreamingDriver(t, [][]driver.Value{{int64(1)}})
	server := NewServer(testSpecWithSQLDriver(driverName), nil)
	server.initialized = true

	connID := openTestConn(t, server)
	beginResp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`2`),
		Method:  "tx/begin",
		Params:  []byte(fmt.Sprintf(`{"conn_id":%d,"isolation":"serializable","read_only":true}`, connID)),
	})
	if beginResp.Error != nil {
		t.Fatalf("tx/begin returned error: %#v", beginResp.Error)
	}
	var begun struct {
		TxID string `json:"tx_id"`
	}
	decodeResult(t, beginResp, &begun)
	if begun.TxID == "" {
		t.Fatalf("tx/begin result = %#v", begun)
	}

	queryResp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`3`),
		Method:  "query/start",
		Params:  []byte(fmt.Sprintf(`{"conn_id":%d,"tx_id":%q,"sql":"SELECT id FROM demo"}`, connID, begun.TxID)),
	})
	if queryResp.Error != nil {
		t.Fatalf("query/start in tx returned error: %#v", queryResp.Error)
	}
	if atomic.LoadInt32(&state.txQueryCalls) != 1 {
		t.Fatalf("tx query calls = %d, want 1", atomic.LoadInt32(&state.txQueryCalls))
	}

	execResp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`4`),
		Method:  "exec/run",
		Params:  []byte(fmt.Sprintf(`{"conn_id":%d,"tx_id":%q,"sql":"UPDATE demo SET id = id"}`, connID, begun.TxID)),
	})
	if execResp.Error != nil {
		t.Fatalf("exec/run in tx returned error: %#v", execResp.Error)
	}
	if atomic.LoadInt32(&state.txExecCalls) != 1 {
		t.Fatalf("tx exec calls = %d, want 1", atomic.LoadInt32(&state.txExecCalls))
	}

	for _, call := range []struct {
		method string
		params string
	}{
		{"tx/savepoint", fmt.Sprintf(`{"tx_id":%q,"name":"sp1"}`, begun.TxID)},
		{"tx/release", fmt.Sprintf(`{"tx_id":%q,"name":"sp1"}`, begun.TxID)},
		{"tx/commit", fmt.Sprintf(`{"tx_id":%q}`, begun.TxID)},
	} {
		resp := server.Handle(context.Background(), ipc.Message{
			JSONRPC: "2.0",
			ID:      json.RawMessage(`5`),
			Method:  call.method,
			Params:  []byte(call.params),
		})
		if resp.Error != nil {
			t.Fatalf("%s returned error: %#v", call.method, resp.Error)
		}
	}
	if atomic.LoadInt32(&state.commitCalls) != 1 {
		t.Fatalf("commit calls = %d, want 1", atomic.LoadInt32(&state.commitCalls))
	}
}

func TestDdlBuildersProduceDialectSQL(t *testing.T) {
	server := NewServer(testSpec(), nil)
	server.initialized = true

	createResp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`1`),
		Method:  "ddl/build_create_table",
		Params: json.RawMessage(`{
			"spec": {
				"schema": "app",
				"name": "demo",
				"columns": [
					{"name":"id","type":"BIGINT","nullable":false,"is_primary":true},
					{"name":"name","type":"VARCHAR(64)","nullable":true,"default":"'anon'"}
				],
				"indexes": [{"name":"idx_demo_name","columns":["name"],"is_unique":true}]
			},
			"options": {"if_not_exists": true}
		}`),
	})
	if createResp.Error != nil {
		t.Fatalf("ddl/build_create_table returned error: %#v", createResp.Error)
	}
	var createResult struct {
		SQL        string   `json:"sql"`
		Statements []string `json:"statements"`
	}
	decodeResult(t, createResp, &createResult)
	if createResult.SQL != `CREATE TABLE IF NOT EXISTS "app"."demo" ("id" BIGINT NOT NULL, "name" VARCHAR(64) DEFAULT 'anon', PRIMARY KEY ("id"))` {
		t.Fatalf("create SQL = %q", createResult.SQL)
	}
	if len(createResult.Statements) != 2 || createResult.Statements[1] != `CREATE UNIQUE INDEX "idx_demo_name" ON "app"."demo" ("name")` {
		t.Fatalf("create statements = %#v", createResult.Statements)
	}

	dropResp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`2`),
		Method:  "ddl/build_drop",
		Params:  json.RawMessage(`{"kind":"table","schema":"app","name":"demo","if_exists":true,"cascade":true}`),
	})
	if dropResp.Error != nil {
		t.Fatalf("ddl/build_drop returned error: %#v", dropResp.Error)
	}
	var dropResult struct {
		SQL string `json:"sql"`
	}
	decodeResult(t, dropResp, &dropResult)
	if dropResult.SQL != `DROP TABLE IF EXISTS "app"."demo" CASCADE` {
		t.Fatalf("drop SQL = %q", dropResult.SQL)
	}
}

func TestDataExportStreamsNdjsonAndClosesStream(t *testing.T) {
	driverName, _ := registerStreamingDriver(t, [][]driver.Value{
		{int64(1), "first"},
		{int64(2), "second"},
	})
	server := NewServer(testSpecWithSQLDriver(driverName), nil)
	server.initialized = true

	connID := openTestConn(t, server)
	exportResp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`2`),
		Method:  "data/export",
		Params:  []byte(fmt.Sprintf(`{"conn_id":%d,"sql":"SELECT id, name FROM demo ORDER BY id","format":"ndjson","stream_id":"s1"}`, connID)),
	})
	if exportResp.Error != nil {
		t.Fatalf("data/export returned error: %#v", exportResp.Error)
	}

	readResp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`3`),
		Method:  "stream/read",
		Params:  json.RawMessage(`{"stream_id":"s1","max_bytes":4096}`),
	})
	if readResp.Error != nil {
		t.Fatalf("stream/read returned error: %#v", readResp.Error)
	}
	var readResult struct {
		Data string `json:"data"`
		Done bool   `json:"done"`
	}
	decodeResult(t, readResp, &readResult)
	raw, err := base64.StdEncoding.DecodeString(readResult.Data)
	if err != nil {
		t.Fatalf("stream/read data is not base64: %v", err)
	}
	if string(raw) != "{\"id\":1,\"name\":\"first\"}\n{\"id\":2,\"name\":\"second\"}\n" || !readResult.Done {
		t.Fatalf("stream chunk = %q done=%v", raw, readResult.Done)
	}

	closeResp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`4`),
		Method:  "stream/close",
		Params:  json.RawMessage(`{"stream_id":"s1"}`),
	})
	if closeResp.Error != nil {
		t.Fatalf("stream/close returned error: %#v", closeResp.Error)
	}
}

func TestDataImportBuildsInsertAndCommits(t *testing.T) {
	driverName, state := registerStreamingDriver(t, nil)
	server := NewServer(testSpecWithSQLDriver(driverName), nil)
	server.initialized = true

	connID := openTestConn(t, server)
	beginResp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`2`),
		Method:  "data/import_begin",
		Params:  []byte(fmt.Sprintf(`{"conn_id":%d,"schema":"app","table":"demo","format":"json","columns":["id","name"]}`, connID)),
	})
	if beginResp.Error != nil {
		t.Fatalf("data/import_begin returned error: %#v", beginResp.Error)
	}
	var begun struct {
		ImportID string `json:"import_id"`
	}
	decodeResult(t, beginResp, &begun)

	chunkResp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`3`),
		Method:  "data/import_chunk",
		Params:  []byte(fmt.Sprintf(`{"import_id":%q,"rows":[[{"type":"i64","value":1},{"type":"text","value":"first"}]]}`, begun.ImportID)),
	})
	if chunkResp.Error != nil {
		t.Fatalf("data/import_chunk returned error: %#v", chunkResp.Error)
	}
	var chunkResult struct {
		Inserted uint64 `json:"inserted"`
	}
	decodeResult(t, chunkResp, &chunkResult)
	if chunkResult.Inserted != 1 {
		t.Fatalf("chunk inserted = %d, want 1", chunkResult.Inserted)
	}
	if state.lastExecSQL != `INSERT INTO "app"."demo" ("id", "name") VALUES (?, ?)` {
		t.Fatalf("last exec SQL = %q", state.lastExecSQL)
	}

	commitResp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`4`),
		Method:  "data/import_commit",
		Params:  []byte(fmt.Sprintf(`{"import_id":%q}`, begun.ImportID)),
	})
	if commitResp.Error != nil {
		t.Fatalf("data/import_commit returned error: %#v", commitResp.Error)
	}
	var commitResult struct {
		Inserted uint64 `json:"inserted"`
	}
	decodeResult(t, commitResp, &commitResult)
	if commitResult.Inserted != 1 {
		t.Fatalf("commit inserted = %d, want 1", commitResult.Inserted)
	}
}

func TestSchemaObjectsUsesDriverSQLAndReturnsKind(t *testing.T) {
	driverName, state := registerStreamingDriver(t, [][]driver.Value{
		{"demo", "table", "demo table"},
	})
	state.columns = []string{"object_name", "kind", "comment"}
	spec := testSpecWithSQLDriver(driverName)
	spec.SchemaSQL.Objects = func(cfg Config, database, schema string, kinds []string) string {
		if database != "main" || schema != "app" || len(kinds) != 1 || kinds[0] != "table" {
			t.Fatalf("objects params = database:%q schema:%q kinds:%#v", database, schema, kinds)
		}
		return "SELECT object_name, kind, comment FROM test_objects"
	}
	server := NewServer(spec, nil)
	server.initialized = true

	connID := openTestConn(t, server)
	resp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`2`),
		Method:  "schema/objects",
		Params:  []byte(fmt.Sprintf(`{"conn_id":%d,"database":"main","schema":"app","kinds":["table"]}`, connID)),
	})
	if resp.Error != nil {
		t.Fatalf("schema/objects returned error: %#v", resp.Error)
	}

	var result []map[string]any
	decodeResult(t, resp, &result)
	if len(result) != 1 {
		t.Fatalf("objects result = %#v, want one object", result)
	}
	object := result[0]
	if object["name"] != "demo" || object["kind"] != "table" || object["comment"] != "demo table" {
		t.Fatalf("object metadata = %#v", object)
	}
	if _, ok := object["kind"]; !ok {
		t.Fatalf("object metadata missing kind: raw=%s", resp.Result)
	}
}

func TestSchemaObjectViewColumnsUsesDriverSQL(t *testing.T) {
	driverName, state := registerStreamingDriver(t, [][]driver.Value{
		{int64(1), "id", "BIGINT", "NO", nil},
		{int64(2), "name", "VARCHAR", "YES", "untitled"},
	})
	state.columns = []string{"ordinal", "column_name", "data_type", "is_nullable", "column_default"}
	spec := testSpecWithSQLDriver(driverName)
	spec.SchemaSQL.Columns = func(cfg Config, database, schema, table string) string {
		if database != "main" || schema != "app" || table != "demo" {
			t.Fatalf("columns params = database:%q schema:%q table:%q", database, schema, table)
		}
		return "SELECT ordinal, column_name, data_type, is_nullable, column_default FROM test_columns"
	}
	server := NewServer(spec, nil)
	server.initialized = true

	connID := openTestConn(t, server)
	resp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`2`),
		Method:  "schema/object_view",
		Params:  []byte(fmt.Sprintf(`{"conn_id":%d,"view":"columns","database":"main","schema":"app","table":"demo"}`, connID)),
	})
	if resp.Error != nil {
		t.Fatalf("schema/object_view returned error: %#v", resp.Error)
	}

	var result struct {
		Title   string `json:"title"`
		Columns []struct {
			Key     string   `json:"key"`
			Name    string   `json:"name"`
			WidthPx *float64 `json:"width_px"`
			Align   string   `json:"align,omitempty"`
		} `json:"columns"`
		Rows [][]string `json:"rows"`
	}
	decodeResult(t, resp, &result)
	if result.Title != "Columns" {
		t.Fatalf("title = %q", result.Title)
	}
	if len(result.Columns) != 5 || result.Columns[0].Key != "name" || result.Columns[0].Name != "Field" {
		t.Fatalf("columns = %#v", result.Columns)
	}
	if len(result.Rows) != 2 || result.Rows[0][0] != "id" || result.Rows[0][1] != "BIGINT" || result.Rows[0][2] != "false" {
		t.Fatalf("rows = %#v", result.Rows)
	}
}

func TestSchemaIndexesUsesDriverSQL(t *testing.T) {
	driverName, state := registerStreamingDriver(t, [][]driver.Value{
		{"idx_demo_name", "id,name", "YES", "NO", "btree"},
	})
	state.columns = []string{"index_name", "columns", "is_unique", "is_primary", "index_type"}
	spec := testSpecWithSQLDriver(driverName)
	spec.SchemaSQL.Indexes = func(cfg Config, database, schema, table string) string {
		if database != "main" || schema != "app" || table != "demo" {
			t.Fatalf("indexes params = database:%q schema:%q table:%q", database, schema, table)
		}
		return "SELECT index_name, columns, is_unique, is_primary, index_type FROM test_indexes"
	}
	server := NewServer(spec, nil)
	server.initialized = true

	connID := openTestConn(t, server)
	resp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`2`),
		Method:  "schema/indexes",
		Params:  []byte(fmt.Sprintf(`{"conn_id":%d,"database":"main","schema":"app","table":"demo"}`, connID)),
	})
	if resp.Error != nil {
		t.Fatalf("schema/indexes returned error: %#v", resp.Error)
	}

	var result []map[string]any
	decodeResult(t, resp, &result)
	if len(result) != 1 {
		t.Fatalf("indexes result = %#v, want one index", result)
	}
	index := result[0]
	if index["name"] != "idx_demo_name" || index["type"] != "btree" {
		t.Fatalf("index metadata = %#v", index)
	}
	if index["is_unique"] != true || index["is_primary"] != false {
		t.Fatalf("index flags = %#v", index)
	}
	columns, ok := index["columns"].([]any)
	if !ok || len(columns) != 2 || columns[0] != "id" || columns[1] != "name" {
		t.Fatalf("index columns = %#v", index["columns"])
	}
}

func TestSchemaForeignKeysUsesDriverSQL(t *testing.T) {
	driverName, state := registerStreamingDriver(t, [][]driver.Value{
		{"fk_demo_account", "account_id,tenant_id", "app", "accounts", "id,tenant_id", "CASCADE", "RESTRICT"},
	})
	state.columns = []string{"constraint_name", "columns", "referenced_schema", "referenced_table", "referenced_columns", "on_update", "on_delete"}
	spec := testSpecWithSQLDriver(driverName)
	spec.SchemaSQL.ForeignKeys = func(cfg Config, database, schema, table string) string {
		if database != "main" || schema != "app" || table != "demo" {
			t.Fatalf("foreign keys params = database:%q schema:%q table:%q", database, schema, table)
		}
		return "SELECT constraint_name, columns, referenced_schema, referenced_table, referenced_columns, on_update, on_delete FROM test_foreign_keys"
	}
	server := NewServer(spec, nil)
	server.initialized = true

	connID := openTestConn(t, server)
	resp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`2`),
		Method:  "schema/foreign_keys",
		Params:  []byte(fmt.Sprintf(`{"conn_id":%d,"database":"main","schema":"app","table":"demo"}`, connID)),
	})
	if resp.Error != nil {
		t.Fatalf("schema/foreign_keys returned error: %#v", resp.Error)
	}

	var result []map[string]any
	decodeResult(t, resp, &result)
	if len(result) != 1 {
		t.Fatalf("foreign keys result = %#v, want one foreign key", result)
	}
	fk := result[0]
	if fk["name"] != "fk_demo_account" || fk["referenced_schema"] != "app" || fk["referenced_table"] != "accounts" {
		t.Fatalf("foreign key metadata = %#v", fk)
	}
	if fk["on_update"] != "CASCADE" || fk["on_delete"] != "RESTRICT" {
		t.Fatalf("foreign key actions = %#v", fk)
	}
	columns, ok := fk["columns"].([]any)
	if !ok || len(columns) != 2 || columns[0] != "account_id" || columns[1] != "tenant_id" {
		t.Fatalf("foreign key columns = %#v", fk["columns"])
	}
	referencedColumns, ok := fk["referenced_columns"].([]any)
	if !ok || len(referencedColumns) != 2 || referencedColumns[0] != "id" || referencedColumns[1] != "tenant_id" {
		t.Fatalf("foreign key referenced columns = %#v", fk["referenced_columns"])
	}
}

func TestSchemaViewsUsesDriverSQL(t *testing.T) {
	driverName, state := registerStreamingDriver(t, [][]driver.Value{
		{"v_demo", "app", "demo view", "NO", "CREATE VIEW app.v_demo AS SELECT 1"},
		{"mv_demo", "app", "materialized demo view", "YES", "CREATE MATERIALIZED VIEW app.mv_demo AS SELECT 1"},
	})
	state.columns = []string{"view_name", "schema_name", "comment", "is_materialized", "definition_sql"}
	spec := testSpecWithSQLDriver(driverName)
	spec.SchemaSQL.Views = func(cfg Config, database, schema string) string {
		if database != "main" || schema != "app" {
			t.Fatalf("views params = database:%q schema:%q", database, schema)
		}
		return "SELECT view_name, schema_name, comment, is_materialized, definition_sql FROM test_views"
	}
	server := NewServer(spec, nil)
	server.initialized = true

	connID := openTestConn(t, server)
	resp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`2`),
		Method:  "schema/views",
		Params:  []byte(fmt.Sprintf(`{"conn_id":%d,"database":"main","schema":"app"}`, connID)),
	})
	if resp.Error != nil {
		t.Fatalf("schema/views returned error: %#v", resp.Error)
	}

	var result []map[string]any
	decodeResult(t, resp, &result)
	if len(result) != 2 {
		t.Fatalf("views result = %#v, want two views", result)
	}
	if result[0]["name"] != "v_demo" || result[0]["schema"] != "app" || result[0]["comment"] != "demo view" {
		t.Fatalf("first view metadata = %#v", result[0])
	}
	if result[0]["kind"] != "view" || result[1]["kind"] != "materialized_view" {
		t.Fatalf("view kinds = %#v", result)
	}
	if result[0]["definition_sql"] != "CREATE VIEW app.v_demo AS SELECT 1" || result[1]["definition_sql"] != "CREATE MATERIALIZED VIEW app.mv_demo AS SELECT 1" {
		t.Fatalf("view definitions = %#v", result)
	}
	if result[0]["is_materialized"] != false || result[1]["is_materialized"] != true {
		t.Fatalf("materialized flags = %#v", result)
	}
}

func TestSchemaFunctionsUsesDriverSQL(t *testing.T) {
	driverName, state := registerStreamingDriver(t, [][]driver.Value{
		{"calc_total", "app", "NUMBER", "SQL", "calculates totals"},
	})
	state.columns = []string{"function_name", "schema_name", "returns", "language", "comment"}
	spec := testSpecWithSQLDriver(driverName)
	spec.SchemaSQL.Functions = func(cfg Config, database, schema string) string {
		if database != "main" || schema != "app" {
			t.Fatalf("functions params = database:%q schema:%q", database, schema)
		}
		return "SELECT function_name, schema_name, returns, language, comment FROM test_functions"
	}
	server := NewServer(spec, nil)
	server.initialized = true

	connID := openTestConn(t, server)
	resp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`2`),
		Method:  "schema/functions",
		Params:  []byte(fmt.Sprintf(`{"conn_id":%d,"database":"main","schema":"app"}`, connID)),
	})
	if resp.Error != nil {
		t.Fatalf("schema/functions returned error: %#v", resp.Error)
	}

	var result []map[string]any
	decodeResult(t, resp, &result)
	if len(result) != 1 {
		t.Fatalf("functions result = %#v, want one function", result)
	}
	fn := result[0]
	if fn["name"] != "calc_total" || fn["schema"] != "app" || fn["returns"] != "NUMBER" || fn["language"] != "SQL" || fn["comment"] != "calculates totals" {
		t.Fatalf("function metadata = %#v", fn)
	}
}

func TestSchemaViewDefinitionUsesDriverSQL(t *testing.T) {
	driverName, state := registerStreamingDriver(t, [][]driver.Value{
		{"CREATE VIEW app.v_demo AS ", "NO"},
		{"SELECT id FROM app.demo", "NO"},
	})
	state.columns = []string{"definition", "is_materialized"}
	spec := testSpecWithSQLDriver(driverName)
	spec.SchemaSQL.ViewDefinition = func(cfg Config, database, schema, view string) string {
		if database != "main" || schema != "app" || view != "v_demo" {
			t.Fatalf("view definition params = database:%q schema:%q view:%q", database, schema, view)
		}
		return "SELECT definition, is_materialized FROM test_views"
	}
	server := NewServer(spec, nil)
	server.initialized = true

	connID := openTestConn(t, server)
	resp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`2`),
		Method:  "schema/view_definition",
		Params:  []byte(fmt.Sprintf(`{"conn_id":%d,"database":"main","schema":"app","view":"v_demo"}`, connID)),
	})
	if resp.Error != nil {
		t.Fatalf("schema/view_definition returned error: %#v", resp.Error)
	}

	var result map[string]any
	decodeResult(t, resp, &result)
	if result["sql"] != "CREATE VIEW app.v_demo AS SELECT id FROM app.demo" {
		t.Fatalf("view definition = %#v", result)
	}
	if result["is_materialized"] != false {
		t.Fatalf("materialized flag = %#v", result)
	}
}

func TestOptionalSchemaMethodsReturnEmptyResults(t *testing.T) {
	server := NewServer(testSpec(), nil)
	server.initialized = true
	server.conns[7] = &connectionState{}

	for _, method := range []string{
		"schema/procedures",
		"schema/triggers",
		"schema/sequences",
		"schema/types",
	} {
		resp := server.Handle(context.Background(), ipc.Message{
			JSONRPC: "2.0",
			ID:      json.RawMessage(`1`),
			Method:  method,
			Params:  json.RawMessage(`{"conn_id":7}`),
		})
		if resp.Error != nil {
			t.Fatalf("%s returned error: %#v", method, resp.Error)
		}
		if string(resp.Result) != "[]" {
			t.Fatalf("%s raw result = %s, want []", method, resp.Result)
		}
		var result []map[string]any
		decodeResult(t, resp, &result)
		if len(result) != 0 {
			t.Fatalf("%s result = %#v, want empty list", method, result)
		}
	}
}

func TestQueriedSchemaMethodsReturnEmptyArrayWhenNoRows(t *testing.T) {
	driverName, state := registerStreamingDriver(t, nil)
	state.columns = []string{"object_name", "kind", "comment"}
	spec := testSpecWithSQLDriver(driverName)
	spec.SchemaSQL.Objects = func(cfg Config, database, schema string, kinds []string) string {
		return "SELECT object_name, kind, comment FROM empty_objects"
	}
	server := NewServer(spec, nil)
	server.initialized = true

	connID := openTestConn(t, server)
	resp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`2`),
		Method:  "schema/objects",
		Params:  []byte(fmt.Sprintf(`{"conn_id":%d,"schema":"empty"}`, connID)),
	})
	if resp.Error != nil {
		t.Fatalf("schema/objects returned error: %#v", resp.Error)
	}
	if string(resp.Result) != "[]" {
		t.Fatalf("schema/objects raw result = %s, want []", resp.Result)
	}
}

func testSpec() DriverSpec {
	return testSpecWithSQLDriver("testdb")
}

func testSpecWithSQLDriver(driverName string) DriverSpec {
	return DriverSpec{
		ID:            "testdb",
		Name:          "TestDB",
		SQLDriverName: driverName,
		DefaultPort:   1234,
		BuildDSN: func(Config) (string, error) {
			return "test", nil
		},
	}
}

func containsString(value any, want string) bool {
	items, ok := value.([]any)
	if !ok {
		return false
	}
	for _, item := range items {
		if item == want {
			return true
		}
	}
	return false
}

func openTestConn(t *testing.T, server *Server) uint64 {
	t.Helper()
	resp := server.Handle(context.Background(), ipc.Message{
		JSONRPC: "2.0",
		ID:      json.RawMessage(`1`),
		Method:  "conn/open",
		Params:  json.RawMessage(`{"driver_id":"testdb","config":{}}`),
	})
	if resp.Error != nil {
		t.Fatalf("conn/open returned error: %#v", resp.Error)
	}
	var opened struct {
		ConnID uint64 `json:"conn_id"`
	}
	decodeResult(t, resp, &opened)
	return opened.ConnID
}

func decodeResult(t *testing.T, resp ipc.Message, out any) {
	t.Helper()
	if err := json.Unmarshal(resp.Result, out); err != nil {
		t.Fatalf("result is not valid JSON: %v; raw=%s", err, resp.Result)
	}
}

var streamingDriverSeq uint64

type streamingDriverState struct {
	rows          [][]driver.Value
	columns       []string
	nextCalls     int32
	closeCalls    int32
	execCalls     int32
	execErrAt     int32
	txQueryCalls  int32
	txExecCalls   int32
	commitCalls   int32
	rollbackCalls int32
	lastExecSQL   string
	lastQueryArgs []driver.NamedValue
	lastExecArgs  []driver.NamedValue
}

func registerStreamingDriver(t *testing.T, rows [][]driver.Value) (string, *streamingDriverState) {
	t.Helper()
	name := fmt.Sprintf("dbipc_streaming_%d", atomic.AddUint64(&streamingDriverSeq, 1))
	state := &streamingDriverState{rows: rows}
	sql.Register(name, &streamingDriver{state: state})
	return name, state
}

type streamingDriver struct {
	state *streamingDriverState
}

func (d *streamingDriver) Open(string) (driver.Conn, error) {
	return &streamingConn{state: d.state}, nil
}

type streamingConn struct {
	state *streamingDriverState
	inTx  int32
}

func (c *streamingConn) Prepare(string) (driver.Stmt, error) {
	return nil, driver.ErrSkip
}

func (c *streamingConn) Close() error {
	return nil
}

func (c *streamingConn) Begin() (driver.Tx, error) {
	atomic.StoreInt32(&c.inTx, 1)
	return &streamingTx{conn: c}, nil
}

func (c *streamingConn) BeginTx(context.Context, driver.TxOptions) (driver.Tx, error) {
	return c.Begin()
}

func (c *streamingConn) Ping(context.Context) error {
	return nil
}

func (c *streamingConn) QueryContext(_ context.Context, _ string, args []driver.NamedValue) (driver.Rows, error) {
	if atomic.LoadInt32(&c.inTx) == 1 {
		atomic.AddInt32(&c.state.txQueryCalls, 1)
	}
	c.state.lastQueryArgs = cloneNamedValues(args)
	return &streamingRows{state: c.state, rows: c.state.rows}, nil
}

func (c *streamingConn) ExecContext(_ context.Context, query string, args []driver.NamedValue) (driver.Result, error) {
	if atomic.LoadInt32(&c.inTx) == 1 {
		atomic.AddInt32(&c.state.txExecCalls, 1)
	}
	call := atomic.AddInt32(&c.state.execCalls, 1)
	if c.state.execErrAt > 0 && call == c.state.execErrAt {
		return nil, errors.New("batch exec failure")
	}
	c.state.lastExecSQL = query
	c.state.lastExecArgs = cloneNamedValues(args)
	return driver.RowsAffected(1), nil
}

type streamingTx struct {
	conn *streamingConn
}

func (tx *streamingTx) Commit() error {
	atomic.StoreInt32(&tx.conn.inTx, 0)
	atomic.AddInt32(&tx.conn.state.commitCalls, 1)
	return nil
}

func (tx *streamingTx) Rollback() error {
	atomic.StoreInt32(&tx.conn.inTx, 0)
	atomic.AddInt32(&tx.conn.state.rollbackCalls, 1)
	return nil
}

type streamingRows struct {
	state *streamingDriverState
	rows  [][]driver.Value
	index int
}

func (r *streamingRows) Columns() []string {
	if len(r.state.columns) > 0 {
		return r.state.columns
	}
	return []string{"id", "name"}
}

func (r *streamingRows) Close() error {
	atomic.AddInt32(&r.state.closeCalls, 1)
	return nil
}

func (r *streamingRows) Next(dest []driver.Value) error {
	if r.index >= len(r.rows) {
		return io.EOF
	}
	atomic.AddInt32(&r.state.nextCalls, 1)
	copy(dest, r.rows[r.index])
	r.index++
	return nil
}

func (r *streamingRows) ColumnTypeDatabaseTypeName(index int) string {
	if index == 0 {
		return "BIGINT"
	}
	return "TEXT"
}

func (r *streamingRows) ColumnTypeNullable(int) (bool, bool) {
	return true, true
}

func cloneNamedValues(values []driver.NamedValue) []driver.NamedValue {
	out := make([]driver.NamedValue, len(values))
	copy(out, values)
	return out
}

func namedValuesToValues(values []driver.NamedValue) []any {
	out := make([]any, 0, len(values))
	for _, value := range values {
		out = append(out, value.Value)
	}
	return out
}
