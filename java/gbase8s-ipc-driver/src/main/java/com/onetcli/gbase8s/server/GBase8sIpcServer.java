package com.onetcli.gbase8s.server;

import com.fasterxml.jackson.core.type.TypeReference;
import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import com.fasterxml.jackson.databind.node.ObjectNode;
import com.onetcli.gbase8s.db.JdbcQueryRunner;
import com.onetcli.gbase8s.db.QueryResult;
import com.onetcli.gbase8s.jdbc.GBase8sConfig;
import com.onetcli.gbase8s.schema.GBase8sSchemaSql;

import java.sql.Connection;
import java.sql.SQLException;
import java.sql.Savepoint;
import java.sql.Statement;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Base64;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

public final class GBase8sIpcServer {
    private static final String DRIVER_ID = "gbase8s";

    private final ObjectMapper mapper = new ObjectMapper();
    private final JdbcConnectionFactory connectionFactory;
    private final JdbcQueryRunner queryRunner = new JdbcQueryRunner();
    private final Map<Long, ConnectionState> connections = new LinkedHashMap<Long, ConnectionState>();
    private final Map<String, CursorState> cursors = new LinkedHashMap<String, CursorState>();
    private final Map<String, TxState> transactions = new LinkedHashMap<String, TxState>();
    private final Map<String, ImportState> imports = new LinkedHashMap<String, ImportState>();
    private final Map<String, StreamState> streams = new LinkedHashMap<String, StreamState>();
    private boolean initialized;
    private long nextConnId = 1L;
    private long nextCursorId = 1L;
    private long nextTxId = 1L;
    private long nextImportId = 1L;

    public GBase8sIpcServer(JdbcConnectionFactory connectionFactory) {
        this.connectionFactory = connectionFactory;
    }

    public synchronized JsonNode handle(JsonNode request) {
        JsonNode id = request == null ? JsonNodeFactory.instance.nullNode() : request.path("id");
        try {
            if (request == null || !request.isObject()) {
                return error(id, ProtocolError.INVALID_REQUEST, "request must be a JSON object");
            }
            JsonNode version = request.get("jsonrpc");
            if (version != null && !"2.0".equals(version.asText())) {
                return error(id, ProtocolError.INVALID_REQUEST, "jsonrpc must be 2.0");
            }
            String method = request.path("method").asText("");
            JsonNode params = request.path("params");
            if (requiresInit(method) && !initialized) {
                return error(id, ProtocolError.NOT_INITIALIZED, "init must be called first");
            }
            return dispatch(id, method, params);
        } catch (IllegalArgumentException error) {
            return error(id, ProtocolError.INVALID_PARAMS, error.getMessage());
        } catch (SQLException error) {
            return error(id, ProtocolError.SQL_SYNTAX, error.getMessage());
        } catch (Exception error) {
            return error(id, ProtocolError.INTERNAL_ERROR, error.getMessage());
        }
    }

    private JsonNode dispatch(JsonNode id, String method, JsonNode params) throws Exception {
        if ("init".equals(method)) {
            initialized = true;
            return ok(id, initResult());
        }
        if ("$/ping".equals(method)) {
            Map<String, Object> result = new LinkedHashMap<String, Object>();
            result.put("pong", Boolean.TRUE);
            return ok(id, result);
        }
        if ("shutdown".equals(method)) {
            closeAll();
            return ok(id, null);
        }
        if ("conn/test".equals(method)) {
            return handleConnTest(id, params);
        }
        if ("conn/open".equals(method)) {
            return handleConnOpen(id, params);
        }
        if ("conn/close".equals(method)) {
            return handleConnClose(id, params);
        }
        if ("conn/ping".equals(method)) {
            return handleConnPing(id, params);
        }
        if ("conn/use".equals(method)) {
            return handleConnUse(id, params);
        }
        if ("schema/databases".equals(method)) {
            return handleSchemaDatabases(id, params);
        }
        if ("schema/schemas".equals(method)) {
            return handleSchemaSchemas(id, params);
        }
        if ("schema/objects".equals(method)) {
            return handleSchemaObjects(id, params);
        }
        if ("schema/object_view".equals(method)) {
            return handleSchemaObjectView(id, params);
        }
        if ("schema/columns".equals(method)) {
            return handleSchemaColumns(id, params);
        }
        if ("schema/views".equals(method)) {
            return handleSchemaViews(id, params);
        }
        if ("query/start".equals(method)) {
            return handleQueryStart(id, params);
        }
        if ("cursor/fetch".equals(method)) {
            return handleCursorFetch(id, params);
        }
        if ("cursor/close".equals(method)) {
            return handleCursorClose(id, params);
        }
        if ("cursor/cancel".equals(method)) {
            return handleCursorCancel(id, params);
        }
        if ("exec/run".equals(method)) {
            return handleExecRun(id, params);
        }
        if ("exec/batch".equals(method)) {
            return handleExecBatch(id, params);
        }
        if ("tx/begin".equals(method)) {
            return handleTxBegin(id, params);
        }
        if ("tx/commit".equals(method)) {
            return handleTxCommit(id, params);
        }
        if ("tx/rollback".equals(method)) {
            return handleTxRollback(id, params);
        }
        if ("tx/savepoint".equals(method)) {
            return handleTxSavepoint(id, params);
        }
        if ("tx/release".equals(method)) {
            return handleTxRelease(id, params);
        }
        if ("ddl/build".equals(method)) {
            return handleDdlBuild(id, params);
        }
        if ("ddl/build_create_table".equals(method)) {
            return handleDdlBuildCreateTable(id, params);
        }
        if ("ddl/build_alter_table".equals(method)) {
            return handleDdlBuildAlterTable(id, params);
        }
        if ("ddl/build_drop".equals(method)) {
            return handleDdlBuildDrop(id, params);
        }
        if ("data/export".equals(method)) {
            return handleDataExport(id, params);
        }
        if ("data/import_begin".equals(method)) {
            return handleDataImportBegin(id, params);
        }
        if ("data/import_chunk".equals(method)) {
            return handleDataImportChunk(id, params);
        }
        if ("data/import_commit".equals(method)) {
            return handleDataImportCommit(id, params);
        }
        if ("data/import_abort".equals(method)) {
            return handleDataImportAbort(id, params);
        }
        if ("stream/read".equals(method)) {
            return handleStreamRead(id, params);
        }
        if ("stream/close".equals(method)) {
            return handleStreamClose(id, params);
        }
        if ("schema/indexes".equals(method) || "schema/foreign_keys".equals(method) || "schema/checks".equals(method)
            || "schema/functions".equals(method) || "schema/procedures".equals(method)
            || "schema/triggers".equals(method) || "schema/sequences".equals(method) || "schema/types".equals(method)) {
            return ok(id, new ArrayList<Map<String, Object>>());
        }
        if ("schema/view_definition".equals(method)) {
            Map<String, Object> result = new LinkedHashMap<String, Object>();
            result.put("sql", "");
            result.put("is_materialized", Boolean.FALSE);
            return ok(id, result);
        }
        if ("schema/dump_ddl".equals(method)) {
            Map<String, Object> result = new LinkedHashMap<String, Object>();
            result.put("statements", new ArrayList<String>());
            return ok(id, result);
        }
        return error(id, ProtocolError.METHOD_NOT_FOUND, "method `" + method + "` is not implemented");
    }

    private boolean requiresInit(String method) {
        return !"init".equals(method) && !"shutdown".equals(method) && !"$/ping".equals(method);
    }

    private Map<String, Object> initResult() {
        Map<String, Object> result = new LinkedHashMap<String, Object>();
        Map<String, String> api = new LinkedHashMap<String, String>();
        api.put("database", "1.0");
        List<String> features = new ArrayList<String>();
        features.add("streaming");
        features.add("schema_introspection");
        features.add("rich_errors");
        List<String> drivers = new ArrayList<String>();
        drivers.add(DRIVER_ID);
        List<String> methods = new ArrayList<String>();
        String[] methodNames = new String[]{
            "$/ping", "shutdown", "conn/test", "conn/open", "conn/close", "conn/ping", "conn/use",
            "query/start", "cursor/fetch", "cursor/close", "cursor/cancel", "exec/run", "exec/batch",
            "tx/begin", "tx/commit", "tx/rollback", "tx/savepoint", "tx/release",
            "ddl/build", "ddl/build_create_table", "ddl/build_alter_table", "ddl/build_drop",
            "data/export", "data/import_begin", "data/import_chunk", "data/import_commit", "data/import_abort",
            "stream/read", "stream/close", "schema/object_view", "schema/databases", "schema/schemas", "schema/objects",
            "schema/columns", "schema/indexes", "schema/foreign_keys", "schema/checks", "schema/views",
            "schema/functions", "schema/procedures", "schema/triggers", "schema/sequences", "schema/types",
            "schema/view_definition", "schema/dump_ddl"
        };
        for (String method : methodNames) {
            methods.add(method);
        }
        result.put("extension_version", "0.1.0");
        result.put("api_used", api);
        result.put("features", features);
        result.put("drivers_ready", drivers);
        result.put("methods", methods);
        result.put("name", "GBase 8s IPC Driver");
        return result;
    }

    private JsonNode handleConnTest(JsonNode id, JsonNode params) throws Exception {
        GBase8sConfig config = parseConfig(params);
        long start = System.currentTimeMillis();
        Connection connection = connectionFactory.open(config);
        try {
            connection.isValid(5);
            Map<String, Object> result = new LinkedHashMap<String, Object>();
            result.put("ok", Boolean.TRUE);
            result.put("server_version", "GBase 8s");
            result.put("warnings", new ArrayList<String>());
            result.put("latency_ms", Long.valueOf(System.currentTimeMillis() - start));
            return ok(id, result);
        } finally {
            connection.close();
        }
    }

    private JsonNode handleConnOpen(JsonNode id, JsonNode params) throws Exception {
        GBase8sConfig config = parseConfig(params);
        Connection connection = connectionFactory.open(config);
        connection.isValid(5);
        long connId = nextConnId++;
        connections.put(Long.valueOf(connId), new ConnectionState(config, connection));

        Map<String, Object> serverInfo = new LinkedHashMap<String, Object>();
        List<String> features = new ArrayList<String>();
        features.add("database_sql");
        serverInfo.put("version", "GBase 8s");
        serverInfo.put("features", features);

        Map<String, Object> result = new LinkedHashMap<String, Object>();
        result.put("conn_id", Long.valueOf(connId));
        result.put("server_info", serverInfo);
        return ok(id, result);
    }

    private JsonNode handleConnClose(JsonNode id, JsonNode params) throws SQLException {
        long connId = requiredLong(params, "conn_id");
        ConnectionState state = connections.remove(Long.valueOf(connId));
        if (state == null) {
            return error(id, ProtocolError.UNKNOWN_CONN_ID, "unknown conn_id " + connId);
        }
        closeTransactionsForConn(connId);
        closeImportsForConn(connId);
        state.connection.close();
        return ok(id, null);
    }

    private JsonNode handleConnPing(JsonNode id, JsonNode params) throws SQLException {
        ConnectionState state = requireConnection(id, requiredLong(params, "conn_id"));
        if (state == null) {
            return lastError;
        }
        long start = System.currentTimeMillis();
        state.connection.isValid(5);
        Map<String, Object> result = new LinkedHashMap<String, Object>();
        result.put("latency_ms", Long.valueOf(System.currentTimeMillis() - start));
        return ok(id, result);
    }

    private JsonNode handleConnUse(JsonNode id, JsonNode params) {
        long connId = requiredLong(params, "conn_id");
        if (!connections.containsKey(Long.valueOf(connId))) {
            return error(id, ProtocolError.UNKNOWN_CONN_ID, "unknown conn_id " + connId);
        }
        return ok(id, null);
    }

    private JsonNode handleSchemaDatabases(JsonNode id, JsonNode params) throws SQLException {
        ConnectionState state = requireConnection(id, requiredLong(params, "conn_id"));
        if (state == null) {
            return lastError;
        }
        QueryResult query = queryRunner.queryBuffered(state.connection, GBase8sSchemaSql.databasesSql(), null, null);
        List<Map<String, Object>> result = new ArrayList<Map<String, Object>>();
        for (List<Map<String, Object>> row : query.getRows()) {
            Map<String, Object> database = new LinkedHashMap<String, Object>();
            database.put("name", rowString(row, 0));
            database.put("charset", null);
            database.put("collation", null);
            database.put("comment", "");
            database.put("owner", null);
            database.put("size_bytes", null);
            database.put("extra", new LinkedHashMap<String, Object>());
            result.add(database);
        }
        return ok(id, result);
    }

    private JsonNode handleSchemaSchemas(JsonNode id, JsonNode params) throws SQLException {
        ConnectionState state = requireConnection(id, requiredLong(params, "conn_id"));
        if (state == null) {
            return lastError;
        }
        String database = optionalText(params, "database", state.config.getDatabase());
        QueryResult query = queryRunner.queryBuffered(state.connection, GBase8sSchemaSql.schemasSql(database), null, null);
        List<Map<String, Object>> result = new ArrayList<Map<String, Object>>();
        for (List<Map<String, Object>> row : query.getRows()) {
            Map<String, Object> schema = new LinkedHashMap<String, Object>();
            schema.put("name", rowString(row, 0));
            schema.put("owner", rowString(row, 1));
            schema.put("comment", "");
            result.add(schema);
        }
        return ok(id, result);
    }

    private JsonNode handleSchemaObjects(JsonNode id, JsonNode params) throws SQLException {
        ConnectionState state = requireConnection(id, requiredLong(params, "conn_id"));
        if (state == null) {
            return lastError;
        }
        String database = optionalText(params, "database", state.config.getDatabase());
        String schema = optionalText(params, "schema", "");
        List<String> kinds = readStringArray(params.path("kinds"));
        QueryResult query = queryRunner.queryBuffered(
            state.connection,
            GBase8sSchemaSql.objectsSql(database, schema, kinds),
            null,
            null
        );
        List<Map<String, Object>> result = new ArrayList<Map<String, Object>>();
        for (List<Map<String, Object>> row : query.getRows()) {
            String kind = rowString(row, 1);
            if (!kinds.isEmpty() && !kinds.contains(kind)) {
                continue;
            }
            Map<String, Object> object = new LinkedHashMap<String, Object>();
            object.put("name", rowString(row, 0));
            object.put("kind", kind);
            object.put("comment", rowString(row, 2));
            object.put("row_count_estimate", null);
            object.put("size_bytes", null);
            object.put("created_at", null);
            object.put("updated_at", null);
            object.put("extra", new LinkedHashMap<String, Object>());
            result.add(object);
        }
        return ok(id, result);
    }

    private JsonNode handleSchemaColumns(JsonNode id, JsonNode params) throws SQLException {
        ConnectionState state = requireConnection(id, requiredLong(params, "conn_id"));
        if (state == null) {
            return lastError;
        }
        String database = optionalText(params, "database", state.config.getDatabase());
        String schema = optionalText(params, "schema", "");
        String table = requiredText(params, "table");
        QueryResult query = queryRunner.queryBuffered(
            state.connection,
            GBase8sSchemaSql.columnsSql(database, schema, table),
            null,
            null
        );
        List<Map<String, Object>> result = new ArrayList<Map<String, Object>>();
        for (List<Map<String, Object>> row : query.getRows()) {
            String rawType = gbase8sColumnType(rowString(row, 2));
            Map<String, Object> column = new LinkedHashMap<String, Object>();
            column.put("ordinal", Integer.valueOf(rowInt(row, 0)));
            column.put("name", rowString(row, 1));
            column.put("type", rawType);
            column.put("raw_type", rawType);
            column.put("nullable", Boolean.valueOf(nullable(rowString(row, 3))));
            column.put("default", rowValue(row, 4));
            column.put("is_primary", Boolean.FALSE);
            column.put("is_unique", Boolean.FALSE);
            column.put("is_partition_key", Boolean.FALSE);
            column.put("is_clustering_key", Boolean.FALSE);
            column.put("max_length", null);
            column.put("precision", null);
            column.put("scale", null);
            column.put("comment", "");
            column.put("extra", new LinkedHashMap<String, Object>());
            result.add(column);
        }
        return ok(id, result);
    }

    private JsonNode handleSchemaViews(JsonNode id, JsonNode params) throws SQLException {
        ConnectionState state = requireConnection(id, requiredLong(params, "conn_id"));
        if (state == null) {
            return lastError;
        }
        String database = optionalText(params, "database", state.config.getDatabase());
        String schema = optionalText(params, "schema", "");
        QueryResult query = queryRunner.queryBuffered(
            state.connection,
            GBase8sSchemaSql.viewsSql(database, schema),
            null,
            null
        );
        List<Map<String, Object>> result = new ArrayList<Map<String, Object>>();
        for (List<Map<String, Object>> row : query.getRows()) {
            Map<String, Object> view = new LinkedHashMap<String, Object>();
            view.put("name", rowString(row, 0));
            view.put("kind", rowString(row, 1));
            view.put("definition_sql", rowString(row, 2));
            view.put("comment", "");
            view.put("extra", new LinkedHashMap<String, Object>());
            result.add(view);
        }
        return ok(id, result);
    }

    private JsonNode handleSchemaObjectView(JsonNode id, JsonNode params) throws SQLException {
        ConnectionState state = requireConnection(id, requiredLong(params, "conn_id"));
        if (state == null) {
            return lastError;
        }
        String view = requiredText(params, "view");
        String database = optionalText(params, "database", state.config.getDatabase());
        String schema = optionalText(params, "schema", "");
        if ("databases".equals(view)) {
            QueryResult query = queryRunner.queryBuffered(state.connection, GBase8sSchemaSql.databasesSql(), null, null);
            List<List<String>> rows = new ArrayList<List<String>>();
            for (List<Map<String, Object>> row : query.getRows()) {
                rows.add(rowValues(rowString(row, 0)));
            }
            return ok(id, objectView("Databases", objectViewColumns("name", "Name"), rows));
        }
        if ("schemas".equals(view)) {
            QueryResult query = queryRunner.queryBuffered(state.connection, GBase8sSchemaSql.schemasSql(database), null, null);
            List<List<String>> rows = new ArrayList<List<String>>();
            for (List<Map<String, Object>> row : query.getRows()) {
                rows.add(rowValues(rowString(row, 0), rowString(row, 1)));
            }
            return ok(id, objectView("Schemas", objectViewColumns("name", "Name", "owner", "Owner"), rows));
        }
        if ("tables".equals(view)) {
            QueryResult query = queryRunner.queryBuffered(
                state.connection,
                GBase8sSchemaSql.objectsSql(database, schema, java.util.Collections.singletonList("table")),
                null,
                null
            );
            List<List<String>> rows = new ArrayList<List<String>>();
            for (List<Map<String, Object>> row : query.getRows()) {
                rows.add(rowValues(rowString(row, 0), rowString(row, 1), rowString(row, 2)));
            }
            return ok(id, objectView("Tables", objectViewColumns("name", "Name", "kind", "Kind", "comment", "Comment"), rows));
        }
        if ("views".equals(view)) {
            QueryResult query = queryRunner.queryBuffered(state.connection, GBase8sSchemaSql.viewsSql(database, schema), null, null);
            List<List<String>> rows = new ArrayList<List<String>>();
            for (List<Map<String, Object>> row : query.getRows()) {
                rows.add(rowValues(rowString(row, 0), rowString(row, 1), ""));
            }
            return ok(id, objectView("Views", objectViewColumns("name", "Name", "kind", "Kind", "comment", "Comment"), rows));
        }
        if ("columns".equals(view)) {
            String table = requiredText(params, "table");
            QueryResult query = queryRunner.queryBuffered(
                state.connection,
                GBase8sSchemaSql.columnsSql(database, schema, table),
                null,
                null
            );
            List<List<String>> rows = new ArrayList<List<String>>();
            for (List<Map<String, Object>> row : query.getRows()) {
                rows.add(rowValues(
                    rowString(row, 1),
                    gbase8sColumnType(rowString(row, 2)),
                    Boolean.toString(nullable(rowString(row, 3))),
                    rowString(row, 4),
                    ""
                ));
            }
            return ok(id, objectView("Columns", columnObjectViewColumns(), rows));
        }
        if ("indexes".equals(view)) {
            return ok(id, objectView("Indexes", indexObjectViewColumns(), new ArrayList<List<String>>()));
        }
        if ("functions".equals(view)) {
            return ok(id, objectView("Functions", objectViewColumns("name", "Name", "returns", "Returns", "language", "Language", "comment", "Comment"), new ArrayList<List<String>>()));
        }
        if ("procedures".equals(view) || "triggers".equals(view) || "sequences".equals(view)) {
            return ok(id, objectView(titleForObjectView(view), objectViewColumns("name", "Name"), new ArrayList<List<String>>()));
        }
        return error(id, ProtocolError.NOT_SUPPORTED, "unsupported object view: " + view);
    }

    private Map<String, Object> objectView(String title, List<Map<String, Object>> columns, List<List<String>> rows) {
        Map<String, Object> result = new LinkedHashMap<String, Object>();
        result.put("title", title);
        result.put("columns", columns);
        result.put("rows", rows);
        return result;
    }

    private List<Map<String, Object>> columnObjectViewColumns() {
        List<Map<String, Object>> columns = new ArrayList<Map<String, Object>>();
        columns.add(objectViewColumn("name", "Field", 220, ""));
        columns.add(objectViewColumn("type", "Type", 160, ""));
        columns.add(objectViewColumn("nullable", "Null?", 72, "right"));
        columns.add(objectViewColumn("default", "Default", 180, ""));
        columns.add(objectViewColumn("comment", "Comment", 260, ""));
        return columns;
    }

    private List<Map<String, Object>> indexObjectViewColumns() {
        List<Map<String, Object>> columns = new ArrayList<Map<String, Object>>();
        columns.add(objectViewColumn("name", "Name", 220, ""));
        columns.add(objectViewColumn("columns", "Columns", 220, ""));
        columns.add(objectViewColumn("unique", "Unique?", 90, "right"));
        columns.add(objectViewColumn("primary", "Primary?", 90, "right"));
        columns.add(objectViewColumn("type", "Type", 140, ""));
        return columns;
    }

    private List<Map<String, Object>> objectViewColumns(String... values) {
        List<Map<String, Object>> columns = new ArrayList<Map<String, Object>>();
        for (int i = 0; i + 1 < values.length; i += 2) {
            int width = "name".equals(values[i]) ? 220 : 0;
            columns.add(objectViewColumn(values[i], values[i + 1], width, ""));
        }
        return columns;
    }

    private Map<String, Object> objectViewColumn(String key, String name, int width, String align) {
        Map<String, Object> column = new LinkedHashMap<String, Object>();
        column.put("key", key);
        column.put("name", name);
        if (width > 0) {
            column.put("width_px", Integer.valueOf(width));
        }
        if (align != null && align.length() > 0) {
            column.put("align", align);
        }
        return column;
    }

    private List<String> rowValues(String... values) {
        List<String> row = new ArrayList<String>();
        for (String value : values) {
            row.add(value == null ? "" : value);
        }
        return row;
    }

    private String titleForObjectView(String view) {
        if (view == null || view.length() == 0) {
            return "";
        }
        return view.substring(0, 1).toUpperCase() + view.substring(1);
    }

    private JsonNode handleQueryStart(JsonNode id, JsonNode params) throws SQLException {
        ConnectionState state = requireConnection(id, requiredLong(params, "conn_id"));
        if (state == null) {
            return lastError;
        }
        String txId = optionalText(params, "tx_id", "");
        if (!txId.isEmpty() && requireTransaction(id, txId, requiredLong(params, "conn_id")) == null) {
            return lastError;
        }
        String sql = requiredText(params, "sql");
        QueryResult query = queryRunner.queryBuffered(
            state.connection,
            sql,
            readParams(params),
            optionalInt(params, "max_rows")
        );
        String cursorId = DRIVER_ID + "-cursor-" + nextCursorId++;
        cursors.put(cursorId, new CursorState(query.getRows()));

        Map<String, Object> result = new LinkedHashMap<String, Object>();
        result.put("cursor_id", cursorId);
        result.put("columns", query.getColumns());
        result.put("row_count_known", Boolean.TRUE);
        result.put("row_count_estimate", Integer.valueOf(query.getRows().size()));
        return ok(id, result);
    }

    private JsonNode handleCursorFetch(JsonNode id, JsonNode params) {
        String cursorId = requiredText(params, "cursor_id");
        CursorState cursor = cursors.get(cursorId);
        if (cursor == null) {
            return error(id, ProtocolError.UNKNOWN_CURSOR_ID, "unknown cursor_id `" + cursorId + "`");
        }
        int n = optionalInt(params, "n") == null ? 500 : Math.max(0, optionalInt(params, "n").intValue());
        List<List<Map<String, Object>>> rows = cursor.take(n);
        Map<String, Object> result = new LinkedHashMap<String, Object>();
        result.put("rows", rows);
        result.put("done", Boolean.valueOf(cursor.isDone()));
        return ok(id, result);
    }

    private JsonNode handleCursorClose(JsonNode id, JsonNode params) {
        String cursorId = requiredText(params, "cursor_id");
        if (cursors.remove(cursorId) == null) {
            return error(id, ProtocolError.UNKNOWN_CURSOR_ID, "unknown cursor_id `" + cursorId + "`");
        }
        return ok(id, null);
    }

    private JsonNode handleCursorCancel(JsonNode id, JsonNode params) {
        String cursorId = requiredText(params, "cursor_id");
        if (cursors.remove(cursorId) == null) {
            return error(id, ProtocolError.UNKNOWN_CURSOR_ID, "unknown cursor_id `" + cursorId + "`");
        }
        return ok(id, null);
    }

    private JsonNode handleExecRun(JsonNode id, JsonNode params) throws SQLException {
        ConnectionState state = requireConnection(id, requiredLong(params, "conn_id"));
        if (state == null) {
            return lastError;
        }
        String txId = optionalText(params, "tx_id", "");
        if (!txId.isEmpty() && requireTransaction(id, txId, requiredLong(params, "conn_id")) == null) {
            return lastError;
        }
        long affected = queryRunner.execRun(state.connection, requiredText(params, "sql"), readParams(params));
        Map<String, Object> result = new LinkedHashMap<String, Object>();
        result.put("affected_rows", Long.valueOf(affected));
        result.put("warnings", new ArrayList<String>());
        return ok(id, result);
    }

    private JsonNode handleExecBatch(JsonNode id, JsonNode params) throws SQLException {
        ConnectionState state = requireConnection(id, requiredLong(params, "conn_id"));
        if (state == null) {
            return lastError;
        }
        List<String> statements = readStringArray(params.path("statements"));
        boolean stopOnError = !params.has("stop_on_error") || params.path("stop_on_error").asBoolean(true);
        boolean inTransaction = params.path("in_transaction").asBoolean(false);
        boolean originalAutoCommit = state.connection.getAutoCommit();
        if (inTransaction) {
            state.connection.setAutoCommit(false);
        }
        List<Map<String, Object>> results = new ArrayList<Map<String, Object>>();
        List<Map<String, Object>> errors = new ArrayList<Map<String, Object>>();
        try {
            for (int i = 0; i < statements.size(); i++) {
                try {
                    long affected = queryRunner.execRun(state.connection, statements.get(i), null);
                    Map<String, Object> result = new LinkedHashMap<String, Object>();
                    result.put("affected_rows", Long.valueOf(affected));
                    result.put("warnings", new ArrayList<String>());
                    results.add(result);
                } catch (SQLException error) {
                    Map<String, Object> item = new LinkedHashMap<String, Object>();
                    item.put("index", Integer.valueOf(i));
                    item.put("code", Integer.valueOf(ProtocolError.SQL_SYNTAX));
                    item.put("message", error.getMessage());
                    errors.add(item);
                    if (stopOnError) {
                        break;
                    }
                }
            }
            if (inTransaction) {
                if (errors.isEmpty()) {
                    state.connection.commit();
                } else {
                    state.connection.rollback();
                }
            }
        } finally {
            if (inTransaction) {
                state.connection.setAutoCommit(originalAutoCommit);
            }
        }
        Map<String, Object> out = new LinkedHashMap<String, Object>();
        out.put("results", results);
        out.put("errors", errors);
        return ok(id, out);
    }

    private JsonNode handleTxBegin(JsonNode id, JsonNode params) throws SQLException {
        long connId = requiredLong(params, "conn_id");
        ConnectionState state = requireConnection(id, connId);
        if (state == null) {
            return lastError;
        }
        if (state.activeTxId != null) {
            return error(id, ProtocolError.INVALID_PARAMS, "connection already has an active transaction");
        }
        state.originalAutoCommit = state.connection.getAutoCommit();
        state.connection.setAutoCommit(false);
        String txId = DRIVER_ID + "-tx-" + nextTxId++;
        state.activeTxId = txId;
        transactions.put(txId, new TxState(connId));
        Map<String, Object> result = new LinkedHashMap<String, Object>();
        result.put("tx_id", txId);
        return ok(id, result);
    }

    private JsonNode handleTxCommit(JsonNode id, JsonNode params) throws SQLException {
        TxState tx = requireTransaction(id, requiredText(params, "tx_id"), -1L);
        if (tx == null) {
            return lastError;
        }
        ConnectionState state = connections.get(Long.valueOf(tx.connId));
        state.connection.commit();
        finishTransaction(requiredText(params, "tx_id"), state);
        return ok(id, null);
    }

    private JsonNode handleTxRollback(JsonNode id, JsonNode params) throws SQLException {
        String txId = requiredText(params, "tx_id");
        TxState tx = requireTransaction(id, txId, -1L);
        if (tx == null) {
            return lastError;
        }
        ConnectionState state = connections.get(Long.valueOf(tx.connId));
        String savepoint = optionalText(params, "to_savepoint", "");
        if (!savepoint.isEmpty()) {
            Savepoint sp = tx.savepoints.get(savepoint);
            if (sp == null) {
                return error(id, ProtocolError.INVALID_PARAMS, "unknown savepoint `" + savepoint + "`");
            }
            state.connection.rollback(sp);
            return ok(id, null);
        }
        state.connection.rollback();
        finishTransaction(txId, state);
        return ok(id, null);
    }

    private JsonNode handleTxSavepoint(JsonNode id, JsonNode params) throws SQLException {
        String txId = requiredText(params, "tx_id");
        TxState tx = requireTransaction(id, txId, -1L);
        if (tx == null) {
            return lastError;
        }
        String name = requiredText(params, "name");
        ConnectionState state = connections.get(Long.valueOf(tx.connId));
        tx.savepoints.put(name, state.connection.setSavepoint(name));
        return ok(id, null);
    }

    private JsonNode handleTxRelease(JsonNode id, JsonNode params) throws SQLException {
        String txId = requiredText(params, "tx_id");
        TxState tx = requireTransaction(id, txId, -1L);
        if (tx == null) {
            return lastError;
        }
        String name = requiredText(params, "name");
        Savepoint sp = tx.savepoints.remove(name);
        if (sp == null) {
            return error(id, ProtocolError.INVALID_PARAMS, "unknown savepoint `" + name + "`");
        }
        connections.get(Long.valueOf(tx.connId)).connection.releaseSavepoint(sp);
        return ok(id, null);
    }

    private JsonNode handleDdlBuild(JsonNode id, JsonNode params) {
        String op = requiredText(params, "op");
        JsonNode payload = params.path("payload");
        if ("create_table".equals(op)) {
            return handleDdlBuildCreateTable(id, payload);
        }
        if ("drop_table".equals(op) || "drop_view".equals(op)) {
            return handleDdlBuildDrop(id, payload);
        }
        return error(id, ProtocolError.INVALID_PARAMS, "ddl op `" + op + "` is not supported");
    }

    private JsonNode handleDdlBuildCreateTable(JsonNode id, JsonNode params) {
        JsonNode spec = params.path("spec");
        String table = requiredText(spec, "name");
        String schema = optionalText(spec, "schema", "");
        List<String> defs = new ArrayList<String>();
        List<String> primary = new ArrayList<String>();
        for (JsonNode col : spec.path("columns")) {
            String name = requiredText(col, "name");
            String type = requiredText(col, "type");
            StringBuilder def = new StringBuilder();
            def.append(qualifiedIdentifier("", "", name)).append(' ').append(type);
            if (col.has("nullable") && !col.path("nullable").asBoolean(true)) {
                def.append(" NOT NULL");
            }
            if (col.has("default") && !col.path("default").isNull()) {
                def.append(" DEFAULT ").append(col.path("default").asText());
            }
            if (col.path("is_primary").asBoolean(false)) {
                primary.add(name);
            }
            defs.add(def.toString());
        }
        JsonNode pk = spec.path("primary_key");
        if (pk.isArray()) {
            primary.clear();
            for (JsonNode item : pk) {
                primary.add(item.asText());
            }
        }
        if (!primary.isEmpty()) {
            defs.add("PRIMARY KEY (" + quoteList(primary) + ")");
        }
        StringBuilder sql = new StringBuilder("CREATE TABLE ");
        if (params.path("options").path("if_not_exists").asBoolean(false)) {
            sql.append("IF NOT EXISTS ");
        }
        sql.append(qualifiedIdentifier("", schema, table)).append(" (").append(join(defs, ", ")).append(")");
        List<String> statements = new ArrayList<String>();
        statements.add(sql.toString());
        Map<String, Object> result = new LinkedHashMap<String, Object>();
        result.put("sql", sql.toString());
        result.put("statements", statements);
        return ok(id, result);
    }

    private JsonNode handleDdlBuildAlterTable(JsonNode id, JsonNode params) {
        JsonNode toSpec = params.path("to_spec");
        String table = requiredText(toSpec, "name");
        String schema = optionalText(toSpec, "schema", "");
        List<String> statements = new ArrayList<String>();
        for (JsonNode rename : params.path("column_renames")) {
            statements.add("ALTER TABLE " + qualifiedIdentifier("", schema, table) + " RENAME COLUMN "
                + quote(rename.path("old_name").asText()) + " TO " + quote(rename.path("new_name").asText()));
        }
        Map<String, Object> result = new LinkedHashMap<String, Object>();
        result.put("statements", statements);
        result.put("rollback_statements", new ArrayList<String>());
        result.put("warnings", new ArrayList<String>());
        return ok(id, result);
    }

    private JsonNode handleDdlBuildDrop(JsonNode id, JsonNode params) {
        String kind = optionalText(params, "kind", "table").toUpperCase().replace('_', ' ');
        StringBuilder sql = new StringBuilder("DROP ").append(kind);
        if (params.path("if_exists").asBoolean(false)) {
            sql.append(" IF EXISTS");
        }
        sql.append(' ').append(qualifiedIdentifier(optionalText(params, "database", ""), optionalText(params, "schema", ""), requiredText(params, "name")));
        if (params.path("cascade").asBoolean(false)) {
            sql.append(" CASCADE");
        }
        Map<String, Object> result = new LinkedHashMap<String, Object>();
        result.put("sql", sql.toString());
        return ok(id, result);
    }

    private JsonNode handleDataExport(JsonNode id, JsonNode params) throws SQLException {
        ConnectionState state = requireConnection(id, requiredLong(params, "conn_id"));
        if (state == null) {
            return lastError;
        }
        String streamId = requiredText(params, "stream_id");
        String sql = optionalText(params, "sql", "");
        if (sql.isEmpty()) {
            sql = "SELECT * FROM " + qualifiedIdentifier(optionalText(params, "database", ""), optionalText(params, "schema", ""), requiredText(params, "table"));
        }
        String format = requiredText(params, "format");
        QueryResult query = queryRunner.queryBuffered(state.connection, sql, readParams(params), optionalInt(params, "max_rows"));
        byte[] data = exportBytes(format, query);
        streams.put(streamId, new StreamState(data));
        Map<String, Object> result = new LinkedHashMap<String, Object>();
        result.put("estimated_bytes", Long.valueOf(data.length));
        result.put("estimated_rows", Long.valueOf(query.getRows().size()));
        Map<String, Object> metadata = new LinkedHashMap<String, Object>();
        metadata.put("format", format);
        metadata.put("columns", query.getColumns());
        result.put("metadata", metadata);
        return ok(id, result);
    }

    private JsonNode handleDataImportBegin(JsonNode id, JsonNode params) {
        long connId = requiredLong(params, "conn_id");
        if (!connections.containsKey(Long.valueOf(connId))) {
            return error(id, ProtocolError.UNKNOWN_CONN_ID, "unknown conn_id " + connId);
        }
        String format = requiredText(params, "format");
        if (!"json".equals(format) && !"ndjson".equals(format) && !"csv".equals(format)) {
            return error(id, ProtocolError.INVALID_PARAMS, "import format `" + format + "` is not supported");
        }
        String importId = DRIVER_ID + "-import-" + nextImportId++;
        imports.put(importId, new ImportState(connId, optionalText(params, "database", ""), optionalText(params, "schema", ""), requiredText(params, "table"), readStringArray(params.path("columns"))));
        Map<String, Object> result = new LinkedHashMap<String, Object>();
        result.put("import_id", importId);
        return ok(id, result);
    }

    private JsonNode handleDataImportChunk(JsonNode id, JsonNode params) throws SQLException {
        String importId = requiredText(params, "import_id");
        ImportState state = imports.get(importId);
        if (state == null) {
            return error(id, ProtocolError.INVALID_PARAMS, "unknown import_id `" + importId + "`");
        }
        ConnectionState conn = connections.get(Long.valueOf(state.connId));
        String sql = insertSql(state);
        long inserted = 0L;
        for (JsonNode row : params.path("rows")) {
            List<Map<String, Object>> cells = mapper.convertValue(row, new TypeReference<List<Map<String, Object>>>() {});
            inserted += queryRunner.execRun(conn.connection, sql, cells);
        }
        state.inserted += inserted;
        Map<String, Object> result = new LinkedHashMap<String, Object>();
        result.put("inserted", Long.valueOf(inserted));
        result.put("failed", new ArrayList<Map<String, Object>>());
        return ok(id, result);
    }

    private JsonNode handleDataImportCommit(JsonNode id, JsonNode params) {
        String importId = requiredText(params, "import_id");
        ImportState state = imports.remove(importId);
        if (state == null) {
            return error(id, ProtocolError.INVALID_PARAMS, "unknown import_id `" + importId + "`");
        }
        Map<String, Object> result = new LinkedHashMap<String, Object>();
        result.put("inserted", Long.valueOf(state.inserted));
        result.put("updated", Long.valueOf(0L));
        result.put("deleted", Long.valueOf(0L));
        result.put("failed", new ArrayList<Map<String, Object>>());
        result.put("elapsed_ms", Long.valueOf(System.currentTimeMillis() - state.startedAt));
        return ok(id, result);
    }

    private JsonNode handleDataImportAbort(JsonNode id, JsonNode params) {
        imports.remove(requiredText(params, "import_id"));
        return ok(id, null);
    }

    private JsonNode handleStreamRead(JsonNode id, JsonNode params) {
        String streamId = requiredText(params, "stream_id");
        StreamState stream = streams.get(streamId);
        if (stream == null) {
            return error(id, ProtocolError.INVALID_PARAMS, "unknown stream_id `" + streamId + "`");
        }
        int max = optionalInt(params, "max_bytes") == null ? 65536 : Math.max(0, optionalInt(params, "max_bytes").intValue());
        int end = Math.min(stream.data.length, stream.offset + max);
        byte[] chunk = new byte[end - stream.offset];
        System.arraycopy(stream.data, stream.offset, chunk, 0, chunk.length);
        stream.offset = end;
        boolean done = stream.offset >= stream.data.length;
        if (done) {
            streams.remove(streamId);
        }
        Map<String, Object> result = new LinkedHashMap<String, Object>();
        result.put("data", Base64.getEncoder().encodeToString(chunk));
        result.put("done", Boolean.valueOf(done));
        return ok(id, result);
    }

    private JsonNode handleStreamClose(JsonNode id, JsonNode params) {
        streams.remove(requiredText(params, "stream_id"));
        return ok(id, null);
    }

    private GBase8sConfig parseConfig(JsonNode params) {
        String driverId = textOrEmpty(params.path("driver_id"));
        if (!driverId.isEmpty() && !DRIVER_ID.equals(driverId)) {
            throw new IllegalArgumentException("unsupported driver_id `" + driverId + "`");
        }
        Map<String, Object> raw = mapper.convertValue(
            params.path("config"),
            new TypeReference<Map<String, Object>>() {
            }
        );
        return GBase8sConfig.fromWire(raw);
    }

    private List<Map<String, Object>> readParams(JsonNode params) {
        JsonNode node = params.path("params");
        if (!node.isArray()) {
            return null;
        }
        return mapper.convertValue(
            node,
            new TypeReference<List<Map<String, Object>>>() {
            }
        );
    }

    private JsonNode lastError;

    private ConnectionState requireConnection(JsonNode id, long connId) {
        ConnectionState state = connections.get(Long.valueOf(connId));
        if (state == null) {
            lastError = error(id, ProtocolError.UNKNOWN_CONN_ID, "unknown conn_id " + connId);
            return null;
        }
        lastError = null;
        return state;
    }

    private TxState requireTransaction(JsonNode id, String txId, long expectedConnId) {
        TxState tx = transactions.get(txId);
        if (tx == null) {
            lastError = error(id, ProtocolError.INVALID_PARAMS, "unknown tx_id `" + txId + "`");
            return null;
        }
        if (expectedConnId >= 0 && tx.connId != expectedConnId) {
            lastError = error(id, ProtocolError.INVALID_PARAMS, "tx_id `" + txId + "` does not belong to conn_id " + expectedConnId);
            return null;
        }
        lastError = null;
        return tx;
    }

    private void finishTransaction(String txId, ConnectionState state) throws SQLException {
        transactions.remove(txId);
        state.connection.setAutoCommit(state.originalAutoCommit);
        state.activeTxId = null;
    }

    private static String quote(String name) {
        return "\"" + name.replace("\"", "\"\"") + "\"";
    }

    private static String qualifiedIdentifier(String database, String schema, String name) {
        List<String> parts = new ArrayList<String>();
        if (database != null && !database.trim().isEmpty()) {
            parts.add(quote(database));
        }
        if (schema != null && !schema.trim().isEmpty()) {
            parts.add(quote(schema));
        }
        parts.add(quote(name));
        return join(parts, ".");
    }

    private static String quoteList(List<String> names) {
        List<String> quoted = new ArrayList<String>();
        for (String name : names) {
            if (name != null && !name.trim().isEmpty()) {
                quoted.add(quote(name));
            }
        }
        return join(quoted, ", ");
    }

    private static String join(List<String> values, String separator) {
        StringBuilder out = new StringBuilder();
        for (int i = 0; i < values.size(); i++) {
            if (i > 0) {
                out.append(separator);
            }
            out.append(values.get(i));
        }
        return out.toString();
    }

    private byte[] exportBytes(String format, QueryResult query) throws SQLException {
        if ("json".equals(format)) {
            try {
                List<Map<String, Object>> rows = rowsAsObjects(query);
                return mapper.writeValueAsBytes(rows);
            } catch (Exception error) {
                throw new SQLException(error);
            }
        }
        if ("ndjson".equals(format)) {
            try {
                StringBuilder out = new StringBuilder();
                for (Map<String, Object> row : rowsAsObjects(query)) {
                    out.append(mapper.writeValueAsString(row)).append('\n');
                }
                return out.toString().getBytes(StandardCharsets.UTF_8);
            } catch (Exception error) {
                throw new SQLException(error);
            }
        }
        if ("csv".equals(format)) {
            StringBuilder out = new StringBuilder();
            List<String> names = columnNames(query);
            out.append(join(names, ",")).append('\n');
            for (Map<String, Object> row : rowsAsObjects(query)) {
                List<String> cells = new ArrayList<String>();
                for (String name : names) {
                    Object value = row.get(name);
                    cells.add(value == null ? "" : String.valueOf(value).replace("\"", "\"\""));
                }
                out.append(join(cells, ",")).append('\n');
            }
            return out.toString().getBytes(StandardCharsets.UTF_8);
        }
        throw new SQLException("export format `" + format + "` is not supported");
    }

    private List<Map<String, Object>> rowsAsObjects(QueryResult query) {
        List<String> names = columnNames(query);
        List<Map<String, Object>> rows = new ArrayList<Map<String, Object>>();
        for (List<Map<String, Object>> row : query.getRows()) {
            Map<String, Object> object = new LinkedHashMap<String, Object>();
            for (int i = 0; i < names.size(); i++) {
                object.put(names.get(i), rowValue(row, i));
            }
            rows.add(object);
        }
        return rows;
    }

    private List<String> columnNames(QueryResult query) {
        List<String> names = new ArrayList<String>();
        for (Map<String, Object> column : query.getColumns()) {
            names.add(String.valueOf(column.get("name")));
        }
        return names;
    }

    private String insertSql(ImportState state) {
        if (state.columns.isEmpty()) {
            throw new IllegalArgumentException("data import requires explicit columns");
        }
        List<String> placeholders = new ArrayList<String>();
        for (int i = 0; i < state.columns.size(); i++) {
            placeholders.add("?");
        }
        return "INSERT INTO " + qualifiedIdentifier(state.database, state.schema, state.table) + " (" + quoteList(state.columns) + ") VALUES (" + join(placeholders, ", ") + ")";
    }

    private long requiredLong(JsonNode params, String field) {
        JsonNode value = params.get(field);
        if (value == null || !value.canConvertToLong()) {
            throw new IllegalArgumentException("missing required parameter `" + field + "`");
        }
        return value.asLong();
    }

    private String requiredText(JsonNode params, String field) {
        String value = textOrEmpty(params.get(field));
        if (value.isEmpty()) {
            throw new IllegalArgumentException("missing required parameter `" + field + "`");
        }
        return value;
    }

    private Integer optionalInt(JsonNode params, String field) {
        JsonNode value = params.get(field);
        if (value == null || value.isNull()) {
            return null;
        }
        return Integer.valueOf(value.asInt());
    }

    private String optionalText(JsonNode params, String field, String defaultValue) {
        String value = textOrEmpty(params.get(field));
        return value.isEmpty() ? defaultValue : value;
    }

    private List<String> readStringArray(JsonNode node) {
        List<String> out = new ArrayList<String>();
        if (node == null || !node.isArray()) {
            return out;
        }
        for (JsonNode item : node) {
            String value = textOrEmpty(item);
            if (!value.isEmpty()) {
                out.add(value);
            }
        }
        return out;
    }

    private Object rowValue(List<Map<String, Object>> row, int index) {
        if (row == null || index < 0 || index >= row.size()) {
            return null;
        }
        Map<String, Object> cell = row.get(index);
        if (cell == null || "null".equals(String.valueOf(cell.get("type")))) {
            return null;
        }
        return cell.get("value");
    }

    private String rowString(List<Map<String, Object>> row, int index) {
        Object value = rowValue(row, index);
        return value == null ? "" : String.valueOf(value);
    }

    private int rowInt(List<Map<String, Object>> row, int index) {
        Object value = rowValue(row, index);
        if (value instanceof Number) {
            return ((Number) value).intValue();
        }
        if (value == null || String.valueOf(value).trim().isEmpty()) {
            return 0;
        }
        return Integer.parseInt(String.valueOf(value));
    }

    private boolean nullable(String value) {
        String normalized = value == null ? "" : value.trim().toUpperCase();
        if ("NO".equals(normalized) || "N".equals(normalized) || "0".equals(normalized) || "FALSE".equals(normalized)) {
            return false;
        }
        return true;
    }

    private String gbase8sColumnType(String value) {
        int code;
        try {
            code = Integer.parseInt(value == null ? "" : value.trim());
        } catch (NumberFormatException error) {
            return value == null ? "" : value;
        }
        switch (code & 255) {
            case 0:
                return "CHAR";
            case 1:
                return "SMALLINT";
            case 2:
                return "INTEGER";
            case 3:
                return "FLOAT";
            case 4:
                return "SMALLFLOAT";
            case 5:
                return "DECIMAL";
            case 6:
                return "SERIAL";
            case 7:
                return "DATE";
            case 8:
                return "MONEY";
            case 10:
                return "DATETIME";
            case 11:
                return "BYTE";
            case 12:
                return "TEXT";
            case 13:
                return "VARCHAR";
            case 14:
                return "INTERVAL";
            case 15:
                return "NCHAR";
            case 16:
                return "NVARCHAR";
            case 17:
                return "INT8";
            case 18:
                return "SERIAL8";
            case 23:
                return "BOOLEAN";
            case 40:
                return "LVARCHAR";
            default:
                return value == null ? "" : value;
        }
    }

    private String textOrEmpty(JsonNode node) {
        return node == null || node.isNull() ? "" : node.asText("");
    }

    private JsonNode ok(JsonNode id, Object result) {
        ObjectNode response = JsonNodeFactory.instance.objectNode();
        response.put("jsonrpc", "2.0");
        response.set("id", id == null || id.isMissingNode() ? JsonNodeFactory.instance.nullNode() : id);
        response.set("result", result == null ? JsonNodeFactory.instance.nullNode() : mapper.valueToTree(result));
        return response;
    }

    private JsonNode error(JsonNode id, int code, String message) {
        ObjectNode response = JsonNodeFactory.instance.objectNode();
        ObjectNode error = JsonNodeFactory.instance.objectNode();
        response.put("jsonrpc", "2.0");
        response.set("id", id == null || id.isMissingNode() ? JsonNodeFactory.instance.nullNode() : id);
        error.put("code", code);
        error.put("message", message == null ? "" : message);
        response.set("error", error);
        return response;
    }

    private void closeAll() throws SQLException {
        SQLException failure = null;
        for (TxState tx : transactions.values()) {
            ConnectionState state = connections.get(Long.valueOf(tx.connId));
            if (state != null) {
                try {
                    state.connection.rollback();
                    state.connection.setAutoCommit(state.originalAutoCommit);
                } catch (SQLException error) {
                    failure = error;
                }
            }
        }
        transactions.clear();
        imports.clear();
        streams.clear();
        for (ConnectionState state : connections.values()) {
            try {
                state.connection.close();
            } catch (SQLException error) {
                failure = error;
            }
        }
        connections.clear();
        cursors.clear();
        if (failure != null) {
            throw failure;
        }
    }

    private void closeTransactionsForConn(long connId) throws SQLException {
        List<String> ids = new ArrayList<String>();
        for (Map.Entry<String, TxState> entry : transactions.entrySet()) {
            if (entry.getValue().connId == connId) {
                ids.add(entry.getKey());
            }
        }
        ConnectionState state = connections.get(Long.valueOf(connId));
        for (String id : ids) {
            if (state != null) {
                state.connection.rollback();
                state.connection.setAutoCommit(state.originalAutoCommit);
                state.activeTxId = null;
            }
            transactions.remove(id);
        }
    }

    private void closeImportsForConn(long connId) {
        List<String> ids = new ArrayList<String>();
        for (Map.Entry<String, ImportState> entry : imports.entrySet()) {
            if (entry.getValue().connId == connId) {
                ids.add(entry.getKey());
            }
        }
        for (String id : ids) {
            imports.remove(id);
        }
    }

    private static final class ConnectionState {
        private final GBase8sConfig config;
        private final Connection connection;
        private boolean originalAutoCommit = true;
        private String activeTxId;

        private ConnectionState(GBase8sConfig config, Connection connection) {
            this.config = config;
            this.connection = connection;
        }
    }

    private static final class TxState {
        private final long connId;
        private final Map<String, Savepoint> savepoints = new LinkedHashMap<String, Savepoint>();

        private TxState(long connId) {
            this.connId = connId;
        }
    }

    private static final class ImportState {
        private final long connId;
        private final String database;
        private final String schema;
        private final String table;
        private final List<String> columns;
        private final long startedAt = System.currentTimeMillis();
        private long inserted;

        private ImportState(long connId, String database, String schema, String table, List<String> columns) {
            this.connId = connId;
            this.database = database;
            this.schema = schema;
            this.table = table;
            this.columns = columns;
        }
    }

    private static final class StreamState {
        private final byte[] data;
        private int offset;

        private StreamState(byte[] data) {
            this.data = data;
        }
    }

    private static final class CursorState {
        private final List<List<Map<String, Object>>> rows;
        private int offset;

        private CursorState(List<List<Map<String, Object>>> rows) {
            this.rows = rows;
        }

        private List<List<Map<String, Object>>> take(int n) {
            int end = Math.min(rows.size(), offset + n);
            List<List<Map<String, Object>>> page = new ArrayList<List<Map<String, Object>>>(rows.subList(offset, end));
            offset = end;
            return page;
        }

        private boolean isDone() {
            return offset >= rows.size();
        }
    }
}
