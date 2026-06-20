package dbipc

import (
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"strings"
	"sync"
	"time"

	"onetcli-db-ipc-drivers/internal/ipc"
)

type Opener func(driverName, dsn string) (*sql.DB, error)

type Server struct {
	spec        DriverSpec
	opener      Opener
	initialized bool
	nextConnID  uint64
	nextCursor  uint64
	nextTx      uint64
	nextImport  uint64
	conns       map[uint64]*connectionState
	cursors     map[string]*cursorState
	txs         map[string]*txState
	imports     map[string]*importState
	streams     map[string]*streamState
	mu          sync.Mutex
}

type connectionState struct {
	config    Config
	db        *sql.DB
	schemaSQL SchemaSQL
}

type cursorState struct {
	connID      uint64
	rows        *sql.Rows
	columnCount int
	maxRows     *uint64
	fetched     uint64
	done        bool
}

type txState struct {
	connID uint64
	tx     *sql.Tx
}

func NewServer(spec DriverSpec, opener Opener) *Server {
	if opener == nil {
		opener = sql.Open
	}
	return &Server{
		spec:       spec,
		opener:     opener,
		nextConnID: 1,
		nextCursor: 1,
		nextTx:     1,
		nextImport: 1,
		conns:      map[uint64]*connectionState{},
		cursors:    map[string]*cursorState{},
		txs:        map[string]*txState{},
		imports:    map[string]*importState{},
		streams:    map[string]*streamState{},
	}
}

func DeclaredMethods() []string {
	return []string{
		"$/ping",
		"shutdown",
		"conn/test",
		"conn/open",
		"conn/close",
		"conn/ping",
		"conn/use",
		"query/start",
		"cursor/fetch",
		"cursor/close",
		"cursor/cancel",
		"exec/run",
		"exec/batch",
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
		"schema/object_view",
		"schema/databases",
		"schema/schemas",
		"schema/objects",
		"schema/columns",
		"schema/indexes",
		"schema/foreign_keys",
		"schema/checks",
		"schema/views",
		"schema/functions",
		"schema/procedures",
		"schema/triggers",
		"schema/sequences",
		"schema/types",
		"schema/view_definition",
		"schema/dump_ddl",
	}
}

func (s *Server) Handle(ctx context.Context, req ipc.Message) ipc.Message {
	s.mu.Lock()
	defer s.mu.Unlock()

	if req.JSONRPC != "" && req.JSONRPC != ipc.JSONRPCVersion {
		return s.err(req.ID, ErrInvalidRequest, "jsonrpc must be 2.0")
	}
	if len(req.ID) == 0 {
		req.ID = json.RawMessage(`null`)
	}

	if req.Method != "init" && req.Method != "$/ping" && req.Method != "shutdown" && !s.initialized {
		return s.err(req.ID, ErrNotInitialized, "init must be called first")
	}

	switch req.Method {
	case "init":
		s.initialized = true
		return s.ok(req.ID, map[string]any{
			"extension_version": "0.1.2",
			"api_used":          map[string]string{"database": "1.0"},
			"features":          []string{"streaming", "schema_introspection", "rich_errors"},
			"drivers_ready":     []string{s.spec.ID},
			"methods":           DeclaredMethods(),
			"name":              s.spec.Name + " IPC Driver",
		})
	case "$/ping":
		return s.ok(req.ID, map[string]bool{"pong": true})
	case "shutdown":
		s.closeAll()
		return s.ok(req.ID, nil)
	case "conn/test":
		return s.handleConnTest(ctx, req)
	case "conn/open":
		return s.handleConnOpen(ctx, req)
	case "conn/close":
		return s.handleConnClose(req)
	case "conn/ping":
		return s.handleConnPing(ctx, req)
	case "conn/use":
		return s.handleConnUse(req)
	case "query/start":
		return s.handleQueryStart(ctx, req)
	case "cursor/fetch":
		return s.handleCursorFetch(req)
	case "cursor/close":
		return s.handleCursorClose(req)
	case "cursor/cancel":
		return s.handleCursorCancel(req)
	case "exec/run":
		return s.handleExecRun(ctx, req)
	case "exec/batch":
		return s.handleExecBatch(ctx, req)
	case "tx/begin":
		return s.handleTxBegin(ctx, req)
	case "tx/commit":
		return s.handleTxCommit(req)
	case "tx/rollback":
		return s.handleTxRollback(req)
	case "tx/savepoint":
		return s.handleTxSavepoint(ctx, req)
	case "tx/release":
		return s.handleTxRelease(ctx, req)
	case "ddl/build":
		return s.handleDdlBuild(req)
	case "ddl/build_create_table":
		return s.handleDdlBuildCreateTable(req)
	case "ddl/build_alter_table":
		return s.handleDdlBuildAlterTable(req)
	case "ddl/build_drop":
		return s.handleDdlBuildDrop(req)
	case "data/export":
		return s.handleDataExport(ctx, req)
	case "data/import_begin":
		return s.handleDataImportBegin(req)
	case "data/import_chunk":
		return s.handleDataImportChunk(ctx, req)
	case "data/import_commit":
		return s.handleDataImportCommit(req)
	case "data/import_abort":
		return s.handleDataImportAbort(req)
	case "stream/read":
		return s.handleStreamRead(req)
	case "stream/close":
		return s.handleStreamClose(req)
	case "schema/object_view":
		return s.handleSchemaObjectView(ctx, req)
	case "schema/databases":
		return s.handleSchemaDatabases(ctx, req)
	case "schema/schemas":
		return s.handleSchemaSchemas(ctx, req)
	case "schema/objects":
		return s.handleSchemaObjects(ctx, req)
	case "schema/columns":
		return s.handleSchemaColumns(ctx, req)
	case "schema/indexes":
		return s.handleSchemaIndexes(ctx, req)
	case "schema/foreign_keys":
		return s.handleSchemaForeignKeys(ctx, req)
	case "schema/checks":
		return s.handleEmptySchemaList(req)
	case "schema/views":
		return s.handleSchemaViews(ctx, req)
	case "schema/functions":
		return s.handleSchemaFunctions(ctx, req)
	case "schema/procedures", "schema/triggers", "schema/sequences", "schema/types":
		return s.handleEmptySchemaList(req)
	case "schema/view_definition":
		return s.handleSchemaViewDefinition(ctx, req)
	case "schema/dump_ddl":
		return s.handleEmptyDumpDDL(req)
	default:
		return s.err(req.ID, ErrMethodNotFound, fmt.Sprintf("method `%s` is not implemented", req.Method))
	}
}

func (s *Server) handleConnTest(ctx context.Context, req ipc.Message) ipc.Message {
	cfg, connSpec, err := s.parseConfig(ctx, req.Params)
	if err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	start := time.Now()
	db, err := s.opener(connSpec.DriverName, connSpec.DSN)
	if err != nil {
		return s.err(req.ID, ErrConnectionFailed, err.Error())
	}
	defer db.Close()
	if err := db.PingContext(ctx); err != nil {
		return s.err(req.ID, ErrConnectionFailed, err.Error())
	}
	return s.ok(req.ID, map[string]any{
		"ok":             true,
		"latency_ms":     uint32(time.Since(start).Milliseconds()),
		"warnings":       []string{},
		"server_version": cfg.Database,
	})
}

func (s *Server) handleConnOpen(ctx context.Context, req ipc.Message) ipc.Message {
	cfg, connSpec, err := s.parseConfig(ctx, req.Params)
	if err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	db, err := s.opener(connSpec.DriverName, connSpec.DSN)
	if err != nil {
		return s.err(req.ID, ErrConnectionFailed, err.Error())
	}
	if err := db.PingContext(ctx); err != nil {
		db.Close()
		return s.err(req.ID, ErrConnectionFailed, err.Error())
	}

	connID := s.nextConnID
	s.nextConnID++
	s.conns[connID] = &connectionState{config: cfg, db: db, schemaSQL: connSpec.SchemaSQL}
	return s.ok(req.ID, map[string]any{
		"conn_id": connID,
		"server_info": map[string]any{
			"version":  s.spec.Name,
			"features": []string{"database_sql"},
		},
	})
}

func (s *Server) handleConnClose(req ipc.Message) ipc.Message {
	var p struct {
		ConnID uint64 `json:"conn_id"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	conn, ok := s.conns[p.ConnID]
	if !ok {
		return s.err(req.ID, ErrUnknownConnID, fmt.Sprintf("unknown conn_id %d", p.ConnID))
	}
	s.closeCursorsForConn(p.ConnID)
	s.rollbackTxsForConn(p.ConnID)
	s.dropImportsForConn(p.ConnID)
	conn.db.Close()
	delete(s.conns, p.ConnID)
	return s.ok(req.ID, nil)
}

func (s *Server) handleConnPing(ctx context.Context, req ipc.Message) ipc.Message {
	conn, errResp := s.connFromParams(req)
	if errResp != nil {
		return *errResp
	}
	start := time.Now()
	if err := conn.db.PingContext(ctx); err != nil {
		return s.err(req.ID, ErrConnectionFailed, err.Error())
	}
	return s.ok(req.ID, map[string]any{"latency_ms": uint32(time.Since(start).Milliseconds())})
}

func (s *Server) handleConnUse(req ipc.Message) ipc.Message {
	var p struct {
		ConnID   uint64 `json:"conn_id"`
		Database string `json:"database,omitempty"`
		Schema   string `json:"schema,omitempty"`
		Role     string `json:"role,omitempty"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	conn, ok := s.conns[p.ConnID]
	if !ok {
		return s.err(req.ID, ErrUnknownConnID, fmt.Sprintf("unknown conn_id %d", p.ConnID))
	}
	if p.Database != "" {
		conn.config.Database = p.Database
	}
	return s.ok(req.ID, nil)
}

func (s *Server) handleQueryStart(ctx context.Context, req ipc.Message) ipc.Message {
	var p struct {
		ConnID  uint64      `json:"conn_id"`
		SQL     string      `json:"sql"`
		Params  []cellValue `json:"params,omitempty"`
		MaxRows *uint64     `json:"max_rows,omitempty"`
		TxID    string      `json:"tx_id,omitempty"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	conn, ok := s.conns[p.ConnID]
	if !ok {
		return s.err(req.ID, ErrUnknownConnID, fmt.Sprintf("unknown conn_id %d", p.ConnID))
	}
	args, err := paramsFromWire(p.Params)
	if err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	var queryer queryExecutor = conn.db
	if p.TxID != "" {
		tx, errResp := s.txForRequest(req.ID, p.TxID, p.ConnID)
		if errResp != nil {
			return *errResp
		}
		queryer = tx.tx
	}
	columns, rows, err := startQuery(ctx, queryer, p.SQL, args)
	if err != nil {
		return s.err(req.ID, ErrSQLSyntax, err.Error())
	}
	cursorID := fmt.Sprintf("%s-cursor-%d", s.spec.ID, s.nextCursor)
	s.nextCursor++
	s.cursors[cursorID] = &cursorState{
		connID:      p.ConnID,
		rows:        rows,
		columnCount: len(columns),
		maxRows:     p.MaxRows,
	}
	return s.ok(req.ID, map[string]any{
		"cursor_id":       cursorID,
		"columns":         columns,
		"row_count_known": false,
	})
}

func (s *Server) handleCursorFetch(req ipc.Message) ipc.Message {
	var p struct {
		CursorID string  `json:"cursor_id"`
		N        *uint32 `json:"n,omitempty"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	cursor, ok := s.cursors[p.CursorID]
	if !ok {
		return s.err(req.ID, ErrUnknownCursorID, fmt.Sprintf("unknown cursor_id `%s`", p.CursorID))
	}
	n := 500
	if p.N != nil && *p.N > 0 {
		n = int(*p.N)
	}
	if cursor.done || cursor.rows == nil {
		return s.ok(req.ID, map[string]any{"rows": [][]cellValue{}, "done": true})
	}
	rows, done, fetched, err := fetchRows(cursor.rows, cursor.columnCount, n, cursor.maxRows, cursor.fetched)
	if err != nil {
		return s.err(req.ID, ErrSQLSyntax, err.Error())
	}
	cursor.fetched = fetched
	if done {
		cursor.done = true
		if err := cursor.rows.Close(); err != nil {
			return s.err(req.ID, ErrSQLSyntax, err.Error())
		}
		cursor.rows = nil
	}
	return s.ok(req.ID, map[string]any{"rows": rows, "done": done})
}

func (s *Server) handleCursorClose(req ipc.Message) ipc.Message {
	var p struct {
		CursorID string `json:"cursor_id"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	if _, ok := s.cursors[p.CursorID]; !ok {
		return s.err(req.ID, ErrUnknownCursorID, fmt.Sprintf("unknown cursor_id `%s`", p.CursorID))
	}
	if err := s.closeCursor(p.CursorID); err != nil {
		return s.err(req.ID, ErrSQLSyntax, err.Error())
	}
	delete(s.cursors, p.CursorID)
	return s.ok(req.ID, nil)
}

func (s *Server) handleCursorCancel(req ipc.Message) ipc.Message {
	var p struct {
		CursorID string `json:"cursor_id"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	cursor, ok := s.cursors[p.CursorID]
	if !ok {
		return s.err(req.ID, ErrUnknownCursorID, fmt.Sprintf("unknown cursor_id `%s`", p.CursorID))
	}
	if err := s.closeCursor(p.CursorID); err != nil {
		return s.err(req.ID, ErrSQLSyntax, err.Error())
	}
	cursor.done = true
	return s.ok(req.ID, nil)
}

func (s *Server) handleExecRun(ctx context.Context, req ipc.Message) ipc.Message {
	var p struct {
		ConnID uint64      `json:"conn_id"`
		SQL    string      `json:"sql"`
		Params []cellValue `json:"params,omitempty"`
		TxID   string      `json:"tx_id,omitempty"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	conn, ok := s.conns[p.ConnID]
	if !ok {
		return s.err(req.ID, ErrUnknownConnID, fmt.Sprintf("unknown conn_id %d", p.ConnID))
	}
	args, err := paramsFromWire(p.Params)
	if err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	var execer interface {
		ExecContext(context.Context, string, ...any) (sql.Result, error)
	} = conn.db
	if p.TxID != "" {
		tx, errResp := s.txForRequest(req.ID, p.TxID, p.ConnID)
		if errResp != nil {
			return *errResp
		}
		execer = tx.tx
	}
	res, err := execer.ExecContext(ctx, p.SQL, args...)
	if err != nil {
		return s.err(req.ID, ErrSQLSyntax, err.Error())
	}
	affected, _ := res.RowsAffected()
	return s.ok(req.ID, map[string]any{"affected_rows": uint64(affected), "warnings": []string{}})
}

func (s *Server) handleExecBatch(ctx context.Context, req ipc.Message) ipc.Message {
	var p struct {
		ConnID        uint64   `json:"conn_id"`
		Statements    []string `json:"statements"`
		StopOnError   bool     `json:"stop_on_error"`
		InTransaction bool     `json:"in_transaction"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	conn, ok := s.conns[p.ConnID]
	if !ok {
		return s.err(req.ID, ErrUnknownConnID, fmt.Sprintf("unknown conn_id %d", p.ConnID))
	}

	var execer interface {
		ExecContext(context.Context, string, ...any) (sql.Result, error)
	} = conn.db
	var tx *sql.Tx
	if p.InTransaction {
		var err error
		tx, err = conn.db.BeginTx(ctx, nil)
		if err != nil {
			return s.err(req.ID, ErrSQLSyntax, err.Error())
		}
		execer = tx
	}

	results := make([]map[string]any, len(p.Statements))
	for i := range results {
		results[i] = map[string]any{"affected_rows": uint64(0), "warnings": []string{}}
	}
	errorsOut := make([]map[string]any, 0)
	for index, statement := range p.Statements {
		res, err := execer.ExecContext(ctx, statement)
		if err != nil {
			errorsOut = append(errorsOut, map[string]any{
				"index":   index,
				"code":    ErrSQLSyntax,
				"message": err.Error(),
			})
			if p.StopOnError {
				break
			}
			continue
		}
		affected, _ := res.RowsAffected()
		results[index] = map[string]any{"affected_rows": uint64(affected), "warnings": []string{}}
	}

	if tx != nil {
		if len(errorsOut) > 0 {
			_ = tx.Rollback()
		} else if err := tx.Commit(); err != nil {
			return s.err(req.ID, ErrSQLSyntax, err.Error())
		}
	}

	return s.ok(req.ID, map[string]any{"results": results, "errors": errorsOut})
}

func (s *Server) handleTxBegin(ctx context.Context, req ipc.Message) ipc.Message {
	var p struct {
		ConnID    uint64 `json:"conn_id"`
		Isolation string `json:"isolation,omitempty"`
		ReadOnly  bool   `json:"read_only,omitempty"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	conn, ok := s.conns[p.ConnID]
	if !ok {
		return s.err(req.ID, ErrUnknownConnID, fmt.Sprintf("unknown conn_id %d", p.ConnID))
	}
	tx, err := conn.db.BeginTx(ctx, &sql.TxOptions{Isolation: isolationLevel(p.Isolation), ReadOnly: p.ReadOnly})
	if err != nil {
		return s.err(req.ID, ErrSQLSyntax, err.Error())
	}
	txID := fmt.Sprintf("%s-tx-%d", s.spec.ID, s.nextTx)
	s.nextTx++
	s.txs[txID] = &txState{connID: p.ConnID, tx: tx}
	return s.ok(req.ID, map[string]any{"tx_id": txID})
}

func (s *Server) handleTxCommit(req ipc.Message) ipc.Message {
	var p struct {
		TxID string `json:"tx_id"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	tx, ok := s.txs[p.TxID]
	if !ok {
		return s.err(req.ID, ErrInvalidParams, fmt.Sprintf("unknown tx_id `%s`", p.TxID))
	}
	if err := tx.tx.Commit(); err != nil {
		return s.err(req.ID, ErrSQLSyntax, err.Error())
	}
	delete(s.txs, p.TxID)
	return s.ok(req.ID, nil)
}

func (s *Server) handleTxRollback(req ipc.Message) ipc.Message {
	var p struct {
		TxID        string `json:"tx_id"`
		ToSavepoint string `json:"to_savepoint,omitempty"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	tx, ok := s.txs[p.TxID]
	if !ok {
		return s.err(req.ID, ErrInvalidParams, fmt.Sprintf("unknown tx_id `%s`", p.TxID))
	}
	if p.ToSavepoint != "" {
		if _, err := tx.tx.Exec("ROLLBACK TO SAVEPOINT " + quoteIdentifier(s.spec, p.ToSavepoint)); err != nil {
			return s.err(req.ID, ErrSQLSyntax, err.Error())
		}
		return s.ok(req.ID, nil)
	}
	if err := tx.tx.Rollback(); err != nil {
		return s.err(req.ID, ErrSQLSyntax, err.Error())
	}
	delete(s.txs, p.TxID)
	return s.ok(req.ID, nil)
}

func (s *Server) handleTxSavepoint(ctx context.Context, req ipc.Message) ipc.Message {
	var p struct {
		TxID string `json:"tx_id"`
		Name string `json:"name"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	tx, ok := s.txs[p.TxID]
	if !ok {
		return s.err(req.ID, ErrInvalidParams, fmt.Sprintf("unknown tx_id `%s`", p.TxID))
	}
	if p.Name == "" {
		return s.err(req.ID, ErrInvalidParams, "missing required parameter `name`")
	}
	if _, err := tx.tx.ExecContext(ctx, "SAVEPOINT "+quoteIdentifier(s.spec, p.Name)); err != nil {
		return s.err(req.ID, ErrSQLSyntax, err.Error())
	}
	return s.ok(req.ID, nil)
}

func (s *Server) handleTxRelease(ctx context.Context, req ipc.Message) ipc.Message {
	var p struct {
		TxID string `json:"tx_id"`
		Name string `json:"name"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	tx, ok := s.txs[p.TxID]
	if !ok {
		return s.err(req.ID, ErrInvalidParams, fmt.Sprintf("unknown tx_id `%s`", p.TxID))
	}
	if p.Name == "" {
		return s.err(req.ID, ErrInvalidParams, "missing required parameter `name`")
	}
	if _, err := tx.tx.ExecContext(ctx, "RELEASE SAVEPOINT "+quoteIdentifier(s.spec, p.Name)); err != nil {
		return s.err(req.ID, ErrSQLSyntax, err.Error())
	}
	return s.ok(req.ID, nil)
}

func (s *Server) txForRequest(id json.RawMessage, txID string, connID uint64) (*txState, *ipc.Message) {
	tx, ok := s.txs[txID]
	if !ok {
		resp := s.err(id, ErrInvalidParams, fmt.Sprintf("unknown tx_id `%s`", txID))
		return nil, &resp
	}
	if tx.connID != connID {
		resp := s.err(id, ErrInvalidParams, fmt.Sprintf("tx_id `%s` does not belong to conn_id %d", txID, connID))
		return nil, &resp
	}
	return tx, nil
}

func isolationLevel(value string) sql.IsolationLevel {
	switch value {
	case "read_uncommitted":
		return sql.LevelReadUncommitted
	case "read_committed":
		return sql.LevelReadCommitted
	case "repeatable_read":
		return sql.LevelRepeatableRead
	case "serializable":
		return sql.LevelSerializable
	default:
		return sql.LevelDefault
	}
}

func (s *Server) handleDdlBuild(req ipc.Message) ipc.Message {
	var p struct {
		Op      string          `json:"op"`
		Payload json.RawMessage `json:"payload"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	switch p.Op {
	case "create_table":
		var payload struct {
			Spec    tableSpec          `json:"spec"`
			Options createTableOptions `json:"options"`
		}
		if err := decodePayload(p.Payload, &payload); err != nil {
			return s.err(req.ID, ErrInvalidParams, err.Error())
		}
		_, statements, err := buildCreateTableSQL(s.spec, payload.Spec, payload.Options)
		if err != nil {
			return s.err(req.ID, ErrInvalidParams, err.Error())
		}
		return s.ok(req.ID, map[string]any{"statements": statements, "warnings": []string{}})
	case "drop_table", "drop_view":
		var payload struct {
			Kind     string `json:"kind"`
			Name     string `json:"name"`
			Schema   string `json:"schema,omitempty"`
			Database string `json:"database,omitempty"`
			IfExists bool   `json:"if_exists,omitempty"`
			Cascade  bool   `json:"cascade,omitempty"`
		}
		if err := decodePayload(p.Payload, &payload); err != nil {
			return s.err(req.ID, ErrInvalidParams, err.Error())
		}
		if payload.Kind == "" {
			if p.Op == "drop_view" {
				payload.Kind = "view"
			} else {
				payload.Kind = "table"
			}
		}
		sqlText, err := buildDropSQL(s.spec, payload.Kind, payload.Database, payload.Schema, payload.Name, payload.IfExists, payload.Cascade)
		if err != nil {
			return s.err(req.ID, ErrInvalidParams, err.Error())
		}
		return s.ok(req.ID, map[string]any{"statements": []string{sqlText}, "warnings": []string{}})
	default:
		return s.err(req.ID, ErrNotSupported, fmt.Sprintf("ddl op %q is not supported by the generic builder", p.Op))
	}
}

func (s *Server) handleDdlBuildCreateTable(req ipc.Message) ipc.Message {
	var p struct {
		Spec    tableSpec          `json:"spec"`
		Options createTableOptions `json:"options"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	sqlText, statements, err := buildCreateTableSQL(s.spec, p.Spec, p.Options)
	if err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	return s.ok(req.ID, map[string]any{"sql": sqlText, "statements": statements})
}

func (s *Server) handleDdlBuildAlterTable(req ipc.Message) ipc.Message {
	var p struct {
		FromSpec      tableSpec         `json:"from_spec"`
		ToSpec        tableSpec         `json:"to_spec"`
		ColumnRenames []columnRenameDDL `json:"column_renames"`
		Options       alterTableOptions `json:"options"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	statements, rollback, warnings, err := buildAlterTableSQL(s.spec, p.FromSpec, p.ToSpec, p.ColumnRenames, p.Options)
	if err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	return s.ok(req.ID, map[string]any{"statements": statements, "rollback_statements": rollback, "warnings": warnings})
}

func (s *Server) handleDdlBuildDrop(req ipc.Message) ipc.Message {
	var p struct {
		Kind     string `json:"kind"`
		Name     string `json:"name"`
		Schema   string `json:"schema,omitempty"`
		Database string `json:"database,omitempty"`
		IfExists bool   `json:"if_exists,omitempty"`
		Cascade  bool   `json:"cascade,omitempty"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	sqlText, err := buildDropSQL(s.spec, p.Kind, p.Database, p.Schema, p.Name, p.IfExists, p.Cascade)
	if err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	return s.ok(req.ID, map[string]any{"sql": sqlText})
}

func (s *Server) handleDataExport(ctx context.Context, req ipc.Message) ipc.Message {
	var p struct {
		ConnID         uint64      `json:"conn_id"`
		Table          string      `json:"table,omitempty"`
		Schema         string      `json:"schema,omitempty"`
		Database       string      `json:"database,omitempty"`
		SQL            string      `json:"sql,omitempty"`
		Format         string      `json:"format"`
		WhereClause    string      `json:"where,omitempty"`
		IncludeColumns []string    `json:"include_columns,omitempty"`
		ExcludeColumns []string    `json:"exclude_columns,omitempty"`
		MaxRows        *uint64     `json:"max_rows,omitempty"`
		Params         []cellValue `json:"params,omitempty"`
		StreamID       string      `json:"stream_id"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	conn, ok := s.conns[p.ConnID]
	if !ok {
		return s.err(req.ID, ErrUnknownConnID, fmt.Sprintf("unknown conn_id %d", p.ConnID))
	}
	if p.StreamID == "" {
		return s.err(req.ID, ErrInvalidParams, "missing required parameter `stream_id`")
	}
	sqlText, err := buildExportSQL(s.spec, p.Table, p.Schema, p.Database, p.SQL, p.WhereClause, p.IncludeColumns, p.ExcludeColumns)
	if err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	args, err := paramsFromWire(p.Params)
	if err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	data, metadata, count, err := exportRows(ctx, conn.db, sqlText, p.Format, args)
	if err != nil {
		return s.err(req.ID, ErrNotSupported, err.Error())
	}
	s.streams[p.StreamID] = &streamState{data: data}
	return s.ok(req.ID, map[string]any{"estimated_bytes": uint64(len(data)), "estimated_rows": count, "metadata": metadata})
}

func (s *Server) handleDataImportBegin(req ipc.Message) ipc.Message {
	var p struct {
		ConnID   uint64   `json:"conn_id"`
		Table    string   `json:"table"`
		Schema   string   `json:"schema,omitempty"`
		Database string   `json:"database,omitempty"`
		Format   string   `json:"format"`
		Columns  []string `json:"columns,omitempty"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	if _, ok := s.conns[p.ConnID]; !ok {
		return s.err(req.ID, ErrUnknownConnID, fmt.Sprintf("unknown conn_id %d", p.ConnID))
	}
	switch p.Format {
	case "json", "ndjson", "csv":
	default:
		return s.err(req.ID, ErrNotSupported, fmt.Sprintf("import format %q is not supported by the generic database/sql driver", p.Format))
	}
	importID := fmt.Sprintf("%s-import-%d", s.spec.ID, s.nextImport)
	s.nextImport++
	s.imports[importID] = &importState{
		connID:   p.ConnID,
		table:    p.Table,
		schema:   p.Schema,
		database: p.Database,
		columns:  p.Columns,
		format:   p.Format,
		started:  time.Now(),
	}
	return s.ok(req.ID, map[string]any{"import_id": importID})
}

func (s *Server) handleDataImportChunk(ctx context.Context, req ipc.Message) ipc.Message {
	var p struct {
		ImportID string        `json:"import_id"`
		Rows     [][]cellValue `json:"rows"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	imp, ok := s.imports[p.ImportID]
	if !ok {
		return s.err(req.ID, ErrInvalidParams, fmt.Sprintf("unknown import_id `%s`", p.ImportID))
	}
	conn, ok := s.conns[imp.connID]
	if !ok {
		return s.err(req.ID, ErrUnknownConnID, fmt.Sprintf("unknown conn_id %d", imp.connID))
	}
	sqlText, err := buildInsertSQL(s.spec, imp)
	if err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	var inserted uint64
	failed := []map[string]any{}
	for index, row := range p.Rows {
		args, err := cellsToArgs(row)
		if err == nil {
			_, err = conn.db.ExecContext(ctx, sqlText, args...)
		}
		if err != nil {
			failed = append(failed, map[string]any{"row_index": uint64(index), "message": err.Error(), "code": ErrSQLSyntax})
			continue
		}
		inserted++
		imp.inserted++
	}
	imp.failed = append(imp.failed, failed...)
	return s.ok(req.ID, map[string]any{"inserted": inserted, "failed": failed})
}

func (s *Server) handleDataImportCommit(req ipc.Message) ipc.Message {
	var p struct {
		ImportID string `json:"import_id"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	imp, ok := s.imports[p.ImportID]
	if !ok {
		return s.err(req.ID, ErrInvalidParams, fmt.Sprintf("unknown import_id `%s`", p.ImportID))
	}
	delete(s.imports, p.ImportID)
	return s.ok(req.ID, map[string]any{
		"inserted":   imp.inserted,
		"updated":    uint64(0),
		"deleted":    uint64(0),
		"failed":     imp.failed,
		"elapsed_ms": uint64(time.Since(imp.started).Milliseconds()),
	})
}

func (s *Server) handleDataImportAbort(req ipc.Message) ipc.Message {
	var p struct {
		ImportID string `json:"import_id"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	delete(s.imports, p.ImportID)
	return s.ok(req.ID, nil)
}

func (s *Server) handleStreamRead(req ipc.Message) ipc.Message {
	var p struct {
		StreamID string  `json:"stream_id"`
		MaxBytes *uint32 `json:"max_bytes,omitempty"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	stream, ok := s.streams[p.StreamID]
	if !ok {
		return s.err(req.ID, ErrInvalidParams, fmt.Sprintf("unknown stream_id `%s`", p.StreamID))
	}
	var maxBytes uint32
	if p.MaxBytes != nil {
		maxBytes = *p.MaxBytes
	}
	result := streamReadChunk(stream, maxBytes)
	if done, _ := result["done"].(bool); done {
		delete(s.streams, p.StreamID)
	}
	return s.ok(req.ID, result)
}

func (s *Server) handleStreamClose(req ipc.Message) ipc.Message {
	var p struct {
		StreamID string `json:"stream_id"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	delete(s.streams, p.StreamID)
	return s.ok(req.ID, nil)
}

type objectViewParams struct {
	ConnID   uint64 `json:"conn_id"`
	View     string `json:"view"`
	Database string `json:"database,omitempty"`
	Schema   string `json:"schema,omitempty"`
	Table    string `json:"table,omitempty"`
}

type objectView struct {
	Title   string             `json:"title,omitempty"`
	Columns []objectViewColumn `json:"columns"`
	Rows    [][]string         `json:"rows"`
}

type objectViewColumn struct {
	Key     string   `json:"key"`
	Name    string   `json:"name"`
	WidthPx *float64 `json:"width_px,omitempty"`
	Align   string   `json:"align,omitempty"`
}

func (s *Server) handleSchemaObjectView(ctx context.Context, req ipc.Message) ipc.Message {
	var p objectViewParams
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	conn, ok := s.conns[p.ConnID]
	if !ok {
		return s.err(req.ID, ErrUnknownConnID, fmt.Sprintf("unknown conn_id %d", p.ConnID))
	}

	switch p.View {
	case "databases":
		if conn.schemaSQL.Databases == nil {
			return s.ok(req.ID, objectViewResult("Databases", objectViewColumns("name", "Name"), [][]string{{conn.config.Database}}))
		}
		rows, err := queryObjectViewRows(ctx, conn.db, conn.schemaSQL.Databases(conn.config), func(cols []any) []string {
			return []string{stringCell(cols, 0)}
		})
		if err != nil {
			return s.err(req.ID, ErrSQLSyntax, err.Error())
		}
		return s.ok(req.ID, objectViewResult("Databases", objectViewColumns("name", "Name"), rows))
	case "schemas":
		if conn.schemaSQL.Schemas == nil {
			return s.ok(req.ID, objectViewResult("Schemas", objectViewColumns("name", "Name"), [][]string{{conn.config.Username}}))
		}
		rows, err := queryObjectViewRows(ctx, conn.db, conn.schemaSQL.Schemas(conn.config, p.Database), func(cols []any) []string {
			return []string{stringCell(cols, 0), stringCell(cols, 1)}
		})
		if err != nil {
			return s.err(req.ID, ErrSQLSyntax, err.Error())
		}
		return s.ok(req.ID, objectViewResult("Schemas", objectViewColumns("name", "Name", "owner", "Owner"), rows))
	case "tables":
		return s.handleObjectListView(ctx, req.ID, conn, p, "Tables", []string{"table"})
	case "views":
		if conn.schemaSQL.Views == nil {
			return s.ok(req.ID, objectViewResult("Views", objectViewColumns("name", "Name", "kind", "Kind", "comment", "Comment"), [][]string{}))
		}
		rows, err := queryObjectViewRows(ctx, conn.db, conn.schemaSQL.Views(conn.config, p.Database, p.Schema), func(cols []any) []string {
			return []string{stringCell(cols, 0), stringCell(cols, 1), stringCell(cols, 2)}
		})
		if err != nil {
			return s.err(req.ID, ErrSQLSyntax, err.Error())
		}
		return s.ok(req.ID, objectViewResult("Views", objectViewColumns("name", "Name", "kind", "Kind", "comment", "Comment"), rows))
	case "columns":
		if p.Table == "" {
			return s.err(req.ID, ErrInvalidParams, "missing required parameter `table`")
		}
		if conn.schemaSQL.Columns == nil {
			return s.ok(req.ID, objectViewResult("Columns", columnObjectViewColumns(), [][]string{}))
		}
		rows, err := queryObjectViewRows(ctx, conn.db, conn.schemaSQL.Columns(conn.config, p.Database, p.Schema, p.Table), func(cols []any) []string {
			return []string{
				stringCell(cols, 1),
				stringCell(cols, 2),
				fmt.Sprint(boolCell(cols, 3)),
				stringCell(cols, 4),
				"",
			}
		})
		if err != nil {
			return s.err(req.ID, ErrSQLSyntax, err.Error())
		}
		return s.ok(req.ID, objectViewResult("Columns", columnObjectViewColumns(), rows))
	case "indexes":
		if p.Table == "" {
			return s.err(req.ID, ErrInvalidParams, "missing required parameter `table`")
		}
		if conn.schemaSQL.Indexes == nil {
			return s.ok(req.ID, objectViewResult("Indexes", indexObjectViewColumns(), [][]string{}))
		}
		rows, err := queryObjectViewRows(ctx, conn.db, conn.schemaSQL.Indexes(conn.config, p.Database, p.Schema, p.Table), func(cols []any) []string {
			return []string{
				stringCell(cols, 0),
				strings.Join(splitListCell(cols, 1), ", "),
				fmt.Sprint(boolCell(cols, 2)),
				fmt.Sprint(boolCell(cols, 3)),
				stringCell(cols, 4),
			}
		})
		if err != nil {
			return s.err(req.ID, ErrSQLSyntax, err.Error())
		}
		return s.ok(req.ID, objectViewResult("Indexes", indexObjectViewColumns(), rows))
	case "functions":
		if conn.schemaSQL.Functions == nil {
			return s.ok(req.ID, objectViewResult("Functions", objectViewColumns("name", "Name", "returns", "Returns", "language", "Language", "comment", "Comment"), [][]string{}))
		}
		rows, err := queryObjectViewRows(ctx, conn.db, conn.schemaSQL.Functions(conn.config, p.Database, p.Schema), func(cols []any) []string {
			return []string{stringCell(cols, 0), stringCell(cols, 2), stringCell(cols, 3), stringCell(cols, 4)}
		})
		if err != nil {
			return s.err(req.ID, ErrSQLSyntax, err.Error())
		}
		return s.ok(req.ID, objectViewResult("Functions", objectViewColumns("name", "Name", "returns", "Returns", "language", "Language", "comment", "Comment"), rows))
	case "procedures", "triggers", "sequences":
		return s.ok(req.ID, objectViewResult(titleForObjectView(p.View), objectViewColumns("name", "Name"), [][]string{}))
	default:
		return s.err(req.ID, ErrNotSupported, fmt.Sprintf("unsupported object view %q", p.View))
	}
}

func (s *Server) handleObjectListView(ctx context.Context, id json.RawMessage, conn *connectionState, p objectViewParams, title string, kinds []string) ipc.Message {
	columns := objectViewColumns("name", "Name", "kind", "Kind", "comment", "Comment")
	if conn.schemaSQL.Objects == nil {
		return s.ok(id, objectViewResult(title, columns, [][]string{}))
	}
	rows, err := queryObjectViewRows(ctx, conn.db, conn.schemaSQL.Objects(conn.config, p.Database, p.Schema, kinds), func(cols []any) []string {
		return []string{stringCell(cols, 0), stringCell(cols, 1), stringCell(cols, 2)}
	})
	if err != nil {
		return s.err(id, ErrSQLSyntax, err.Error())
	}
	return s.ok(id, objectViewResult(title, columns, rows))
}

func queryObjectViewRows(ctx context.Context, db *sql.DB, sqlText string, mapRow func([]any) []string) ([][]string, error) {
	rows, err := db.QueryContext(ctx, sqlText)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	cols, err := rows.Columns()
	if err != nil {
		return nil, err
	}
	out := [][]string{}
	for rows.Next() {
		values := make([]any, len(cols))
		ptrs := make([]any, len(cols))
		for i := range values {
			ptrs[i] = &values[i]
		}
		if err := rows.Scan(ptrs...); err != nil {
			return nil, err
		}
		out = append(out, mapRow(values))
	}
	return out, rows.Err()
}

func objectViewResult(title string, columns []objectViewColumn, rows [][]string) objectView {
	return objectView{Title: title, Columns: columns, Rows: rows}
}

func columnObjectViewColumns() []objectViewColumn {
	return []objectViewColumn{
		objectViewColumnWithWidth("name", "Field", 220, ""),
		objectViewColumnWithWidth("type", "Type", 160, ""),
		objectViewColumnWithWidth("nullable", "Null?", 72, "right"),
		objectViewColumnWithWidth("default", "Default", 180, ""),
		objectViewColumnWithWidth("comment", "Comment", 260, ""),
	}
}

func indexObjectViewColumns() []objectViewColumn {
	return []objectViewColumn{
		objectViewColumnWithWidth("name", "Name", 220, ""),
		objectViewColumnWithWidth("columns", "Columns", 220, ""),
		objectViewColumnWithWidth("unique", "Unique?", 90, "right"),
		objectViewColumnWithWidth("primary", "Primary?", 90, "right"),
		objectViewColumnWithWidth("type", "Type", 140, ""),
	}
}

func objectViewColumns(values ...string) []objectViewColumn {
	columns := make([]objectViewColumn, 0, len(values)/2)
	for i := 0; i+1 < len(values); i += 2 {
		width := 0.0
		if values[i] == "name" {
			width = 220
		}
		columns = append(columns, objectViewColumnWithWidth(values[i], values[i+1], width, ""))
	}
	return columns
}

func objectViewColumnWithWidth(key, name string, width float64, align string) objectViewColumn {
	var widthPtr *float64
	if width > 0 {
		widthPtr = &width
	}
	return objectViewColumn{Key: key, Name: name, WidthPx: widthPtr, Align: align}
}

func titleForObjectView(view string) string {
	if view == "" {
		return ""
	}
	return strings.ToUpper(view[:1]) + view[1:]
}

func (s *Server) handleSchemaDatabases(ctx context.Context, req ipc.Message) ipc.Message {
	conn, errResp := s.connFromParams(req)
	if errResp != nil {
		return *errResp
	}
	sqlText := ""
	if conn.schemaSQL.Databases != nil {
		sqlText = conn.schemaSQL.Databases(conn.config)
	}
	if sqlText == "" {
		return s.ok(req.ID, []map[string]any{{"name": conn.config.Database}})
	}
	rows, err := queryObjects(ctx, conn.db, sqlText, func(cols []any) map[string]any {
		return map[string]any{"name": stringCell(cols, 0)}
	})
	if err != nil {
		return s.err(req.ID, ErrSQLSyntax, err.Error())
	}
	return s.ok(req.ID, rows)
}

func (s *Server) handleSchemaSchemas(ctx context.Context, req ipc.Message) ipc.Message {
	var p struct {
		ConnID   uint64 `json:"conn_id"`
		Database string `json:"database"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	conn, ok := s.conns[p.ConnID]
	if !ok {
		return s.err(req.ID, ErrUnknownConnID, fmt.Sprintf("unknown conn_id %d", p.ConnID))
	}
	if conn.schemaSQL.Schemas == nil {
		return s.ok(req.ID, []map[string]any{{"name": conn.config.Username}})
	}
	rows, err := queryObjects(ctx, conn.db, conn.schemaSQL.Schemas(conn.config, p.Database), func(cols []any) map[string]any {
		return map[string]any{"name": stringCell(cols, 0), "owner": stringCell(cols, 1)}
	})
	if err != nil {
		return s.err(req.ID, ErrSQLSyntax, err.Error())
	}
	return s.ok(req.ID, rows)
}

func (s *Server) handleSchemaObjects(ctx context.Context, req ipc.Message) ipc.Message {
	var p struct {
		ConnID   uint64   `json:"conn_id"`
		Database string   `json:"database,omitempty"`
		Schema   string   `json:"schema,omitempty"`
		Kinds    []string `json:"kinds,omitempty"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	conn, ok := s.conns[p.ConnID]
	if !ok {
		return s.err(req.ID, ErrUnknownConnID, fmt.Sprintf("unknown conn_id %d", p.ConnID))
	}
	if conn.schemaSQL.Objects == nil {
		return s.ok(req.ID, []map[string]any{})
	}
	rows, err := queryObjects(ctx, conn.db, conn.schemaSQL.Objects(conn.config, p.Database, p.Schema, p.Kinds), func(cols []any) map[string]any {
		return map[string]any{"name": stringCell(cols, 0), "kind": stringCell(cols, 1), "comment": stringCell(cols, 2)}
	})
	if err != nil {
		return s.err(req.ID, ErrSQLSyntax, err.Error())
	}
	return s.ok(req.ID, rows)
}

func (s *Server) handleSchemaColumns(ctx context.Context, req ipc.Message) ipc.Message {
	var p struct {
		ConnID   uint64 `json:"conn_id"`
		Database string `json:"database,omitempty"`
		Schema   string `json:"schema,omitempty"`
		Table    string `json:"table"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	conn, ok := s.conns[p.ConnID]
	if !ok {
		return s.err(req.ID, ErrUnknownConnID, fmt.Sprintf("unknown conn_id %d", p.ConnID))
	}
	if p.Table == "" {
		return s.err(req.ID, ErrInvalidParams, "missing required parameter `table`")
	}
	if conn.schemaSQL.Columns == nil {
		return s.ok(req.ID, []map[string]any{})
	}
	rows, err := queryObjects(ctx, conn.db, conn.schemaSQL.Columns(conn.config, p.Database, p.Schema, p.Table), func(cols []any) map[string]any {
		return map[string]any{
			"ordinal":    uint32(intCell(cols, 0)),
			"name":       stringCell(cols, 1),
			"type":       stringCell(cols, 2),
			"raw_type":   stringCell(cols, 2),
			"nullable":   boolCell(cols, 3),
			"default":    nullableString(cols, 4),
			"is_primary": false,
			"is_unique":  false,
		}
	})
	if err != nil {
		return s.err(req.ID, ErrSQLSyntax, err.Error())
	}
	return s.ok(req.ID, rows)
}

func (s *Server) handleSchemaIndexes(ctx context.Context, req ipc.Message) ipc.Message {
	var p struct {
		ConnID   uint64 `json:"conn_id"`
		Database string `json:"database,omitempty"`
		Schema   string `json:"schema,omitempty"`
		Table    string `json:"table"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	conn, ok := s.conns[p.ConnID]
	if !ok {
		return s.err(req.ID, ErrUnknownConnID, fmt.Sprintf("unknown conn_id %d", p.ConnID))
	}
	if p.Table == "" {
		return s.err(req.ID, ErrInvalidParams, "missing required parameter `table`")
	}
	if conn.schemaSQL.Indexes == nil {
		return s.ok(req.ID, []map[string]any{})
	}
	rows, err := queryObjects(ctx, conn.db, conn.schemaSQL.Indexes(conn.config, p.Database, p.Schema, p.Table), func(cols []any) map[string]any {
		kind := stringCell(cols, 4)
		return map[string]any{
			"name":       stringCell(cols, 0),
			"columns":    splitListCell(cols, 1),
			"is_unique":  boolCell(cols, 2),
			"is_primary": boolCell(cols, 3),
			"kind":       kind,
			"type":       kind,
		}
	})
	if err != nil {
		return s.err(req.ID, ErrSQLSyntax, err.Error())
	}
	return s.ok(req.ID, rows)
}

func (s *Server) handleSchemaForeignKeys(ctx context.Context, req ipc.Message) ipc.Message {
	var p struct {
		ConnID   uint64 `json:"conn_id"`
		Database string `json:"database,omitempty"`
		Schema   string `json:"schema,omitempty"`
		Table    string `json:"table"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	conn, ok := s.conns[p.ConnID]
	if !ok {
		return s.err(req.ID, ErrUnknownConnID, fmt.Sprintf("unknown conn_id %d", p.ConnID))
	}
	if p.Table == "" {
		return s.err(req.ID, ErrInvalidParams, "missing required parameter `table`")
	}
	if conn.schemaSQL.ForeignKeys == nil {
		return s.ok(req.ID, []map[string]any{})
	}
	rows, err := queryObjects(ctx, conn.db, conn.schemaSQL.ForeignKeys(conn.config, p.Database, p.Schema, p.Table), func(cols []any) map[string]any {
		return map[string]any{
			"name":               stringCell(cols, 0),
			"columns":            splitListCell(cols, 1),
			"referenced_schema":  stringCell(cols, 2),
			"referenced_table":   stringCell(cols, 3),
			"referenced_columns": splitListCell(cols, 4),
			"on_update":          stringCell(cols, 5),
			"on_delete":          stringCell(cols, 6),
		}
	})
	if err != nil {
		return s.err(req.ID, ErrSQLSyntax, err.Error())
	}
	return s.ok(req.ID, rows)
}

func (s *Server) handleSchemaViews(ctx context.Context, req ipc.Message) ipc.Message {
	var p struct {
		ConnID   uint64 `json:"conn_id"`
		Database string `json:"database,omitempty"`
		Schema   string `json:"schema,omitempty"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	conn, ok := s.conns[p.ConnID]
	if !ok {
		return s.err(req.ID, ErrUnknownConnID, fmt.Sprintf("unknown conn_id %d", p.ConnID))
	}
	if conn.schemaSQL.Views == nil {
		return s.ok(req.ID, []map[string]any{})
	}
	rows, err := queryObjects(ctx, conn.db, conn.schemaSQL.Views(conn.config, p.Database, p.Schema), func(cols []any) map[string]any {
		isMaterialized := boolCell(cols, 3)
		kind := "view"
		if isMaterialized {
			kind = "materialized_view"
		}
		return map[string]any{
			"name":            stringCell(cols, 0),
			"schema":          stringCell(cols, 1),
			"kind":            kind,
			"definition_sql":  stringCell(cols, 4),
			"comment":         stringCell(cols, 2),
			"is_materialized": isMaterialized,
		}
	})
	if err != nil {
		return s.err(req.ID, ErrSQLSyntax, err.Error())
	}
	return s.ok(req.ID, rows)
}

func (s *Server) handleSchemaFunctions(ctx context.Context, req ipc.Message) ipc.Message {
	var p struct {
		ConnID   uint64 `json:"conn_id"`
		Database string `json:"database,omitempty"`
		Schema   string `json:"schema,omitempty"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	conn, ok := s.conns[p.ConnID]
	if !ok {
		return s.err(req.ID, ErrUnknownConnID, fmt.Sprintf("unknown conn_id %d", p.ConnID))
	}
	if conn.schemaSQL.Functions == nil {
		return s.ok(req.ID, []map[string]any{})
	}
	rows, err := queryObjects(ctx, conn.db, conn.schemaSQL.Functions(conn.config, p.Database, p.Schema), func(cols []any) map[string]any {
		return map[string]any{
			"name":     stringCell(cols, 0),
			"schema":   stringCell(cols, 1),
			"returns":  stringCell(cols, 2),
			"language": stringCell(cols, 3),
			"comment":  stringCell(cols, 4),
		}
	})
	if err != nil {
		return s.err(req.ID, ErrSQLSyntax, err.Error())
	}
	return s.ok(req.ID, rows)
}

func (s *Server) handleSchemaViewDefinition(ctx context.Context, req ipc.Message) ipc.Message {
	var p struct {
		ConnID   uint64 `json:"conn_id"`
		Database string `json:"database,omitempty"`
		Schema   string `json:"schema,omitempty"`
		View     string `json:"view"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		return s.err(req.ID, ErrInvalidParams, err.Error())
	}
	conn, ok := s.conns[p.ConnID]
	if !ok {
		return s.err(req.ID, ErrUnknownConnID, fmt.Sprintf("unknown conn_id %d", p.ConnID))
	}
	if p.View == "" {
		return s.err(req.ID, ErrInvalidParams, "missing required parameter `view`")
	}
	if conn.schemaSQL.ViewDefinition == nil {
		return s.ok(req.ID, map[string]any{"sql": "", "is_materialized": false})
	}
	rows, err := queryObjects(ctx, conn.db, conn.schemaSQL.ViewDefinition(conn.config, p.Database, p.Schema, p.View), func(cols []any) map[string]any {
		return map[string]any{
			"sql":             stringCell(cols, 0),
			"is_materialized": boolCell(cols, 1),
		}
	})
	if err != nil {
		return s.err(req.ID, ErrSQLSyntax, err.Error())
	}
	if len(rows) == 0 {
		return s.ok(req.ID, map[string]any{"sql": "", "is_materialized": false})
	}
	var sqlText string
	isMaterialized := false
	for _, row := range rows {
		if part, ok := row["sql"].(string); ok {
			sqlText += part
		}
		if materialized, ok := row["is_materialized"].(bool); ok && materialized {
			isMaterialized = true
		}
	}
	return s.ok(req.ID, map[string]any{"sql": sqlText, "is_materialized": isMaterialized})
}

func (s *Server) handleEmptySchemaList(req ipc.Message) ipc.Message {
	if _, errResp := s.connFromParams(req); errResp != nil {
		return *errResp
	}
	return s.ok(req.ID, []map[string]any{})
}

func (s *Server) handleEmptyDumpDDL(req ipc.Message) ipc.Message {
	if _, errResp := s.connFromParams(req); errResp != nil {
		return *errResp
	}
	return s.ok(req.ID, map[string]any{"statements": []string{}})
}

func (s *Server) parseConfig(ctx context.Context, params json.RawMessage) (Config, ConnectionSpec, error) {
	var p struct {
		DriverID string         `json:"driver_id"`
		Config   map[string]any `json:"config"`
	}
	if err := decodeParams(params, &p); err != nil {
		return Config{}, ConnectionSpec{}, err
	}
	if p.DriverID != "" && p.DriverID != s.spec.ID {
		return Config{}, ConnectionSpec{}, fmt.Errorf("unsupported driver_id `%s`", p.DriverID)
	}
	cfg, err := ConfigFromWire(p.Config, s.spec.DefaultPort)
	if err != nil {
		return Config{}, ConnectionSpec{}, err
	}
	connSpec, err := s.resolveConnection(ctx, cfg)
	return cfg, connSpec, err
}

func (s *Server) resolveConnection(ctx context.Context, cfg Config) (ConnectionSpec, error) {
	if s.spec.ResolveConnection != nil {
		connSpec, err := s.spec.ResolveConnection(ctx, cfg)
		if err != nil {
			return ConnectionSpec{}, err
		}
		return s.normalizeConnectionSpec(connSpec)
	}
	dsn, err := s.spec.BuildDSN(cfg)
	if err != nil {
		return ConnectionSpec{}, err
	}
	return s.normalizeConnectionSpec(ConnectionSpec{
		DriverName: s.spec.SQLDriverName,
		DSN:        dsn,
		SchemaSQL:  s.spec.SchemaSQL,
	})
}

func (s *Server) normalizeConnectionSpec(connSpec ConnectionSpec) (ConnectionSpec, error) {
	if connSpec.DriverName == "" {
		connSpec.DriverName = s.spec.SQLDriverName
	}
	if schemaSQLIsEmpty(connSpec.SchemaSQL) {
		connSpec.SchemaSQL = s.spec.SchemaSQL
	}
	if connSpec.DriverName == "" {
		return ConnectionSpec{}, fmt.Errorf("missing SQL driver name")
	}
	return connSpec, nil
}

func schemaSQLIsEmpty(schemaSQL SchemaSQL) bool {
	return schemaSQL.Databases == nil &&
		schemaSQL.Schemas == nil &&
		schemaSQL.Objects == nil &&
		schemaSQL.Columns == nil &&
		schemaSQL.Indexes == nil &&
		schemaSQL.ForeignKeys == nil &&
		schemaSQL.Views == nil &&
		schemaSQL.Functions == nil &&
		schemaSQL.ViewDefinition == nil
}

func (s *Server) connFromParams(req ipc.Message) (*connectionState, *ipc.Message) {
	var p struct {
		ConnID uint64 `json:"conn_id"`
	}
	if err := decodeParams(req.Params, &p); err != nil {
		resp := s.err(req.ID, ErrInvalidParams, err.Error())
		return nil, &resp
	}
	conn, ok := s.conns[p.ConnID]
	if !ok {
		resp := s.err(req.ID, ErrUnknownConnID, fmt.Sprintf("unknown conn_id %d", p.ConnID))
		return nil, &resp
	}
	return conn, nil
}

func (s *Server) closeAll() {
	for id := range s.cursors {
		_ = s.closeCursor(id)
		delete(s.cursors, id)
	}
	for id, tx := range s.txs {
		_ = tx.tx.Rollback()
		delete(s.txs, id)
	}
	for id := range s.imports {
		delete(s.imports, id)
	}
	for id := range s.streams {
		delete(s.streams, id)
	}
	for id, conn := range s.conns {
		conn.db.Close()
		delete(s.conns, id)
	}
}

func (s *Server) closeCursorsForConn(connID uint64) {
	for cursorID, cursor := range s.cursors {
		if cursor.connID == connID {
			_ = s.closeCursor(cursorID)
			delete(s.cursors, cursorID)
		}
	}
}

func (s *Server) rollbackTxsForConn(connID uint64) {
	for txID, tx := range s.txs {
		if tx.connID == connID {
			_ = tx.tx.Rollback()
			delete(s.txs, txID)
		}
	}
}

func (s *Server) dropImportsForConn(connID uint64) {
	for importID, imp := range s.imports {
		if imp.connID == connID {
			delete(s.imports, importID)
		}
	}
}

func (s *Server) closeCursor(cursorID string) error {
	cursor, ok := s.cursors[cursorID]
	if !ok || cursor.rows == nil {
		if ok {
			cursor.done = true
		}
		return nil
	}
	err := cursor.rows.Close()
	cursor.rows = nil
	cursor.done = true
	return err
}

func (s *Server) ok(id json.RawMessage, result any) ipc.Message {
	raw, err := json.Marshal(result)
	if err != nil {
		return s.err(id, ErrInternalError, err.Error())
	}
	return ipc.Message{JSONRPC: ipc.JSONRPCVersion, ID: id, Result: raw}
}

func (s *Server) err(id json.RawMessage, code int32, message string) ipc.Message {
	return ipc.Message{
		JSONRPC: ipc.JSONRPCVersion,
		ID:      id,
		Error:   &ipc.ProtocolError{Code: code, Message: message},
	}
}

func decodeParams(raw json.RawMessage, out any) error {
	if len(raw) == 0 {
		raw = json.RawMessage(`{}`)
	}
	return json.Unmarshal(raw, out)
}
