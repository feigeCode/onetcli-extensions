package iotdb

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strconv"
	"strings"
	"sync"
	"time"

	iotdbclient "github.com/apache/iotdb-client-go/client"
	iotdbcommon "github.com/apache/iotdb-client-go/common"

	"navop-db-ipc-drivers/internal/dbipc"
	"navop-db-ipc-drivers/internal/ipc"
)

const (
	driverID      = "iotdb"
	driverName    = "Apache IoTDB"
	defaultPort   = 6667
	defaultUser   = "root"
	defaultPass   = "root"
	defaultDB     = "root"
	defaultTZ     = "UTC+8"
	defaultFetch  = 10000
	defaultSocket = "ONETCLI_EXT_SOCKET"
)

type Server struct {
	initialized bool
	nextConnID  uint64
	nextCursor  uint64
	conns       map[uint64]*connection
	cursors     map[string]*cursor
	mu          sync.Mutex
}

type connection struct {
	cfg     dbipc.Config
	session *iotdbclient.Session
}

type cursor struct {
	connID  uint64
	dataSet *iotdbclient.SessionDataSet
	maxRows *uint64
	fetched uint64
	done    bool
}

type columnSpec struct {
	Name     string `json:"name"`
	Type     string `json:"type"`
	TypeKind string `json:"type_kind"`
	Nullable *bool  `json:"nullable,omitempty"`
}

type cellValue map[string]any

func Run(args []string) error {
	socketName := ipc.SocketNameFromEnvOrArg(args)
	if socketName == "" {
		return fmt.Errorf("%s requires %s or a socket name argument", driverName, defaultSocket)
	}
	conn, err := ipc.DialHostSocket(socketName)
	if err != nil {
		return err
	}
	server := NewServer()
	return ipc.ServeConnected(conn, func(req ipc.Message) ipc.Message {
		return server.Handle(context.Background(), req)
	})
}

func NewServer() *Server {
	return &Server{
		nextConnID: 1,
		nextCursor: 1,
		conns:      map[uint64]*connection{},
		cursors:    map[string]*cursor{},
	}
}

func declaredMethods() []string {
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
		"schema/object_view",
		"schema/databases",
		"schema/schemas",
		"schema/objects",
		"schema/columns",
		"schema/views",
		"schema/indexes",
		"schema/checks",
		"schema/functions",
		"ddl/build",
		"ddl/build_create_table",
		"ddl/build_alter_table",
		"ddl/build_drop",
	}
}

func (s *Server) Handle(ctx context.Context, req ipc.Message) ipc.Message {
	s.mu.Lock()
	defer s.mu.Unlock()

	if req.JSONRPC != "" && req.JSONRPC != ipc.JSONRPCVersion {
		return errResp(req.ID, dbipc.ErrInvalidRequest, "jsonrpc must be 2.0")
	}
	if len(req.ID) == 0 {
		req.ID = json.RawMessage(`null`)
	}
	if req.Method != "init" && req.Method != "$/ping" && req.Method != "shutdown" && !s.initialized {
		return errResp(req.ID, dbipc.ErrNotInitialized, "init must be called first")
	}

	switch req.Method {
	case "init":
		if err := dbipc.ValidateHostVersion(req.Params); err != nil {
			return errRespFromError(req.ID, dbipc.ErrServerIncompatible, err)
		}
		s.initialized = true
		return okResp(req.ID, map[string]any{
			"extension_version": "0.1.3",
			"api_used":          map[string]string{"database": "1.0"},
			"features":          []string{"streaming", "schema_introspection", "iotdb_go_client"},
			"drivers_ready":     []string{driverID},
			"methods":           declaredMethods(),
			"name":              driverName + " IPC Driver",
		})
	case "$/ping":
		return okResp(req.ID, map[string]bool{"pong": true})
	case "shutdown":
		if err := s.closeAll(); err != nil {
			return errRespFromError(req.ID, dbipc.ErrConnectionFailed, err)
		}
		return okResp(req.ID, nil)
	case "conn/test":
		return s.handleConnTest(req)
	case "conn/open":
		return s.handleConnOpen(req)
	case "conn/close":
		return s.handleConnClose(req)
	case "conn/ping":
		return s.handleConnPing(req)
	case "conn/use":
		return s.handleConnUse(req)
	case "query/start":
		return s.handleQueryStart(req)
	case "cursor/fetch":
		return s.handleCursorFetch(req)
	case "cursor/close":
		return s.handleCursorClose(req)
	case "cursor/cancel":
		return s.handleCursorCancel(req)
	case "exec/run":
		return s.handleExecRun(req)
	case "exec/batch":
		return s.handleExecBatch(req)
	case "schema/object_view":
		return s.handleSchemaObjectView(req)
	case "schema/databases":
		return s.handleSchemaDatabases(req)
	case "schema/schemas":
		return s.handleSchemaSchemas(req)
	case "schema/objects":
		return s.handleSchemaObjects(req)
	case "schema/columns":
		return s.handleSchemaColumns(req)
	case "schema/views", "schema/indexes", "schema/checks":
		return s.handleEmptySchemaList(req)
	case "schema/functions":
		return s.handleSchemaFunctions(req)
	case "ddl/build":
		return s.handleDdlBuild(req)
	case "ddl/build_create_table":
		return s.handleDdlBuildCreateTable(req)
	case "ddl/build_alter_table":
		return s.handleDdlBuildAlterTable(req)
	case "ddl/build_drop":
		return s.handleDdlBuildDrop(req)
	default:
		return errResp(req.ID, dbipc.ErrMethodNotFound, fmt.Sprintf("method `%s` is not implemented", req.Method))
	}
}

func (s *Server) handleConnTest(req ipc.Message) ipc.Message {
	cfg, err := parseConfig(req.Params)
	if err != nil {
		return errRespFromError(req.ID, dbipc.ErrInvalidParams, err)
	}
	start := time.Now()
	session, err := openSession(cfg)
	if err != nil {
		return errRespFromError(req.ID, dbipc.ErrConnectionFailed, err)
	}
	version, versionErr := queryServerVersion(session)
	closeErr := closeSession(session)
	if err := joinCleanupError(versionErr, "close test connection", closeErr); err != nil {
		return errRespFromError(req.ID, dbipc.ErrConnectionFailed, err)
	}
	return okResp(req.ID, map[string]any{
		"ok":             true,
		"latency_ms":     uint32(time.Since(start).Milliseconds()),
		"warnings":       []string{},
		"server_version": version,
	})
}

func (s *Server) handleConnOpen(req ipc.Message) ipc.Message {
	cfg, err := parseConfig(req.Params)
	if err != nil {
		return errRespFromError(req.ID, dbipc.ErrInvalidParams, err)
	}
	session, err := openSession(cfg)
	if err != nil {
		return errRespFromError(req.ID, dbipc.ErrConnectionFailed, err)
	}
	version, versionErr := queryServerVersion(session)
	if versionErr != nil {
		versionErr = joinCleanupError(versionErr, "close connection after version query failure", closeSession(session))
		return errRespFromError(req.ID, dbipc.ErrConnectionFailed, versionErr)
	}
	connID := s.nextConnID
	s.nextConnID++
	s.conns[connID] = &connection{cfg: cfg, session: session}
	return okResp(req.ID, map[string]any{
		"conn_id": connID,
		"server_info": map[string]any{
			"version":  version,
			"features": []string{"iotdb_go_client"},
		},
	})
}

func (s *Server) handleConnClose(req ipc.Message) ipc.Message {
	var p struct {
		ConnID uint64 `json:"conn_id"`
	}
	if err := decode(req.Params, &p); err != nil {
		return errRespFromError(req.ID, dbipc.ErrInvalidParams, err)
	}
	conn, ok := s.conns[p.ConnID]
	if !ok {
		return errResp(req.ID, dbipc.ErrUnknownConnID, fmt.Sprintf("unknown conn_id %d", p.ConnID))
	}
	closeErr := s.closeCursorsForConn(p.ConnID)
	closeErr = joinCleanupError(closeErr, fmt.Sprintf("close connection %d", p.ConnID), closeSession(conn.session))
	delete(s.conns, p.ConnID)
	if closeErr != nil {
		return errRespFromError(req.ID, dbipc.ErrConnectionFailed, closeErr)
	}
	return okResp(req.ID, nil)
}

func (s *Server) handleConnPing(req ipc.Message) ipc.Message {
	conn, errMsg := s.connFromParams(req)
	if errMsg != nil {
		return *errMsg
	}
	start := time.Now()
	if _, err := conn.session.GetTimeZone(); err != nil {
		return errRespFromError(req.ID, dbipc.ErrConnectionFailed, err)
	}
	return okResp(req.ID, map[string]any{"latency_ms": uint32(time.Since(start).Milliseconds())})
}

func (s *Server) handleConnUse(req ipc.Message) ipc.Message {
	var p struct {
		ConnID   uint64 `json:"conn_id"`
		Database string `json:"database,omitempty"`
		Schema   string `json:"schema,omitempty"`
		Role     string `json:"role,omitempty"`
	}
	if err := decode(req.Params, &p); err != nil {
		return errRespFromError(req.ID, dbipc.ErrInvalidParams, err)
	}
	conn, ok := s.conns[p.ConnID]
	if !ok {
		return errResp(req.ID, dbipc.ErrUnknownConnID, fmt.Sprintf("unknown conn_id %d", p.ConnID))
	}
	if strings.TrimSpace(p.Database) != "" {
		conn.cfg.Database = strings.TrimSpace(p.Database)
	}
	if strings.TrimSpace(p.Schema) != "" {
		conn.cfg.Database = strings.TrimSpace(p.Schema)
	}
	return okResp(req.ID, nil)
}

func (s *Server) handleQueryStart(req ipc.Message) ipc.Message {
	var p struct {
		ConnID  uint64      `json:"conn_id"`
		SQL     string      `json:"sql"`
		Params  []cellValue `json:"params,omitempty"`
		MaxRows *uint64     `json:"max_rows,omitempty"`
	}
	if err := decode(req.Params, &p); err != nil {
		return errRespFromError(req.ID, dbipc.ErrInvalidParams, err)
	}
	if len(p.Params) > 0 {
		return errResp(req.ID, dbipc.ErrInvalidParams, "IoTDB driver does not support query parameters")
	}
	conn, ok := s.conns[p.ConnID]
	if !ok {
		return errResp(req.ID, dbipc.ErrUnknownConnID, fmt.Sprintf("unknown conn_id %d", p.ConnID))
	}
	sql := normalizeSQL(conn.storageGroup(), p.SQL)
	timeout := timeoutMs(conn.cfg)
	dataSet, err := conn.session.ExecuteQueryStatement(sql, &timeout)
	if err != nil {
		return errRespFromError(req.ID, dbipc.ErrSQLSyntax, err)
	}
	cursorID := fmt.Sprintf("%s-cursor-%d", driverID, s.nextCursor)
	s.nextCursor++
	s.cursors[cursorID] = &cursor{connID: p.ConnID, dataSet: dataSet, maxRows: p.MaxRows}
	return okResp(req.ID, map[string]any{
		"cursor_id":       cursorID,
		"columns":         columnsFromDataSet(dataSet),
		"row_count_known": false,
	})
}

func (s *Server) handleCursorFetch(req ipc.Message) ipc.Message {
	var p struct {
		CursorID string  `json:"cursor_id"`
		N        *uint32 `json:"n,omitempty"`
	}
	if err := decode(req.Params, &p); err != nil {
		return errRespFromError(req.ID, dbipc.ErrInvalidParams, err)
	}
	cur, ok := s.cursors[p.CursorID]
	if !ok {
		return errResp(req.ID, dbipc.ErrUnknownCursorID, fmt.Sprintf("unknown cursor_id `%s`", p.CursorID))
	}
	if cur.done || cur.dataSet == nil {
		return okResp(req.ID, map[string]any{"rows": [][]cellValue{}, "done": true})
	}
	n := 500
	if p.N != nil && *p.N > 0 {
		n = int(*p.N)
	}
	rows := make([][]cellValue, 0, n)
	for len(rows) < n {
		if cur.maxRows != nil && cur.fetched >= *cur.maxRows {
			cur.done = true
			break
		}
		next, err := nextDataSet(cur.dataSet)
		if err != nil {
			return errRespFromError(req.ID, dbipc.ErrSQLSyntax, err)
		}
		if !next {
			cur.done = true
			break
		}
		rows = append(rows, rowFromDataSet(cur.dataSet))
		cur.fetched++
	}
	if cur.done {
		if err := cur.dataSet.Close(); err != nil {
			if !isStatementIDNotSet(err) {
				return errRespFromError(req.ID, dbipc.ErrSQLSyntax, err)
			}
		}
		cur.dataSet = nil
	}
	return okResp(req.ID, map[string]any{"rows": rows, "done": cur.done})
}

func (s *Server) handleCursorClose(req ipc.Message) ipc.Message {
	var p struct {
		CursorID string `json:"cursor_id"`
	}
	if err := decode(req.Params, &p); err != nil {
		return errRespFromError(req.ID, dbipc.ErrInvalidParams, err)
	}
	if _, ok := s.cursors[p.CursorID]; !ok {
		return errResp(req.ID, dbipc.ErrUnknownCursorID, fmt.Sprintf("unknown cursor_id `%s`", p.CursorID))
	}
	if err := s.closeCursor(p.CursorID); err != nil {
		delete(s.cursors, p.CursorID)
		return errRespFromError(req.ID, dbipc.ErrSQLSyntax, err)
	}
	delete(s.cursors, p.CursorID)
	return okResp(req.ID, nil)
}

func (s *Server) handleCursorCancel(req ipc.Message) ipc.Message {
	var p struct {
		CursorID string `json:"cursor_id"`
	}
	if err := decode(req.Params, &p); err != nil {
		return errRespFromError(req.ID, dbipc.ErrInvalidParams, err)
	}
	cur, ok := s.cursors[p.CursorID]
	if !ok {
		return errResp(req.ID, dbipc.ErrUnknownCursorID, fmt.Sprintf("unknown cursor_id `%s`", p.CursorID))
	}
	if err := s.closeCursor(p.CursorID); err != nil {
		cur.done = true
		return errRespFromError(req.ID, dbipc.ErrSQLSyntax, err)
	}
	cur.done = true
	return okResp(req.ID, nil)
}

func (s *Server) handleExecRun(req ipc.Message) ipc.Message {
	var p struct {
		ConnID uint64      `json:"conn_id"`
		SQL    string      `json:"sql"`
		Params []cellValue `json:"params,omitempty"`
	}
	if err := decode(req.Params, &p); err != nil {
		return errRespFromError(req.ID, dbipc.ErrInvalidParams, err)
	}
	if len(p.Params) > 0 {
		return errResp(req.ID, dbipc.ErrInvalidParams, "IoTDB driver does not support exec parameters")
	}
	conn, ok := s.conns[p.ConnID]
	if !ok {
		return errResp(req.ID, dbipc.ErrUnknownConnID, fmt.Sprintf("unknown conn_id %d", p.ConnID))
	}
	if err := execStatement(conn, p.SQL); err != nil {
		return errRespFromError(req.ID, dbipc.ErrSQLSyntax, err)
	}
	return okResp(req.ID, map[string]any{"affected_rows": uint64(0), "warnings": []string{}})
}

func (s *Server) handleExecBatch(req ipc.Message) ipc.Message {
	var p struct {
		ConnID      uint64   `json:"conn_id"`
		Statements  []string `json:"statements"`
		StopOnError bool     `json:"stop_on_error"`
	}
	if err := decode(req.Params, &p); err != nil {
		return errRespFromError(req.ID, dbipc.ErrInvalidParams, err)
	}
	conn, ok := s.conns[p.ConnID]
	if !ok {
		return errResp(req.ID, dbipc.ErrUnknownConnID, fmt.Sprintf("unknown conn_id %d", p.ConnID))
	}
	results := make([]map[string]any, 0, len(p.Statements))
	errors := []map[string]any{}
	for idx, stmt := range p.Statements {
		if err := execStatement(conn, stmt); err != nil {
			protocolError := iotdbProtocolError(dbipc.ErrSQLSyntax, err)
			results = append(results, map[string]any{"affected_rows": uint64(0), "warnings": []string{}})
			errors = append(errors, map[string]any{
				"index":   uint32(idx),
				"code":    protocolError.Code,
				"message": protocolError.Message,
				"data":    json.RawMessage(protocolError.Data),
			})
			if p.StopOnError {
				break
			}
			continue
		}
		results = append(results, map[string]any{"affected_rows": uint64(0), "warnings": []string{}})
	}
	return okResp(req.ID, map[string]any{"results": results, "errors": errors})
}

func (s *Server) handleSchemaDatabases(req ipc.Message) ipc.Message {
	conn, errMsg := s.connFromParams(req)
	if errMsg != nil {
		return *errMsg
	}
	names, err := firstColumn(conn.session, "SHOW STORAGE GROUP")
	if err != nil {
		return errRespFromError(req.ID, dbipc.ErrSQLSyntax, err)
	}
	filter := conn.storageGroup()
	out := make([]map[string]any, 0, len(names))
	for _, name := range names {
		if pathMatchesFilter(name, filter) {
			out = append(out, map[string]any{
				"name":    name,
				"comment": "IoTDB storage group",
				"extra":   map[string]any{"kind": "storage_group"},
			})
		}
	}
	return okResp(req.ID, out)
}

func (s *Server) handleSchemaSchemas(req ipc.Message) ipc.Message {
	var p struct {
		ConnID   uint64 `json:"conn_id"`
		Database string `json:"database"`
	}
	if err := decode(req.Params, &p); err != nil {
		return errRespFromError(req.ID, dbipc.ErrInvalidParams, err)
	}
	if _, ok := s.conns[p.ConnID]; !ok {
		return errResp(req.ID, dbipc.ErrUnknownConnID, fmt.Sprintf("unknown conn_id %d", p.ConnID))
	}
	db := strings.TrimSpace(p.Database)
	if db == "" {
		db = defaultDB
	}
	return okResp(req.ID, []map[string]any{{
		"name":    db,
		"comment": "IoTDB storage group",
		"extra":   map[string]any{"kind": "storage_group"},
	}})
}

func (s *Server) handleSchemaObjects(req ipc.Message) ipc.Message {
	var p struct {
		ConnID   uint64   `json:"conn_id"`
		Database string   `json:"database,omitempty"`
		Schema   string   `json:"schema,omitempty"`
		Kinds    []string `json:"kinds,omitempty"`
	}
	if err := decode(req.Params, &p); err != nil {
		return errRespFromError(req.ID, dbipc.ErrInvalidParams, err)
	}
	conn, ok := s.conns[p.ConnID]
	if !ok {
		return errResp(req.ID, dbipc.ErrUnknownConnID, fmt.Sprintf("unknown conn_id %d", p.ConnID))
	}
	if !wantsKind(p.Kinds, "table") {
		return okResp(req.ID, []map[string]any{})
	}
	prefix := effectivePrefix(conn.storageGroup(), p.Database, p.Schema)
	names, err := firstColumn(conn.session, "SHOW DEVICES "+prefix+".**")
	if err != nil {
		return errRespFromError(req.ID, dbipc.ErrSQLSyntax, err)
	}
	out := make([]map[string]any, 0, len(names))
	for _, name := range names {
		out = append(out, map[string]any{
			"name":    relativePath(name, prefix),
			"kind":    "table",
			"comment": "IoTDB device",
			"extra":   map[string]any{"kind": "device"},
		})
	}
	return okResp(req.ID, out)
}

func (s *Server) handleSchemaColumns(req ipc.Message) ipc.Message {
	var p struct {
		ConnID   uint64 `json:"conn_id"`
		Database string `json:"database,omitempty"`
		Schema   string `json:"schema,omitempty"`
		Table    string `json:"table"`
	}
	if err := decode(req.Params, &p); err != nil {
		return errRespFromError(req.ID, dbipc.ErrInvalidParams, err)
	}
	conn, ok := s.conns[p.ConnID]
	if !ok {
		return errResp(req.ID, dbipc.ErrUnknownConnID, fmt.Sprintf("unknown conn_id %d", p.ConnID))
	}
	tablePath := qualifyPath(effectivePrefix(conn.storageGroup(), p.Database, p.Schema), p.Table)
	rows, err := queryRows(conn.session, "SHOW TIMESERIES "+tablePath+".*")
	if err != nil {
		return errRespFromError(req.ID, dbipc.ErrSQLSyntax, err)
	}
	out := []map[string]any{{
		"ordinal":    uint32(1),
		"name":       "Time",
		"type":       "int64",
		"raw_type":   "INT64",
		"nullable":   false,
		"is_primary": true,
		"extra":      map[string]any{"kind": "timestamp"},
	}}
	for _, row := range rows {
		path := row.textAtName("Timeseries")
		if path == "" {
			path = row.textAt(0)
		}
		if path == "" {
			continue
		}
		rawType := row.textAtName("DataType")
		if rawType == "" {
			rawType = row.textAtName("dataType")
		}
		if rawType == "" {
			rawType = "TEXT"
		}
		name := path[strings.LastIndex(path, ".")+1:]
		out = append(out, map[string]any{
			"ordinal":  uint32(len(out) + 1),
			"name":     name,
			"type":     hostTypeName(rawType),
			"raw_type": rawType,
			"nullable": true,
			"extra":    map[string]any{"timeseries": path},
		})
	}
	return okResp(req.ID, out)
}

func (s *Server) handleEmptySchemaList(req ipc.Message) ipc.Message {
	if _, errMsg := s.connFromParams(req); errMsg != nil {
		return *errMsg
	}
	return okResp(req.ID, []map[string]any{})
}

func (s *Server) handleSchemaFunctions(req ipc.Message) ipc.Message {
	if _, errMsg := s.connFromParams(req); errMsg != nil {
		return *errMsg
	}
	return okResp(req.ID, builtinFunctionRows())
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
	Columns []objectViewColumn `json:"columns,omitempty"`
	Rows    [][]string         `json:"rows,omitempty"`
}

type objectViewColumn struct {
	Key     string   `json:"key"`
	Name    string   `json:"name"`
	WidthPx *float64 `json:"width_px,omitempty"`
	Align   string   `json:"align,omitempty"`
}

func (s *Server) handleSchemaObjectView(req ipc.Message) ipc.Message {
	var p objectViewParams
	if err := decode(req.Params, &p); err != nil {
		return errRespFromError(req.ID, dbipc.ErrInvalidParams, err)
	}
	conn, ok := s.conns[p.ConnID]
	if !ok {
		return errResp(req.ID, dbipc.ErrUnknownConnID, fmt.Sprintf("unknown conn_id %d", p.ConnID))
	}

	view := strings.ToLower(strings.TrimSpace(p.View))
	switch view {
	case "databases":
		names, err := firstColumn(conn.session, "SHOW STORAGE GROUP")
		if err != nil {
			return errRespFromError(req.ID, dbipc.ErrSQLSyntax, err)
		}
		filter := conn.storageGroup()
		rows := make([][]string, 0, len(names))
		for _, name := range names {
			if pathMatchesFilter(name, filter) {
				rows = append(rows, []string{name, "IoTDB storage group"})
			}
		}
		return okResp(req.ID, objectViewResult("Storage Groups", objectViewColumns("name", "Name", "comment", "Comment"), rows))
	case "schemas":
		db := strings.TrimSpace(p.Database)
		if db == "" {
			db = conn.storageGroup()
		}
		return okResp(req.ID, objectViewResult("Storage Groups", objectViewColumns("name", "Name", "comment", "Comment"), [][]string{{db, "IoTDB storage group"}}))
	case "tables":
		prefix := effectivePrefix(conn.storageGroup(), p.Database, p.Schema)
		names, err := firstColumn(conn.session, "SHOW DEVICES "+prefix+".**")
		if err != nil {
			return errRespFromError(req.ID, dbipc.ErrSQLSyntax, err)
		}
		rows := make([][]string, 0, len(names))
		for _, name := range names {
			rows = append(rows, []string{relativePath(name, prefix), name, "IoTDB device"})
		}
		return okResp(req.ID, objectViewResult("Devices", objectViewColumns("name", "Name", "path", "Path", "comment", "Comment"), rows))
	case "columns":
		if strings.TrimSpace(p.Table) == "" {
			return errResp(req.ID, dbipc.ErrInvalidParams, "missing required parameter `table`")
		}
		tablePath := qualifyPath(effectivePrefix(conn.storageGroup(), p.Database, p.Schema), p.Table)
		rows, err := queryRows(conn.session, "SHOW TIMESERIES "+tablePath+".*")
		if err != nil {
			return errRespFromError(req.ID, dbipc.ErrSQLSyntax, err)
		}
		return okResp(req.ID, objectViewResult("Timeseries", timeseriesObjectViewColumns(), timeseriesObjectRows(rows)))
	case "views":
		return okResp(req.ID, objectViewResult("Views", objectViewColumns("name", "Name", "comment", "Comment"), [][]string{}))
	case "indexes":
		return okResp(req.ID, objectViewResult("Indexes", objectViewColumns("name", "Name", "table", "Table", "type", "Type"), [][]string{}))
	case "functions":
		rows := builtinFunctionRows()
		viewRows := make([][]string, 0, len(rows))
		for _, row := range rows {
			viewRows = append(viewRows, []string{
				stringValue(row["name"]),
				stringValue(row["return_type"]),
				stringValue(row["language"]),
				stringValue(row["comment"]),
			})
		}
		return okResp(req.ID, objectViewResult("Functions", objectViewColumns("name", "Name", "returns", "Returns", "language", "Language", "comment", "Comment"), viewRows))
	default:
		return errResp(req.ID, dbipc.ErrNotSupported, fmt.Sprintf("unsupported object view `%s`", p.View))
	}
}

func builtinFunctionRows() []map[string]any {
	out := []map[string]any{
		{"name": "COUNT", "return_type": "INT64", "language": "builtin", "comment": "IoTDB aggregate function"},
		{"name": "SUM", "return_type": "DOUBLE", "language": "builtin", "comment": "IoTDB aggregate function"},
		{"name": "AVG", "return_type": "DOUBLE", "language": "builtin", "comment": "IoTDB aggregate function"},
		{"name": "MIN_VALUE", "return_type": "DOUBLE", "language": "builtin", "comment": "IoTDB aggregate function"},
		{"name": "MAX_VALUE", "return_type": "DOUBLE", "language": "builtin", "comment": "IoTDB aggregate function"},
	}
	return out
}

func objectViewResult(title string, columns []objectViewColumn, rows [][]string) objectView {
	return objectView{Title: title, Columns: columns, Rows: rows}
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

func timeseriesObjectViewColumns() []objectViewColumn {
	return []objectViewColumn{
		objectViewColumnWithWidth("name", "Field", 220, ""),
		objectViewColumnWithWidth("type", "Type", 140, ""),
		objectViewColumnWithWidth("nullable", "Null?", 72, "right"),
		objectViewColumnWithWidth("path", "Timeseries", 280, ""),
	}
}

func objectViewColumnWithWidth(key, name string, width float64, align string) objectViewColumn {
	var widthPtr *float64
	if width > 0 {
		widthPtr = &width
	}
	return objectViewColumn{Key: key, Name: name, WidthPx: widthPtr, Align: align}
}

func timeseriesObjectRows(rows []queryRow) [][]string {
	out := [][]string{{"Time", "INT64", "false", ""}}
	for _, row := range rows {
		path := row.textAtName("Timeseries")
		if path == "" {
			path = row.textAt(0)
		}
		if path == "" {
			continue
		}
		rawType := row.textAtName("DataType")
		if rawType == "" {
			rawType = row.textAtName("dataType")
		}
		if rawType == "" {
			rawType = "TEXT"
		}
		out = append(out, []string{shortColumnName(path), rawType, "true", path})
	}
	return out
}

func stringValue(value any) string {
	if value == nil {
		return ""
	}
	return fmt.Sprint(value)
}

func (s *Server) handleDdlBuild(req ipc.Message) ipc.Message {
	var p struct {
		Op      string          `json:"op"`
		Payload json.RawMessage `json:"payload"`
	}
	if err := decode(req.Params, &p); err != nil {
		return errRespFromError(req.ID, dbipc.ErrInvalidParams, err)
	}
	switch p.Op {
	case "create_database":
		sql, err := buildCreateDatabaseFromRaw(p.Payload)
		if err != nil {
			return errRespFromError(req.ID, dbipc.ErrInvalidParams, err)
		}
		return okResp(req.ID, map[string]any{"statements": []string{sql}, "warnings": []string{}})
	case "drop_database":
		sql, err := buildDropDatabaseFromRaw(p.Payload)
		if err != nil {
			return errRespFromError(req.ID, dbipc.ErrInvalidParams, err)
		}
		return okResp(req.ID, map[string]any{"statements": []string{sql}, "warnings": []string{}})
	case "create_table":
		statements, warnings, err := buildCreateTableFromRaw(p.Payload, false)
		if err != nil {
			return errRespFromError(req.ID, dbipc.ErrInvalidParams, err)
		}
		return okResp(req.ID, map[string]any{"statements": statements, "warnings": warnings})
	case "drop_table":
		sql, err := buildDropFromRaw(p.Payload)
		if err != nil {
			return errRespFromError(req.ID, dbipc.ErrInvalidParams, err)
		}
		return okResp(req.ID, map[string]any{"statements": []string{sql}, "warnings": []string{}})
	default:
		return errResp(req.ID, dbipc.ErrInvalidParams, fmt.Sprintf("IoTDB DDL builder does not support op `%s`", p.Op))
	}
}

func (s *Server) handleDdlBuildCreateTable(req ipc.Message) ipc.Message {
	var p struct {
		Spec    tableSpec `json:"spec"`
		Options struct {
			IfNotExists bool `json:"if_not_exists"`
		} `json:"options"`
	}
	if err := decode(req.Params, &p); err != nil {
		return errRespFromError(req.ID, dbipc.ErrInvalidParams, err)
	}
	statements, _ := buildCreateTimeseriesStatements(p.Spec, p.Options.IfNotExists)
	return okResp(req.ID, map[string]any{"sql": strings.Join(statements, ";\n"), "statements": statements})
}

func (s *Server) handleDdlBuildAlterTable(req ipc.Message) ipc.Message {
	var p struct {
		FromSpec      tableSpec `json:"from_spec"`
		ToSpec        tableSpec `json:"to_spec"`
		ColumnRenames []any     `json:"column_renames"`
	}
	if err := decode(req.Params, &p); err != nil {
		return errRespFromError(req.ID, dbipc.ErrInvalidParams, err)
	}
	existing := map[string]bool{}
	for _, col := range p.FromSpec.Columns {
		existing[col.Name] = true
	}
	device := tableDevicePath(p.ToSpec)
	statements := []string{}
	for _, col := range p.ToSpec.Columns {
		if existing[col.Name] || strings.EqualFold(col.Name, "time") {
			continue
		}
		statements = append(statements, buildCreateTimeseries(device, col, false))
	}
	warnings := []string{}
	if len(p.ColumnRenames) > 0 {
		warnings = append(warnings, "IoTDB does not support renaming timeseries through this builder")
	}
	if len(p.FromSpec.Columns) > len(p.ToSpec.Columns) {
		warnings = append(warnings, "IoTDB destructive timeseries drops are not generated by alter builder")
	}
	return okResp(req.ID, map[string]any{"statements": statements, "rollback_statements": []string{}, "warnings": warnings})
}

func (s *Server) handleDdlBuildDrop(req ipc.Message) ipc.Message {
	sql, err := buildDropFromRaw(req.Params)
	if err != nil {
		return errRespFromError(req.ID, dbipc.ErrInvalidParams, err)
	}
	return okResp(req.ID, map[string]any{"sql": sql})
}

func (s *Server) connFromParams(req ipc.Message) (*connection, *ipc.Message) {
	var p struct {
		ConnID uint64 `json:"conn_id"`
	}
	if err := decode(req.Params, &p); err != nil {
		resp := errRespFromError(req.ID, dbipc.ErrInvalidParams, err)
		return nil, &resp
	}
	conn, ok := s.conns[p.ConnID]
	if !ok {
		resp := errResp(req.ID, dbipc.ErrUnknownConnID, fmt.Sprintf("unknown conn_id %d", p.ConnID))
		return nil, &resp
	}
	return conn, nil
}

func (s *Server) closeAll() error {
	var closeErr error
	for id := range s.cursors {
		closeErr = joinCleanupError(closeErr, fmt.Sprintf("close cursor %s", id), s.closeCursor(id))
		delete(s.cursors, id)
	}
	for id, conn := range s.conns {
		closeErr = joinCleanupError(closeErr, fmt.Sprintf("close connection %d", id), closeSession(conn.session))
		delete(s.conns, id)
	}
	return closeErr
}

func (s *Server) closeCursorsForConn(connID uint64) error {
	var closeErr error
	for id, cur := range s.cursors {
		if cur.connID == connID {
			closeErr = joinCleanupError(closeErr, fmt.Sprintf("close cursor %s", id), s.closeCursor(id))
			delete(s.cursors, id)
		}
	}
	return closeErr
}

func (s *Server) closeCursor(cursorID string) error {
	if cur, ok := s.cursors[cursorID]; ok && cur.dataSet != nil {
		err := cur.dataSet.Close()
		cur.dataSet = nil
		cur.done = true
		if err != nil && !isStatementIDNotSet(err) {
			return err
		}
	}
	return nil
}

func (c *connection) storageGroup() string {
	if db := strings.TrimSpace(c.cfg.Database); db != "" {
		return db
	}
	return defaultDB
}

func parseConfig(params json.RawMessage) (dbipc.Config, error) {
	var p struct {
		DriverID string         `json:"driver_id"`
		Config   map[string]any `json:"config"`
	}
	if err := decode(params, &p); err != nil {
		return dbipc.Config{}, err
	}
	if p.DriverID != "" && p.DriverID != driverID {
		return dbipc.Config{}, fmt.Errorf("unsupported driver_id `%s`", p.DriverID)
	}
	cfg, err := dbipc.ConfigFromWire(p.Config, defaultPort)
	if err != nil {
		return cfg, err
	}
	if cfg.Host == "" {
		cfg.Host = "127.0.0.1"
	}
	if cfg.Port == 0 {
		cfg.Port = defaultPort
	}
	if cfg.Username == "" {
		cfg.Username = defaultUser
	}
	if cfg.Password == "" {
		cfg.Password = defaultPass
	}
	if cfg.Database == "" {
		cfg.Database = defaultDB
	}
	return cfg, nil
}

func openSession(cfg dbipc.Config) (*iotdbclient.Session, error) {
	fetchSize := int32(defaultFetch)
	if value := strings.TrimSpace(cfg.Extra["fetch_size"]); value != "" {
		if parsed, err := strconv.Atoi(value); err == nil && parsed > 0 {
			fetchSize = int32(parsed)
		}
	}
	timeZone := cfg.Extra["time_zone"]
	if strings.TrimSpace(timeZone) == "" {
		timeZone = defaultTZ
	}
	sessionValue := iotdbclient.NewSession(&iotdbclient.Config{
		Host:      cfg.Host,
		Port:      strconv.Itoa(cfg.Port),
		UserName:  cfg.Username,
		Password:  cfg.Password,
		FetchSize: fetchSize,
		TimeZone:  timeZone,
	})
	session := &sessionValue
	if err := session.Open(boolExtra(cfg.Extra, "rpc_compression"), int(timeoutMs(cfg))); err != nil {
		return nil, err
	}
	return session, nil
}

func timeoutMs(cfg dbipc.Config) int64 {
	if value := strings.TrimSpace(cfg.Extra["timeout_ms"]); value != "" {
		if parsed, err := strconv.ParseInt(value, 10, 64); err == nil && parsed > 0 {
			return parsed
		}
	}
	return 30000
}

func boolExtra(extra map[string]string, key string) bool {
	value := strings.ToLower(strings.TrimSpace(extra[key]))
	return value == "1" || value == "true" || value == "yes"
}

func queryServerVersion(session *iotdbclient.Session) (string, error) {
	rows, err := queryRows(session, "SHOW VERSION")
	if err != nil || len(rows) == 0 {
		return driverName, err
	}
	if value := rows[0].textAt(0); value != "" {
		return value, nil
	}
	return driverName, nil
}

func execStatement(conn *connection, sql string) error {
	normalized := normalizeSQL(conn.storageGroup(), sql)
	status, err := conn.session.ExecuteNonQueryStatement(normalized)
	if err != nil {
		return err
	}
	return statusError(status)
}

func columnsFromDataSet(ds *iotdbclient.SessionDataSet) []columnSpec {
	columns := make([]columnSpec, 0, ds.GetColumnCount()+1)
	nullableFalse := false
	if !ds.IsIgnoreTimeStamp() {
		columns = append(columns, columnSpec{Name: "Time", Type: "INT64", TypeKind: "i64", Nullable: &nullableFalse})
	}
	for i, name := range ds.GetColumnNames() {
		typeName, kind := typeInfo(ds.GetColumnDataType(i))
		columns = append(columns, columnSpec{Name: shortColumnName(name), Type: typeName, TypeKind: kind})
	}
	return columns
}

func rowFromDataSet(ds *iotdbclient.SessionDataSet) []cellValue {
	row := make([]cellValue, 0, ds.GetColumnCount()+1)
	if !ds.IsIgnoreTimeStamp() {
		row = append(row, cellValue{"type": "i64", "value": ds.GetTimestamp()})
	}
	for _, name := range ds.GetColumnNames() {
		row = append(row, toCell(ds.GetValue(name)))
	}
	return row
}

type queryRow struct {
	columns []string
	values  []cellValue
}

func (r queryRow) textAt(idx int) string {
	if idx < 0 || idx >= len(r.values) {
		return ""
	}
	return cellText(r.values[idx])
}

func (r queryRow) textAtName(name string) string {
	for idx, column := range r.columns {
		if strings.EqualFold(column, name) || strings.Contains(strings.ToLower(column), strings.ToLower(name)) {
			return r.textAt(idx)
		}
	}
	return ""
}

func queryRows(session *iotdbclient.Session, sql string) (rows []queryRow, resultErr error) {
	timeout := int64(30000)
	ds, err := session.ExecuteQueryStatement(sql, &timeout)
	if err != nil {
		return nil, err
	}
	defer func() {
		if closeErr := ds.Close(); closeErr != nil && !isStatementIDNotSet(closeErr) {
			resultErr = joinCleanupError(resultErr, "close query dataset", closeErr)
		}
	}()
	columns := ds.GetColumnNames()
	rows = []queryRow{}
	for {
		next, err := nextDataSet(ds)
		if err != nil {
			return nil, err
		}
		if !next {
			break
		}
		values := make([]cellValue, 0, len(columns))
		for _, column := range columns {
			values = append(values, toCell(ds.GetValue(column)))
		}
		rows = append(rows, queryRow{columns: columns, values: values})
	}
	return rows, nil
}

func nextDataSet(ds *iotdbclient.SessionDataSet) (bool, error) {
	next, err := ds.Next()
	if err == nil {
		return next, nil
	}
	if isStatementIDNotSet(err) {
		return false, nil
	}
	return false, err
}

func isStatementIDNotSet(err error) bool {
	return err != nil && strings.Contains(strings.ToLower(err.Error()), "statement id not set by client")
}

func firstColumn(session *iotdbclient.Session, sql string) ([]string, error) {
	rows, err := queryRows(session, sql)
	if err != nil {
		return nil, err
	}
	out := make([]string, 0, len(rows))
	for _, row := range rows {
		if value := row.textAt(0); value != "" {
			out = append(out, value)
		}
	}
	return out, nil
}

func toCell(value any) cellValue {
	switch v := value.(type) {
	case nil:
		return cellValue{"type": "null"}
	case bool:
		return cellValue{"type": "bool", "value": v}
	case int:
		return cellValue{"type": "i64", "value": int64(v)}
	case int32:
		return cellValue{"type": "i64", "value": int64(v)}
	case int64:
		return cellValue{"type": "i64", "value": v}
	case float32:
		return cellValue{"type": "f64", "value": float64(v)}
	case float64:
		return cellValue{"type": "f64", "value": v}
	case []byte:
		return cellValue{"type": "text", "value": string(v)}
	case string:
		return cellValue{"type": "text", "value": v}
	case time.Time:
		return cellValue{"type": "date", "value": v.Format("2006-01-02")}
	default:
		return cellValue{"type": "text", "value": fmt.Sprint(v)}
	}
}

func cellText(cell cellValue) string {
	if cell == nil || cell["type"] == "null" {
		return ""
	}
	if value, ok := cell["value"]; ok && value != nil {
		return fmt.Sprint(value)
	}
	return ""
}

func typeInfo(t iotdbclient.TSDataType) (string, string) {
	switch t {
	case iotdbclient.BOOLEAN:
		return "BOOLEAN", "bool"
	case iotdbclient.INT32, iotdbclient.INT64, iotdbclient.TIMESTAMP:
		return "INT64", "i64"
	case iotdbclient.FLOAT, iotdbclient.DOUBLE:
		return "DOUBLE", "f64"
	case iotdbclient.TEXT, iotdbclient.STRING, iotdbclient.BLOB:
		return "TEXT", "text"
	case iotdbclient.DATE:
		return "DATE", "date"
	default:
		return "TEXT", "unknown"
	}
}

func hostTypeName(rawType string) string {
	switch strings.ToUpper(strings.TrimSpace(rawType)) {
	case "BOOLEAN", "BOOL":
		return "bool"
	case "INT32", "INT64", "INT", "INTEGER", "BIGINT", "LONG", "TIMESTAMP":
		return "int64"
	case "FLOAT", "DOUBLE", "REAL":
		return "float64"
	default:
		return "text"
	}
}

func pathMatchesFilter(path, filter string) bool {
	filter = strings.TrimSpace(filter)
	return filter == "" || filter == "root" || path == filter || strings.HasPrefix(path, filter+".")
}

func effectivePrefix(defaultPrefix, database, schema string) string {
	for _, value := range []string{schema, database, defaultPrefix} {
		if trimmed := strings.TrimSpace(value); trimmed != "" {
			return trimmed
		}
	}
	return defaultDB
}

func qualifyPath(prefix, path string) string {
	prefix = strings.TrimSpace(prefix)
	path = strings.TrimSpace(path)
	if strings.HasPrefix(path, "root.") || prefix == "" || path == prefix || strings.HasPrefix(path, prefix+".") {
		return path
	}
	return prefix + "." + path
}

func relativePath(path, prefix string) string {
	prefix = strings.TrimSpace(prefix)
	if prefix != "" {
		if rest, ok := strings.CutPrefix(path, prefix+"."); ok {
			return rest
		}
	}
	return path
}

func wantsKind(kinds []string, wanted string) bool {
	if len(kinds) == 0 {
		return true
	}
	for _, kind := range kinds {
		if strings.EqualFold(kind, wanted) {
			return true
		}
	}
	return false
}

func shortColumnName(column string) string {
	column = strings.Trim(column, "`\"")
	if strings.HasPrefix(column, "root.") {
		if idx := strings.LastIndex(column, "."); idx >= 0 {
			return column[idx+1:]
		}
	}
	return column
}

func okResp(id json.RawMessage, result any) ipc.Message {
	raw, err := json.Marshal(result)
	if err != nil {
		return errRespFromError(id, dbipc.ErrInternalError, err)
	}
	return ipc.Message{JSONRPC: ipc.JSONRPCVersion, ID: id, Result: raw}
}

func errResp(id json.RawMessage, code int32, message string) ipc.Message {
	return ipc.Message{
		JSONRPC: ipc.JSONRPCVersion,
		ID:      id,
		Error:   &ipc.ProtocolError{Code: code, Message: message},
	}
}

func errRespFromError(id json.RawMessage, code int32, err error) ipc.Message {
	return ipc.Message{
		JSONRPC: ipc.JSONRPCVersion,
		ID:      id,
		Error:   iotdbProtocolError(code, err),
	}
}

func iotdbProtocolError(code int32, err error) *ipc.ProtocolError {
	if err == nil {
		return &ipc.ProtocolError{Code: code}
	}

	message, data := iotdbErrorDetails(err)
	raw, marshalErr := json.Marshal(data)
	if marshalErr != nil {
		return &ipc.ProtocolError{Code: code, Message: message}
	}
	return &ipc.ProtocolError{
		Code:    code,
		Message: message,
		Data:    raw,
	}
}

func iotdbErrorDetails(err error) (string, ipc.ErrorData) {
	message := safeIoTDBErrorMessage(err)
	data := ipc.ErrorData{
		Extra: map[string]any{
			"chain":  safeIoTDBErrorChain(err),
			"source": message,
		},
	}

	var batchError *iotdbclient.BatchError
	if errors.As(err, &batchError) && batchError != nil {
		batchMessage, batchData := batchErrorDetails(batchError)
		batchData.Extra["chain"] = safeIoTDBErrorChain(err)
		batchData.Extra["source"] = batchMessage
		return batchMessage, batchData
	}

	var singleStatusError *iotdbStatusError
	if errors.As(err, &singleStatusError) && singleStatusError != nil {
		return statusErrorDetails(singleStatusError.status, data)
	}

	return message, data
}

func safeIoTDBErrorMessage(err error) string {
	if err == nil {
		return ""
	}
	var batchError *iotdbclient.BatchError
	if errors.As(err, &batchError) && batchError != nil {
		message, _ := batchErrorDetails(batchError)
		return message
	}
	var singleStatusError *iotdbStatusError
	if errors.As(err, &singleStatusError) && singleStatusError != nil {
		return formatIoTDBStatus(singleStatusError.status)
	}
	return err.Error()
}

func safeIoTDBErrorChain(err error) []string {
	chain := []string{}
	var visit func(error, int)
	visit = func(current error, depth int) {
		if current == nil || depth >= 64 {
			return
		}
		chain = append(chain, safeIoTDBErrorMessage(current))
		switch unwrapped := current.(type) {
		case interface{ Unwrap() []error }:
			for _, child := range unwrapped.Unwrap() {
				visit(child, depth+1)
			}
		case interface{ Unwrap() error }:
			visit(unwrapped.Unwrap(), depth+1)
		}
	}
	visit(err, 0)
	return chain
}

func batchErrorDetails(batch *iotdbclient.BatchError) (string, ipc.ErrorData) {
	data := ipc.ErrorData{Extra: map[string]any{}}
	if batch == nil {
		return "IoTDB batch error", data
	}

	statuses := batch.GetStatuses()
	entries := make([]map[string]any, 0, len(statuses))
	messages := make([]string, 0, len(statuses))
	var retryable bool
	var retryableSet bool
	for _, status := range statuses {
		entries = append(entries, iotdbStatusData(status))
		if status == nil || !iotdbStatusFailed(status) {
			continue
		}
		messages = append(messages, formatIoTDBStatus(status))
		if data.VendorCode == nil {
			vendorCode := int64(status.Code)
			data.VendorCode = &vendorCode
		}
		if status.NeedRetry != nil {
			retryableSet = true
			retryable = retryable || *status.NeedRetry
		}
	}
	if retryableSet {
		data.Retryable = &retryable
	}
	data.Extra["statuses"] = entries
	if len(messages) == 0 {
		return "IoTDB batch error", data
	}
	return strings.Join(messages, "; "), data
}

func statusErrorDetails(status *iotdbcommon.TSStatus, data ipc.ErrorData) (string, ipc.ErrorData) {
	message := formatIoTDBStatus(status)
	if status != nil {
		vendorCode := int64(status.Code)
		data.VendorCode = &vendorCode
		if status.NeedRetry != nil {
			retryable := *status.NeedRetry
			data.Retryable = &retryable
		}
		data.Extra["status"] = iotdbStatusData(status)
	}
	data.Extra["source"] = message
	return message, data
}

func iotdbStatusData(status *iotdbcommon.TSStatus) map[string]any {
	if status == nil {
		return map[string]any{"message": "IoTDB returned a nil status"}
	}
	entry := map[string]any{
		"code":    status.Code,
		"message": formatIoTDBStatus(status),
	}
	if status.NeedRetry != nil {
		entry["retryable"] = *status.NeedRetry
	}
	if len(status.SubStatus) > 0 {
		subStatuses := make([]map[string]any, 0, len(status.SubStatus))
		for _, child := range status.SubStatus {
			subStatuses = append(subStatuses, iotdbStatusData(child))
		}
		entry["sub_statuses"] = subStatuses
	}
	return entry
}

func formatIoTDBStatus(status *iotdbcommon.TSStatus) string {
	if status == nil {
		return "IoTDB returned a nil status"
	}
	if status.Message != nil && strings.TrimSpace(*status.Message) != "" {
		return fmt.Sprintf("IoTDB error code %d: %s", status.Code, *status.Message)
	}
	return fmt.Sprintf("IoTDB error code %d", status.Code)
}

func iotdbStatusFailed(status *iotdbcommon.TSStatus) bool {
	return status != nil &&
		status.Code != iotdbclient.SuccessStatus &&
		status.Code != iotdbclient.RedirectionRecommend
}

type iotdbStatusError struct {
	status *iotdbcommon.TSStatus
}

func (e *iotdbStatusError) Error() string {
	return formatIoTDBStatus(e.status)
}

func statusError(status *iotdbcommon.TSStatus) error {
	if status == nil || !iotdbStatusFailed(status) {
		return nil
	}
	if status.Code == iotdbclient.MultipleError {
		for _, child := range status.SubStatus {
			if iotdbStatusFailed(child) {
				return iotdbclient.NewBatchError(status.SubStatus)
			}
		}
		return nil
	}
	return &iotdbStatusError{status: status}
}

func closeSession(session *iotdbclient.Session) error {
	if session == nil {
		return nil
	}
	status, err := session.Close()
	if err != nil {
		return err
	}
	return statusError(status)
}

func joinCleanupError(existing error, operation string, err error) error {
	if err == nil {
		return existing
	}
	wrapped := fmt.Errorf("%s: %w", operation, err)
	if existing == nil {
		return wrapped
	}
	return errors.Join(existing, wrapped)
}

func decode(raw json.RawMessage, target any) error {
	if len(raw) == 0 || string(raw) == "null" {
		raw = json.RawMessage(`{}`)
	}
	return json.Unmarshal(raw, target)
}
