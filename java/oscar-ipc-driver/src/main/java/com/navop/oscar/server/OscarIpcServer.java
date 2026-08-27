package com.navop.oscar.server;

import com.fasterxml.jackson.core.type.TypeReference;
import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import com.fasterxml.jackson.databind.node.ObjectNode;
import com.navop.oscar.db.JdbcQueryRunner;
import com.navop.oscar.db.QueryResult;
import com.navop.oscar.jdbc.OscarConfig;

import java.sql.Connection;
import java.sql.DatabaseMetaData;
import java.sql.ResultSet;
import java.sql.SQLException;
import java.sql.Savepoint;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Base64;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;

public final class OscarIpcServer {
    private static final String DRIVER_ID = "oscar";

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

    public OscarIpcServer(JdbcConnectionFactory connectionFactory) {
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
            return sqlError(id, ProtocolError.SQL_SYNTAX, error);
        } catch (Exception error) {
            return error(id, ProtocolError.INTERNAL_ERROR, error.getMessage());
        }
    }

    private JsonNode dispatch(JsonNode id, String method, JsonNode params) throws Exception {
        if ("init".equals(method)) {
            String incompatibility = HostVersion.incompatibility(params.path("host_version").asText(null));
            if (incompatibility != null) {
                return error(id, ProtocolError.SERVER_INCOMPATIBLE, incompatibility);
            }
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
        if ("schema/indexes".equals(method)) {
            return handleSchemaIndexes(id, params);
        }
        if ("schema/foreign_keys".equals(method)) {
            return handleSchemaForeignKeys(id, params);
        }
        if ("schema/checks".equals(method)) {
            return handleSchemaChecks(id, params);
        }
        if ("schema/views".equals(method)) {
            return handleSchemaViews(id, params);
        }
        if ("schema/functions".equals(method)) {
            return handleSchemaFunctions(id, params);
        }
        if ("schema/procedures".equals(method)) {
            return handleSchemaProcedures(id, params);
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
        if ("schema/triggers".equals(method) || "schema/sequences".equals(method) || "schema/types".equals(method)) {
            return ok(id, new ArrayList<Map<String, Object>>());
        }
        if ("schema/view_definition".equals(method)) {
            Map<String, Object> result = new LinkedHashMap<String, Object>();
            result.put("sql", "");
            result.put("is_materialized", Boolean.FALSE);
            return ok(id, result);
        }
        if ("schema/dump_ddl".equals(method)) {
            return handleSchemaDumpDdl(id, params);
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
        result.put("extension_version", "0.1.3");
        result.put("api_used", api);
        result.put("features", features);
        result.put("drivers_ready", drivers);
        result.put("methods", methods);
        result.put("name", "Oscar IPC Driver");
        return result;
    }

    private JsonNode handleConnTest(JsonNode id, JsonNode params) throws Exception {
        OscarConfig config = parseConfig(params);
        long start = System.currentTimeMillis();
        Connection connection = connectionFactory.open(config);
        try {
            connection.isValid(5);
            Map<String, Object> result = new LinkedHashMap<String, Object>();
            result.put("ok", Boolean.TRUE);
            result.put("server_version", "Oscar");
            result.put("warnings", new ArrayList<String>());
            result.put("latency_ms", Long.valueOf(System.currentTimeMillis() - start));
            return ok(id, result);
        } finally {
            connection.close();
        }
    }

    private JsonNode handleConnOpen(JsonNode id, JsonNode params) throws Exception {
        OscarConfig config = parseConfig(params);
        Connection connection = connectionFactory.open(config);
        connection.isValid(5);
        long connId = nextConnId++;
        connections.put(Long.valueOf(connId), new ConnectionState(config, connection));

        Map<String, Object> serverInfo = new LinkedHashMap<String, Object>();
        List<String> features = new ArrayList<String>();
        features.add("database_sql");
        serverInfo.put("version", "Oscar");
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
        return ok(id, readDatabases(state));
    }

    private JsonNode handleSchemaSchemas(JsonNode id, JsonNode params) throws SQLException {
        ConnectionState state = requireConnection(id, requiredLong(params, "conn_id"));
        if (state == null) {
            return lastError;
        }
        return ok(id, readSchemas(state.connection));
    }

    private JsonNode handleSchemaObjects(JsonNode id, JsonNode params) throws SQLException {
        ConnectionState state = requireConnection(id, requiredLong(params, "conn_id"));
        if (state == null) {
            return lastError;
        }
        String database = optionalText(params, "database", state.config.getDatabase());
        String schema = optionalText(params, "schema", "");
        List<String> kinds = readStringArray(params.path("kinds"));
        return ok(id, readObjects(state.connection, database, schema, kinds));
    }

    private JsonNode handleSchemaColumns(JsonNode id, JsonNode params) throws SQLException {
        ConnectionState state = requireConnection(id, requiredLong(params, "conn_id"));
        if (state == null) {
            return lastError;
        }
        String database = optionalText(params, "database", state.config.getDatabase());
        String schema = optionalText(params, "schema", "");
        String table = requiredText(params, "table");
        return ok(id, readColumns(state.connection, database, schema, table));
    }

    private JsonNode handleSchemaIndexes(JsonNode id, JsonNode params) throws SQLException {
        ConnectionState state = requireConnection(id, requiredLong(params, "conn_id"));
        if (state == null) {
            return lastError;
        }
        String database = optionalText(params, "database", state.config.getDatabase());
        String schema = optionalText(params, "schema", "");
        String table = requiredText(params, "table");
        List<Map<String, Object>> result = readIndexes(state.connection, database, schema, table);
        return ok(id, result);
    }

    private JsonNode handleSchemaForeignKeys(JsonNode id, JsonNode params) throws SQLException {
        ConnectionState state = requireConnection(id, requiredLong(params, "conn_id"));
        if (state == null) {
            return lastError;
        }
        String database = optionalText(params, "database", state.config.getDatabase());
        String schema = optionalText(params, "schema", "");
        String table = requiredText(params, "table");
        return ok(id, readForeignKeys(state.connection, database, schema, table));
    }

    private JsonNode handleSchemaChecks(JsonNode id, JsonNode params) {
        ConnectionState state = requireConnection(id, requiredLong(params, "conn_id"));
        if (state == null) {
            return lastError;
        }
        return ok(id, new ArrayList<Map<String, Object>>());
    }

    private JsonNode handleSchemaDumpDdl(JsonNode id, JsonNode params) throws SQLException {
        ConnectionState state = requireConnection(id, requiredLong(params, "conn_id"));
        if (state == null) {
            return lastError;
        }
        JsonNode objects = params.get("objects");
        if (objects == null || !objects.isArray()) {
            return ok(id, emptyDumpResult());
        }
        // The host sends every export target as an object ref. Only tables have
        // a JDBC-metadata based dump; views/sequences/etc. fall through to the
        // host's shared builders.
        String database = "";
        String schema = "";
        String table = "";
        for (JsonNode object : objects) {
            if (!isTableKind(textOrEmpty(object.get("kind")))) {
                continue;
            }
            database = optionalText(object, "database", "");
            schema = optionalText(object, "schema", "");
            table = textOrEmpty(object.get("name"));
            break;
        }
        if (table.isEmpty()) {
            return ok(id, emptyDumpResult());
        }
        if (database.isEmpty()) {
            database = state.config.getDatabase();
        }
        List<String> statements = buildTableDdl(state.connection, database, schema, table);
        Map<String, Object> result = new LinkedHashMap<String, Object>();
        result.put("statements", statements);
        return ok(id, result);
    }

    private boolean isTableKind(String kind) {
        return "table".equals(kind) || "base_table".equals(kind);
    }

    private Map<String, Object> emptyDumpResult() {
        Map<String, Object> result = new LinkedHashMap<String, Object>();
        result.put("statements", new ArrayList<String>());
        return result;
    }

    /**
     * 从 JDBC metadata 组装完整表结构 DDL（列、主键、注释、普通/唯一索引、外键）。
     * Oscar 没有稳定的服务端 get_ddl 函数，因此按 metadata 逐条重建，
     * 输出结果优于 host 的共享列构建器（后者不含索引与外键）。
     */
    private List<String> buildTableDdl(Connection connection, String database, String schema, String table) throws SQLException {
        List<Map<String, Object>> columns = readColumns(connection, database, schema, table);
        if (columns.isEmpty()) {
            return new ArrayList<String>();
        }
        String tableName = qualifiedIdentifier("", schema, table);
        List<String> definitions = new ArrayList<String>();
        List<String> primary = new ArrayList<String>();
        for (Map<String, Object> column : columns) {
            definitions.add(dumpColumnDefinition(column));
            if (Boolean.TRUE.equals(column.get("is_primary"))) {
                primary.add(quote(String.valueOf(column.get("name"))));
            }
        }
        if (!primary.isEmpty()) {
            definitions.add("PRIMARY KEY (" + join(primary, ", ") + ")");
        }
        List<String> statements = new ArrayList<String>();
        statements.add("CREATE TABLE " + tableName + " (" + join(definitions, ", ") + ")");
        String tableComment = readTableComment(connection, database, schema, table);
        if (!tableComment.isEmpty()) {
            statements.add(commentStatement(tableName, null, tableComment));
        }
        for (Map<String, Object> column : columns) {
            String comment = String.valueOf(column.get("comment"));
            if (!comment.isEmpty()) {
                statements.add(commentStatement(tableName, String.valueOf(column.get("name")), comment));
            }
        }
        for (Map<String, Object> index : readIndexes(connection, database, schema, table)) {
            if (Boolean.TRUE.equals(index.get("is_primary"))) {
                continue;
            }
            statements.add(indexStatement(schema, table, index));
        }
        for (Map<String, Object> foreignKey : readForeignKeys(connection, database, schema, table)) {
            statements.add(foreignKeyStatement(schema, foreignKey));
        }
        return statements;
    }

    private static String dumpColumnDefinition(Map<String, Object> column) {
        StringBuilder definition = new StringBuilder();
        definition.append(quote(String.valueOf(column.get("name")))).append(' ').append(String.valueOf(column.get("raw_type")));
        if (Boolean.FALSE.equals(column.get("nullable"))) {
            definition.append(" NOT NULL");
        }
        String defaultValue = rawDefault(column);
        if (!defaultValue.isEmpty()) {
            definition.append(" DEFAULT ").append(defaultValue);
        }
        return definition.toString();
    }

    /**
     * 优先使用 metadata 的原始 COLUMN_DEF（未去除引号的 raw_default），
     * 避免把字符串默认值 'abc' 展开成裸标识符 abc。
     */
    @SuppressWarnings("unchecked")
    private static String rawDefault(Map<String, Object> column) {
        Object extra = column.get("extra");
        if (extra instanceof Map) {
            Object raw = ((Map<String, Object>) extra).get("raw_default");
            if (raw != null && !String.valueOf(raw).isEmpty()) {
                return String.valueOf(raw);
            }
        }
        Object value = column.get("default");
        return value == null ? "" : String.valueOf(value);
    }

    private static String indexStatement(String schema, String table, Map<String, Object> index) {
        StringBuilder sql = new StringBuilder("CREATE ");
        if (Boolean.TRUE.equals(index.get("is_unique"))) {
            sql.append("UNIQUE ");
        }
        sql.append("INDEX ").append(quote(String.valueOf(index.get("name"))))
            .append(" ON ").append(qualifiedIdentifier("", schema, table))
            .append(" (").append(quoteList(columnList(index))).append(")");
        return sql.toString();
    }

    private static List<String> columnList(Map<String, Object> map) {
        return toStringList(map.get("columns"));
    }

    private String foreignKeyStatement(String schema, Map<String, Object> foreignKey) {
        StringBuilder sql = new StringBuilder("ALTER TABLE ");
        sql.append(qualifiedIdentifier("", schema, String.valueOf(foreignKey.get("from_table"))));
        sql.append(" ADD CONSTRAINT ").append(quote(String.valueOf(foreignKey.get("name"))));
        sql.append(" FOREIGN KEY (").append(quoteList(toStringList(foreignKey.get("from_columns")))).append(")");
        String refSchema = firstNonEmpty(String.valueOf(foreignKey.get("to_schema")), schema);
        sql.append(" REFERENCES ").append(qualifiedIdentifier("", refSchema, String.valueOf(foreignKey.get("to_table"))));
        List<String> refColumns = toStringList(foreignKey.get("to_columns"));
        sql.append(" (").append(quoteList(refColumns)).append(")");
        String onUpdate = String.valueOf(foreignKey.get("on_update"));
        if (!onUpdate.isEmpty() && !"NO ACTION".equals(onUpdate)) {
            sql.append(" ON UPDATE ").append(onUpdate);
        }
        String onDelete = String.valueOf(foreignKey.get("on_delete"));
        if (!onDelete.isEmpty() && !"NO ACTION".equals(onDelete)) {
            sql.append(" ON DELETE ").append(onDelete);
        }
        return sql.toString();
    }

    @SuppressWarnings("unchecked")
    private static List<String> toStringList(Object value) {
        List<String> result = new ArrayList<String>();
        if (value instanceof List) {
            for (Object item : (List<Object>) value) {
                result.add(String.valueOf(item));
            }
        }
        return result;
    }

    private String readTableComment(Connection connection, String database, String schema, String table) {
        try {
            DatabaseMetaData metadata = connection.getMetaData();
            for (String catalogPattern : catalogPatterns(database)) {
                for (String schemaPattern : schemaPatterns(schema)) {
                    ResultSet rows;
                    try {
                        rows = metadata.getTables(emptyToNull(catalogPattern), emptyToNull(schemaPattern), table, new String[]{"TABLE"});
                    } catch (SQLException error) {
                        continue;
                    }
                    try {
                        while (rows.next()) {
                            String rowSchema = resultString(rows, "TABLE_SCHEM");
                            String rowTable = resultString(rows, "TABLE_NAME");
                            if (!schemaMatches(schema, rowSchema) || !nameMatches(table, rowTable)) {
                                continue;
                            }
                            return resultString(rows, "REMARKS");
                        }
                    } finally {
                        rows.close();
                    }
                }
            }
        } catch (SQLException error) {
            return "";
        }
        return "";
    }

    private JsonNode handleSchemaViews(JsonNode id, JsonNode params) throws SQLException {
        ConnectionState state = requireConnection(id, requiredLong(params, "conn_id"));
        if (state == null) {
            return lastError;
        }
        String database = optionalText(params, "database", state.config.getDatabase());
        String schema = optionalText(params, "schema", "");
        return ok(id, readViews(state.connection, database, schema));
    }

    private JsonNode handleSchemaFunctions(JsonNode id, JsonNode params) throws SQLException {
        ConnectionState state = requireConnection(id, requiredLong(params, "conn_id"));
        if (state == null) {
            return lastError;
        }
        String database = optionalText(params, "database", state.config.getDatabase());
        String schema = optionalText(params, "schema", "");
        List<Map<String, Object>> result = readFunctions(state.connection, database, schema);
        return ok(id, result);
    }

    private JsonNode handleSchemaProcedures(JsonNode id, JsonNode params) throws SQLException {
        ConnectionState state = requireConnection(id, requiredLong(params, "conn_id"));
        if (state == null) {
            return lastError;
        }
        String database = optionalText(params, "database", state.config.getDatabase());
        String schema = optionalText(params, "schema", "");
        List<Map<String, Object>> result = readProcedures(state.connection, database, schema);
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
            List<List<String>> rows = new ArrayList<List<String>>();
            for (Map<String, Object> databaseRow : readDatabases(state)) {
                rows.add(rowValues(String.valueOf(databaseRow.get("name"))));
            }
            return ok(id, objectView("Databases", objectViewColumns("name", "Name"), rows));
        }
        if ("schemas".equals(view)) {
            List<List<String>> rows = new ArrayList<List<String>>();
            for (Map<String, Object> schemaRow : readSchemas(state.connection)) {
                rows.add(rowValues(String.valueOf(schemaRow.get("name")), String.valueOf(schemaRow.get("owner"))));
            }
            return ok(id, objectView("Schemas", objectViewColumns("name", "Name", "owner", "Owner"), rows));
        }
        if ("tables".equals(view)) {
            List<List<String>> rows = new ArrayList<List<String>>();
            for (Map<String, Object> object : readObjects(state.connection, database, schema, singletonStringList("table"))) {
                rows.add(rowValues(String.valueOf(object.get("name")), String.valueOf(object.get("kind")), String.valueOf(object.get("comment"))));
            }
            return ok(id, objectView("Tables", objectViewColumns("name", "Name", "kind", "Kind", "comment", "Comment"), rows));
        }
        if ("views".equals(view)) {
            List<List<String>> rows = new ArrayList<List<String>>();
            for (Map<String, Object> object : readViews(state.connection, database, schema)) {
                rows.add(rowValues(String.valueOf(object.get("name")), String.valueOf(object.get("kind")), String.valueOf(object.get("comment"))));
            }
            return ok(id, objectView("Views", objectViewColumns("name", "Name", "kind", "Kind", "comment", "Comment"), rows));
        }
        if ("columns".equals(view)) {
            String table = requiredText(params, "table");
            List<List<String>> rows = new ArrayList<List<String>>();
            for (Map<String, Object> column : readColumns(state.connection, database, schema, table)) {
                rows.add(rowValues(
                    String.valueOf(column.get("name")),
                    String.valueOf(column.get("type")),
                    String.valueOf(column.get("nullable")),
                    emptyIfNull((String) column.get("default")),
                    String.valueOf(column.get("comment"))
                ));
            }
            return ok(id, objectView("Columns", columnObjectViewColumns(), rows));
        }
        if ("indexes".equals(view)) {
            String table = requiredText(params, "table");
            List<List<String>> rows = new ArrayList<List<String>>();
            for (Map<String, Object> index : readIndexes(state.connection, database, schema, table)) {
                rows.add(rowValues(
                    String.valueOf(index.get("name")),
                    join(stringList(index.get("columns")), ", "),
                    String.valueOf(index.get("is_unique")),
                    String.valueOf(index.get("is_primary")),
                    String.valueOf(index.get("type"))
                ));
            }
            return ok(id, objectView("Indexes", indexObjectViewColumns(), rows));
        }
        if ("functions".equals(view)) {
            List<List<String>> rows = new ArrayList<List<String>>();
            for (Map<String, Object> function : readFunctions(state.connection, database, schema)) {
                rows.add(rowValues(
                    String.valueOf(function.get("name")),
                    String.valueOf(function.get("returns")),
                    String.valueOf(function.get("language")),
                    String.valueOf(function.get("comment"))
                ));
            }
            return ok(id, objectView("Functions", objectViewColumns("name", "Name", "returns", "Returns", "language", "Language", "comment", "Comment"), rows));
        }
        if ("procedures".equals(view)) {
            List<List<String>> rows = new ArrayList<List<String>>();
            for (Map<String, Object> procedure : readProcedures(state.connection, database, schema)) {
                rows.add(rowValues(
                    String.valueOf(procedure.get("name")),
                    String.valueOf(procedure.get("language")),
                    String.valueOf(procedure.get("comment"))
                ));
            }
            return ok(id, objectView("Procedures", objectViewColumns("name", "Name", "language", "Language", "comment", "Comment"), rows));
        }
        if ("triggers".equals(view) || "sequences".equals(view)) {
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

    private List<Map<String, Object>> readDatabases(ConnectionState state) {
        List<Map<String, Object>> result = new ArrayList<Map<String, Object>>();
        String name = trim(state.config.getDatabase());
        if (name.isEmpty()) {
            try {
                name = trim(state.connection.getCatalog());
            } catch (SQLException error) {
                name = "";
            }
        }
        if (name.isEmpty()) {
            name = "Oscar";
        }
        Map<String, Object> database = new LinkedHashMap<String, Object>();
        database.put("name", name);
        database.put("charset", null);
        database.put("collation", null);
        database.put("comment", "");
        database.put("owner", null);
        database.put("size_bytes", null);
        database.put("extra", new LinkedHashMap<String, Object>());
        result.add(database);
        return result;
    }

    private List<Map<String, Object>> readSchemas(Connection connection) throws SQLException {
        DatabaseMetaData metadata = connection.getMetaData();
        Map<String, Map<String, Object>> schemas = new LinkedHashMap<String, Map<String, Object>>();
        ResultSet rows = metadata.getSchemas();
        try {
            while (rows.next()) {
                String name = firstNonEmpty(resultString(rows, "TABLE_SCHEM"), resultString(rows, "TABLE_SCHEMA"));
                if (name.isEmpty()) {
                    continue;
                }
                Map<String, Object> schema = new LinkedHashMap<String, Object>();
                schema.put("name", name);
                schema.put("owner", name);
                schema.put("comment", "");
                schemas.put(name.toUpperCase(), schema);
            }
        } finally {
            rows.close();
        }
        return new ArrayList<Map<String, Object>>(schemas.values());
    }

    @SuppressWarnings("unchecked")
    private List<String> stringList(Object value) {
        if (value instanceof List<?>) {
            List<String> out = new ArrayList<String>();
            for (Object item : (List<Object>) value) {
                out.add(String.valueOf(item));
            }
            return out;
        }
        return new ArrayList<String>();
    }

    private List<Map<String, Object>> readObjects(Connection connection, String database, String schema, List<String> kinds) throws SQLException {
        List<String> wantedKinds = kinds == null || kinds.isEmpty() ? tableAndViewKinds() : kinds;
        List<String> types = tableTypes(wantedKinds);
        if (types.isEmpty()) {
            return new ArrayList<Map<String, Object>>();
        }
        DatabaseMetaData metadata = connection.getMetaData();
        Map<String, Map<String, Object>> objects = new LinkedHashMap<String, Map<String, Object>>();
        for (String catalogPattern : catalogPatterns(database)) {
            for (String schemaPattern : schemaPatterns(schema)) {
                ResultSet rows;
                try {
                    rows = metadata.getTables(emptyToNull(catalogPattern), emptyToNull(schemaPattern), "%", types.toArray(new String[types.size()]));
                } catch (SQLException error) {
                    continue;
                }
                try {
                    while (rows.next()) {
                        String rowSchema = resultString(rows, "TABLE_SCHEM");
                        String name = resultString(rows, "TABLE_NAME");
                        String kind = tableKind(resultString(rows, "TABLE_TYPE"));
                        if (name.isEmpty() || kind.isEmpty() || !wantedKinds.contains(kind) || !schemaMatches(schema, rowSchema)) {
                            continue;
                        }
                        String key = firstNonEmpty(rowSchema, schema) + "." + name + "." + kind;
                        if (objects.containsKey(key)) {
                            continue;
                        }
                        Map<String, Object> object = new LinkedHashMap<String, Object>();
                        object.put("database", database);
                        object.put("schema", firstNonEmpty(rowSchema, schema));
                        object.put("name", name);
                        object.put("kind", kind);
                        object.put("comment", resultString(rows, "REMARKS"));
                        object.put("row_count_estimate", null);
                        object.put("size_bytes", null);
                        object.put("created_at", null);
                        object.put("updated_at", null);
                        object.put("extra", new LinkedHashMap<String, Object>());
                        objects.put(key, object);
                    }
                } finally {
                    rows.close();
                }
                if (!objects.isEmpty()) {
                    return new ArrayList<Map<String, Object>>(objects.values());
                }
            }
        }
        return new ArrayList<Map<String, Object>>(objects.values());
    }

    private List<Map<String, Object>> readColumns(Connection connection, String database, String schema, String table) throws SQLException {
        PrimaryKeyInfo primaryKey = readPrimaryKey(connection, database, schema, table);
        DatabaseMetaData metadata = connection.getMetaData();
        List<Map<String, Object>> columns = new ArrayList<Map<String, Object>>();
        for (String catalogPattern : catalogPatterns(database)) {
            for (String schemaPattern : schemaPatterns(schema)) {
                ResultSet rows;
                try {
                    rows = metadata.getColumns(emptyToNull(catalogPattern), emptyToNull(schemaPattern), table, "%");
                } catch (SQLException error) {
                    continue;
                }
                try {
                    while (rows.next()) {
                        String rowSchema = resultString(rows, "TABLE_SCHEM");
                        String rowTable = resultString(rows, "TABLE_NAME");
                        String name = resultString(rows, "COLUMN_NAME");
                        if (name.isEmpty() || !schemaMatches(schema, rowSchema) || !nameMatches(table, rowTable)) {
                            continue;
                        }
                        int nullableCode = resultInt(rows, "NULLABLE", DatabaseMetaData.columnNullableUnknown);
                        int size = resultInt(rows, "COLUMN_SIZE", 0);
                        int scale = resultInt(rows, "DECIMAL_DIGITS", 0);
                        String rawType = resultString(rows, "TYPE_NAME");
                        String rawColumnDef = resultString(rows, "COLUMN_DEF");
                        String typeName = columnType(rawType, size, scale);
                        Map<String, Object> extra = new LinkedHashMap<String, Object>();
                        extra.put("raw_default", rawColumnDef);
                        Map<String, Object> column = new LinkedHashMap<String, Object>();
                        column.put("ordinal", Integer.valueOf(resultInt(rows, "ORDINAL_POSITION", columns.size() + 1)));
                        column.put("name", name);
                        column.put("type", typeName);
                        column.put("raw_type", typeName);
                        column.put("nullable", Boolean.valueOf(nullableCode != DatabaseMetaData.columnNoNulls));
                        column.put("default", metadataDefault(rawColumnDef));
                        column.put("is_primary", Boolean.valueOf(containsIgnoreCase(primaryKey.columns, name)));
                        column.put("is_unique", Boolean.FALSE);
                        column.put("is_partition_key", Boolean.FALSE);
                        column.put("is_clustering_key", Boolean.FALSE);
                        column.put("max_length", size > 0 ? Integer.valueOf(size) : null);
                        column.put("precision", size > 0 ? Integer.valueOf(size) : null);
                        column.put("scale", scale > 0 ? Integer.valueOf(scale) : null);
                        column.put("comment", resultString(rows, "REMARKS"));
                        column.put("extra", extra);
                        columns.add(column);
                    }
                } finally {
                    rows.close();
                }
                if (!columns.isEmpty()) {
                    return columns;
                }
            }
        }
        return columns;
    }

    private List<Map<String, Object>> readIndexes(Connection connection, String database, String schema, String table) {
        PrimaryKeyInfo primaryKey = readPrimaryKey(connection, database, schema, table);
        Map<String, Map<String, Object>> indexes = new LinkedHashMap<String, Map<String, Object>>();
        Map<String, TreeMap<Integer, String>> columnOrders = new LinkedHashMap<String, TreeMap<Integer, String>>();
        try {
            DatabaseMetaData metadata = connection.getMetaData();
            for (String catalogPattern : catalogPatterns(database)) {
                for (String schemaPattern : schemaPatterns(schema)) {
                    ResultSet rows;
                    try {
                        rows = metadata.getIndexInfo(emptyToNull(catalogPattern), emptyToNull(schemaPattern), table, false, false);
                    } catch (SQLException error) {
                        continue;
                    }
                    try {
                        while (rows.next()) {
                            String rowSchema = resultString(rows, "TABLE_SCHEM");
                            String rowTable = resultString(rows, "TABLE_NAME");
                            String name = resultString(rows, "INDEX_NAME");
                            String column = resultString(rows, "COLUMN_NAME");
                            if (name.isEmpty() || column.isEmpty() || !schemaMatches(schema, rowSchema) || !nameMatches(table, rowTable)) {
                                continue;
                            }
                            Map<String, Object> index = indexes.get(name);
                            if (index == null) {
                                boolean primary = primaryKey.matches(name, new ArrayList<String>());
                                boolean unique = !resultBoolean(rows, "NON_UNIQUE", true);
                                index = new LinkedHashMap<String, Object>();
                                index.put("database", database);
                                index.put("schema", firstNonEmpty(rowSchema, schema));
                                index.put("table", table);
                                index.put("name", name);
                                index.put("columns", new ArrayList<String>());
                                index.put("is_unique", Boolean.valueOf(primary || unique));
                                index.put("is_primary", Boolean.valueOf(primary));
                                index.put("type", primary ? "PRIMARY" : (unique ? "UNIQUE" : "INDEX"));
                                index.put("comment", "");
                                index.put("extra", new LinkedHashMap<String, Object>());
                                indexes.put(name, index);
                                columnOrders.put(name, new TreeMap<Integer, String>());
                            }
                            int ordinal = resultInt(rows, "ORDINAL_POSITION", columnOrders.get(name).size() + 1);
                            columnOrders.get(name).put(Integer.valueOf(ordinal), column);
                        }
                    } finally {
                        rows.close();
                    }
                    if (!indexes.isEmpty()) {
                        for (Map.Entry<String, Map<String, Object>> entry : indexes.entrySet()) {
                            List<String> columns = new ArrayList<String>(columnOrders.get(entry.getKey()).values());
                            boolean primary = primaryKey.matches(entry.getKey(), columns);
                            entry.getValue().put("columns", columns);
                            entry.getValue().put("is_primary", Boolean.valueOf(primary));
                            if (primary) {
                                entry.getValue().put("is_unique", Boolean.TRUE);
                                entry.getValue().put("type", "PRIMARY");
                            }
                        }
                        return new ArrayList<Map<String, Object>>(indexes.values());
                    }
                }
            }
        } catch (SQLException error) {
            return new ArrayList<Map<String, Object>>();
        }
        return new ArrayList<Map<String, Object>>(indexes.values());
    }

    private List<Map<String, Object>> readForeignKeys(Connection connection, String database, String schema, String table) {
        Map<String, Map<String, Object>> foreignKeys = new LinkedHashMap<String, Map<String, Object>>();
        Map<String, TreeMap<Integer, String>> fromColumns = new LinkedHashMap<String, TreeMap<Integer, String>>();
        Map<String, TreeMap<Integer, String>> toColumns = new LinkedHashMap<String, TreeMap<Integer, String>>();
        try {
            DatabaseMetaData metadata = connection.getMetaData();
            for (String catalogPattern : catalogPatterns(database)) {
                for (String schemaPattern : schemaPatterns(schema)) {
                    ResultSet rows;
                    try {
                        rows = metadata.getImportedKeys(emptyToNull(catalogPattern), emptyToNull(schemaPattern), table);
                    } catch (SQLException error) {
                        continue;
                    }
                    try {
                        while (rows.next()) {
                            String rowSchema = resultString(rows, "FKTABLE_SCHEM");
                            String rowTable = resultString(rows, "FKTABLE_NAME");
                            if (!schemaMatches(schema, rowSchema) || !nameMatches(table, rowTable)) {
                                continue;
                            }
                            String name = firstNonEmpty(resultString(rows, "FK_NAME"), rowTable + "_fk_" + resultString(rows, "PKTABLE_NAME"));
                            Map<String, Object> foreignKey = foreignKeys.get(name);
                            if (foreignKey == null) {
                                foreignKey = new LinkedHashMap<String, Object>();
                                foreignKey.put("database", database);
                                foreignKey.put("schema", firstNonEmpty(rowSchema, schema));
                                foreignKey.put("name", name);
                                foreignKey.put("from_table", rowTable);
                                foreignKey.put("from_columns", new ArrayList<String>());
                                foreignKey.put("to_table", resultString(rows, "PKTABLE_NAME"));
                                foreignKey.put("to_schema", resultString(rows, "PKTABLE_SCHEM"));
                                foreignKey.put("to_columns", new ArrayList<String>());
                                foreignKey.put("on_update", jdbcReferentialAction(resultShort(rows, "UPDATE_RULE", (short) DatabaseMetaData.importedKeyNoAction)));
                                foreignKey.put("on_delete", jdbcReferentialAction(resultShort(rows, "DELETE_RULE", (short) DatabaseMetaData.importedKeyNoAction)));
                                foreignKey.put("comment", "");
                                foreignKey.put("extra", new LinkedHashMap<String, Object>());
                                foreignKeys.put(name, foreignKey);
                                fromColumns.put(name, new TreeMap<Integer, String>());
                                toColumns.put(name, new TreeMap<Integer, String>());
                            }
                            int sequence = resultInt(rows, "KEY_SEQ", fromColumns.get(name).size() + 1);
                            fromColumns.get(name).put(Integer.valueOf(sequence), resultString(rows, "FKCOLUMN_NAME"));
                            toColumns.get(name).put(Integer.valueOf(sequence), resultString(rows, "PKCOLUMN_NAME"));
                        }
                    } finally {
                        rows.close();
                    }
                    if (!foreignKeys.isEmpty()) {
                        for (Map.Entry<String, Map<String, Object>> entry : foreignKeys.entrySet()) {
                            entry.getValue().put("from_columns", new ArrayList<String>(fromColumns.get(entry.getKey()).values()));
                            entry.getValue().put("to_columns", new ArrayList<String>(toColumns.get(entry.getKey()).values()));
                        }
                        return new ArrayList<Map<String, Object>>(foreignKeys.values());
                    }
                }
            }
        } catch (SQLException error) {
            return new ArrayList<Map<String, Object>>();
        }
        return new ArrayList<Map<String, Object>>(foreignKeys.values());
    }

    private List<Map<String, Object>> readViews(Connection connection, String database, String schema) throws SQLException {
        List<Map<String, Object>> views = readObjects(connection, database, schema, singletonStringList("view"));
        for (Map<String, Object> view : views) {
            view.put("definition_sql", "");
        }
        return views;
    }

    private List<Map<String, Object>> readFunctions(Connection connection, String database, String schema) {
        Map<String, Map<String, Object>> routines = new LinkedHashMap<String, Map<String, Object>>();
        try {
            DatabaseMetaData metadata = connection.getMetaData();
            for (String catalogPattern : catalogPatterns(database)) {
                for (String schemaPattern : schemaPatterns(schema)) {
                    ResultSet rows;
                    try {
                        rows = metadata.getFunctions(emptyToNull(catalogPattern), emptyToNull(schemaPattern), "%");
                    } catch (SQLException error) {
                        continue;
                    }
                    try {
                        while (rows.next()) {
                            String rowSchema = resultString(rows, "FUNCTION_SCHEM");
                            String name = resultString(rows, "FUNCTION_NAME");
                            if (name.isEmpty() || !schemaMatches(schema, rowSchema)) {
                                continue;
                            }
                            String returnType = functionReturnType(metadata, catalogPattern, rowSchema, name);
                            Map<String, Object> routine = new LinkedHashMap<String, Object>();
                            routine.put("database", database);
                            routine.put("schema", firstNonEmpty(rowSchema, schema));
                            routine.put("name", name);
                            routine.put("return_type", returnType.isEmpty() ? null : returnType);
                            routine.put("returns", returnType);
                            routine.put("language", "");
                            routine.put("comment", resultString(rows, "REMARKS"));
                            routine.put("definition", "");
                            routine.put("extra", new LinkedHashMap<String, Object>());
                            routines.put(firstNonEmpty(rowSchema, schema) + "." + name, routine);
                        }
                    } finally {
                        rows.close();
                    }
                    if (!routines.isEmpty()) {
                        return new ArrayList<Map<String, Object>>(routines.values());
                    }
                }
            }
        } catch (SQLException error) {
            return new ArrayList<Map<String, Object>>();
        }
        return new ArrayList<Map<String, Object>>(routines.values());
    }

    private List<Map<String, Object>> readProcedures(Connection connection, String database, String schema) {
        Map<String, Map<String, Object>> routines = new LinkedHashMap<String, Map<String, Object>>();
        try {
            DatabaseMetaData metadata = connection.getMetaData();
            for (String catalogPattern : catalogPatterns(database)) {
                for (String schemaPattern : schemaPatterns(schema)) {
                    ResultSet rows;
                    try {
                        rows = metadata.getProcedures(emptyToNull(catalogPattern), emptyToNull(schemaPattern), "%");
                    } catch (SQLException error) {
                        continue;
                    }
                    try {
                        while (rows.next()) {
                            String rowSchema = resultString(rows, "PROCEDURE_SCHEM");
                            String name = resultString(rows, "PROCEDURE_NAME");
                            if (name.isEmpty() || !schemaMatches(schema, rowSchema)) {
                                continue;
                            }
                            Map<String, Object> routine = new LinkedHashMap<String, Object>();
                            routine.put("database", database);
                            routine.put("schema", firstNonEmpty(rowSchema, schema));
                            routine.put("name", name);
                            routine.put("return_type", null);
                            routine.put("returns", "");
                            routine.put("language", "");
                            routine.put("comment", resultString(rows, "REMARKS"));
                            routine.put("definition", "");
                            routine.put("extra", new LinkedHashMap<String, Object>());
                            routines.put(firstNonEmpty(rowSchema, schema) + "." + name, routine);
                        }
                    } finally {
                        rows.close();
                    }
                    if (!routines.isEmpty()) {
                        return new ArrayList<Map<String, Object>>(routines.values());
                    }
                }
            }
        } catch (SQLException error) {
            return new ArrayList<Map<String, Object>>();
        }
        return new ArrayList<Map<String, Object>>(routines.values());
    }

    private String functionReturnType(DatabaseMetaData metadata, String catalog, String schema, String name) {
        try {
            ResultSet rows = metadata.getFunctionColumns(emptyToNull(catalog), emptyToNull(schema), name, "%");
            try {
                while (rows.next()) {
                    if (resultInt(rows, "COLUMN_TYPE", -1) == DatabaseMetaData.functionReturn) {
                        return resultString(rows, "TYPE_NAME");
                    }
                }
            } finally {
                rows.close();
            }
        } catch (SQLException error) {
            return "";
        }
        return "";
    }

    private PrimaryKeyInfo readPrimaryKey(Connection connection, String database, String schema, String table) {
        PrimaryKeyInfo info = new PrimaryKeyInfo();
        try {
            DatabaseMetaData metadata = connection.getMetaData();
            for (String catalogPattern : catalogPatterns(database)) {
                for (String schemaPattern : schemaPatterns(schema)) {
                    ResultSet rows;
                    try {
                        rows = metadata.getPrimaryKeys(emptyToNull(catalogPattern), emptyToNull(schemaPattern), table);
                    } catch (SQLException error) {
                        continue;
                    }
                    TreeMap<Integer, String> ordered = new TreeMap<Integer, String>();
                    try {
                        while (rows.next()) {
                            String rowSchema = resultString(rows, "TABLE_SCHEM");
                            String rowTable = resultString(rows, "TABLE_NAME");
                            String column = resultString(rows, "COLUMN_NAME");
                            if (column.isEmpty() || !schemaMatches(schema, rowSchema) || !nameMatches(table, rowTable)) {
                                continue;
                            }
                            if (info.name.isEmpty()) {
                                info.name = resultString(rows, "PK_NAME");
                            }
                            ordered.put(Integer.valueOf(resultInt(rows, "KEY_SEQ", ordered.size() + 1)), column);
                        }
                    } finally {
                        rows.close();
                    }
                    if (!ordered.isEmpty()) {
                        info.columns.addAll(ordered.values());
                        return info;
                    }
                }
            }
        } catch (SQLException error) {
            return info;
        }
        return info;
    }

    private List<String> tableAndViewKinds() {
        List<String> kinds = new ArrayList<String>();
        kinds.add("table");
        kinds.add("view");
        return kinds;
    }

    private List<String> tableTypes(List<String> kinds) {
        List<String> types = new ArrayList<String>();
        if (kinds.contains("table")) {
            types.add("TABLE");
        }
        if (kinds.contains("view")) {
            types.add("VIEW");
        }
        return types;
    }

    private List<String> singletonStringList(String value) {
        List<String> values = new ArrayList<String>();
        values.add(value);
        return values;
    }

    private String tableKind(String tableType) {
        String normalized = tableType == null ? "" : tableType.trim().toUpperCase();
        if ("TABLE".equals(normalized)) {
            return "table";
        }
        if ("VIEW".equals(normalized)) {
            return "view";
        }
        return "";
    }

    private List<String> catalogPatterns(String database) {
        List<String> patterns = new ArrayList<String>();
        addPattern(patterns, database);
        addPattern(patterns, null);
        return patterns;
    }

    private List<String> schemaPatterns(String schema) {
        List<String> patterns = new ArrayList<String>();
        addPattern(patterns, schema);
        if (schema != null && !schema.trim().isEmpty()) {
            addPattern(patterns, schema.trim().toUpperCase());
            addPattern(patterns, schema.trim().toLowerCase());
        }
        addPattern(patterns, null);
        return patterns;
    }

    private void addPattern(List<String> patterns, String value) {
        String text = trim(value);
        if (!patterns.contains(text)) {
            patterns.add(text);
        }
    }

    private String emptyToNull(String value) {
        return value == null || value.trim().isEmpty() ? null : value;
    }

    private boolean schemaMatches(String expected, String actual) {
        String expectedText = trim(expected);
        if (expectedText.isEmpty()) {
            return true;
        }
        return expectedText.equalsIgnoreCase(trim(actual));
    }

    private boolean nameMatches(String expected, String actual) {
        String expectedText = trim(expected);
        if (expectedText.isEmpty()) {
            return true;
        }
        return expectedText.equalsIgnoreCase(trim(actual));
    }

    private boolean containsIgnoreCase(List<String> values, String expected) {
        for (String value : values) {
            if (trim(value).equalsIgnoreCase(trim(expected))) {
                return true;
            }
        }
        return false;
    }

    private String firstNonEmpty(String first, String second) {
        String firstText = trim(first);
        return firstText.isEmpty() ? trim(second) : firstText;
    }

    private String trim(String value) {
        return value == null ? "" : value.trim();
    }

    private String resultString(ResultSet rows, String column) {
        try {
            String value = rows.getString(column);
            return trim(value);
        } catch (SQLException error) {
            return "";
        }
    }

    private int resultInt(ResultSet rows, String column, int defaultValue) {
        try {
            int value = rows.getInt(column);
            return rows.wasNull() ? defaultValue : value;
        } catch (SQLException error) {
            return defaultValue;
        }
    }

    private short resultShort(ResultSet rows, String column, short defaultValue) {
        try {
            short value = rows.getShort(column);
            return rows.wasNull() ? defaultValue : value;
        } catch (SQLException error) {
            return defaultValue;
        }
    }

    private boolean resultBoolean(ResultSet rows, String column, boolean defaultValue) {
        try {
            boolean value = rows.getBoolean(column);
            return rows.wasNull() ? defaultValue : value;
        } catch (SQLException error) {
            return defaultValue;
        }
    }

    private String metadataDefault(String value) {
        String text = trim(value);
        if (text.length() >= 2 && text.startsWith("'") && text.endsWith("'")) {
            return text.substring(1, text.length() - 1).replace("''", "'");
        }
        return text.isEmpty() ? null : text;
    }

    private String jdbcReferentialAction(short value) {
        if (value == DatabaseMetaData.importedKeyCascade) {
            return "CASCADE";
        }
        if (value == DatabaseMetaData.importedKeySetNull) {
            return "SET NULL";
        }
        if (value == DatabaseMetaData.importedKeySetDefault) {
            return "SET DEFAULT";
        }
        if (value == DatabaseMetaData.importedKeyRestrict) {
            return "RESTRICT";
        }
        return "NO ACTION";
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
                    item.put("message", sqlErrorMessage(error));
                    item.put("data", sqlErrorData(error));
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
            if (col.path("is_primary").asBoolean(false)) {
                primary.add(requiredText(col, "name"));
            }
            defs.add(columnDefinition(col));
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
        if (params.path("options").path("with_comments").asBoolean(true)) {
            addCommentStatements(statements, qualifiedIdentifier("", schema, table),
                    optionalText(spec, "comment", ""), spec.path("columns"));
        }
        Map<String, Object> result = new LinkedHashMap<String, Object>();
        result.put("sql", sql.toString());
        result.put("statements", statements);
        return ok(id, result);
    }

    private JsonNode handleDdlBuildAlterTable(JsonNode id, JsonNode params) {
        JsonNode toSpec = params.path("to_spec");
        JsonNode fromSpec = params.path("from_spec");
        String table = requiredText(toSpec, "name");
        String schema = optionalText(toSpec, "schema", "");
        JsonNode options = params.path("options");
        boolean withRollback = options.path("with_rollback").asBoolean(false);
        boolean allowDestructive = options.path("allow_destructive").asBoolean(false);
        List<String> statements = new ArrayList<String>();
        List<String> rollback = new ArrayList<String>();
        List<String> warnings = new ArrayList<String>();
        Map<String, JsonNode> fromColumns = columnsByName(fromSpec.path("columns"));
        Map<String, JsonNode> toColumns = columnsByName(toSpec.path("columns"));
        String tableName = qualifiedIdentifier("", schema, table);
        String fromTableComment = optionalText(fromSpec, "comment", "");
        String toTableComment = optionalText(toSpec, "comment", "");
        if (!fromTableComment.equals(toTableComment)) {
            statements.add(commentStatement(tableName, null, toTableComment));
            if (withRollback) {
                rollback.add(0, commentStatement(tableName, null, fromTableComment));
            }
        }
        for (JsonNode rename : params.path("column_renames")) {
            String oldName = rename.path("old_name").asText("");
            String newName = rename.path("new_name").asText("");
            if (!oldName.trim().isEmpty() && !newName.trim().isEmpty() && !oldName.equals(newName)) {
                statements.add("ALTER TABLE " + tableName + " RENAME COLUMN " + quote(oldName) + " TO " + quote(newName));
                if (withRollback) {
                    rollback.add(0, "ALTER TABLE " + tableName + " RENAME COLUMN " + quote(newName) + " TO " + quote(oldName));
                }
            }
        }
        for (JsonNode column : toSpec.path("columns")) {
            String name = column.path("name").asText("");
            if (!name.isEmpty() && !fromColumns.containsKey(name)) {
                statements.add("ALTER TABLE " + tableName + " ADD " + columnDefinition(column));
                if (withRollback) {
                    rollback.add(0, "ALTER TABLE " + tableName + " DROP " + quote(name));
                }
                String comment = optionalText(column, "comment", "");
                if (!comment.isEmpty()) {
                    statements.add(commentStatement(tableName, name, comment));
                }
            }
        }
        if (allowDestructive) {
            for (JsonNode column : fromSpec.path("columns")) {
                String name = column.path("name").asText("");
                if (!name.isEmpty() && !toColumns.containsKey(name)) {
                    statements.add("ALTER TABLE " + tableName + " DROP " + quote(name));
                    warnings.add("drop column may lose data: " + name);
                }
            }
        }
        for (JsonNode column : toSpec.path("columns")) {
            String name = column.path("name").asText("");
            JsonNode fromColumn = fromColumns.get(name);
            if (name.isEmpty() || fromColumn == null) {
                continue;
            }
            String fromComment = optionalText(fromColumn, "comment", "");
            String toComment = optionalText(column, "comment", "");
            if (!fromComment.equals(toComment)) {
                statements.add(commentStatement(tableName, name, toComment));
                if (withRollback) {
                    rollback.add(0, commentStatement(tableName, name, fromComment));
                }
            }
        }
        Map<String, Object> result = new LinkedHashMap<String, Object>();
        result.put("statements", statements);
        result.put("rollback_statements", rollback);
        result.put("warnings", warnings);
        return ok(id, result);
    }

    /**
     * 追加 COMMENT ON TABLE / COMMENT ON COLUMN 语句（仅在注释非空时）。
     */
    private void addCommentStatements(List<String> statements, String tableName, String tableComment, JsonNode columns) {
        if (tableComment != null && !tableComment.isEmpty()) {
            statements.add(commentStatement(tableName, null, tableComment));
        }
        for (JsonNode column : columns) {
            String name = column.path("name").asText("");
            String comment = optionalText(column, "comment", "");
            if (!name.isEmpty() && !comment.isEmpty()) {
                statements.add(commentStatement(tableName, name, comment));
            }
        }
    }

    private static String commentStatement(String tableName, String column, String comment) {
        String kind = "TABLE";
        String target = tableName;
        if (column != null && !column.isEmpty()) {
            kind = "COLUMN";
            target = tableName + "." + quote(column);
        }
        return "COMMENT ON " + kind + " " + target + " IS '" + sqlString(comment == null ? "" : comment) + "'";
    }

    private static String sqlString(String value) {
        return value.replace("'", "''");
    }

    /**
     * 补全 JDBC TYPE_NAME 的类型限定符：CHAR/VARCHAR 追加长度，
     * DECIMAL/NUMERIC 追加 (精度[,小数位])，TIMESTAMP/DATETIME/TIME
     * 追加小数秒精度。真实驱动若已返回带括号的类型则原样保留。
     */
    private static String columnType(String base, int size, int scale) {
        if (base.isEmpty() || base.indexOf('(') >= 0 || base.indexOf(' ') >= 0) {
            return base;
        }
        String upper = base.toUpperCase();
        if (upper.contains("CHAR") || upper.contains("VARCHAR") || upper.contains("BINARY")) {
            if (size > 0) {
                return base + "(" + size + ")";
            }
        } else if ("DECIMAL".equals(upper) || "NUMERIC".equals(upper) || "NUMBER".equals(upper)) {
            if (size > 0) {
                return scale > 0 ? base + "(" + size + "," + scale + ")" : base + "(" + size + ")";
            }
        } else if ("TIMESTAMP".equals(upper) || "DATETIME".equals(upper) || "TIME".equals(upper)) {
            if (scale > 0) {
                return base + "(" + scale + ")";
            }
        }
        return base;
    }

    private String columnDefinition(JsonNode col) {
        String name = requiredText(col, "name");
        String type = requiredText(col, "type");
        StringBuilder def = new StringBuilder();
        def.append(qualifiedIdentifier("", "", name)).append(' ').append(type);
        if (col.has("nullable") && !col.path("nullable").asBoolean(true)) {
            def.append(" NOT NULL");
        }
        if (col.has("default") && !col.path("default").isNull() && !col.path("default").asText("").isEmpty()) {
            def.append(" DEFAULT ").append(col.path("default").asText());
        }
        return def.toString();
    }

    private Map<String, JsonNode> columnsByName(JsonNode columns) {
        Map<String, JsonNode> result = new LinkedHashMap<String, JsonNode>();
        for (JsonNode column : columns) {
            String name = column.path("name").asText("");
            if (!name.isEmpty()) {
                result.put(name, column);
            }
        }
        return result;
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
        byte[] data = exportBytes(format, query, params.path("options"));
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

    private OscarConfig parseConfig(JsonNode params) {
        String driverId = textOrEmpty(params.path("driver_id"));
        if (!driverId.isEmpty() && !DRIVER_ID.equals(driverId)) {
            throw new IllegalArgumentException("unsupported driver_id `" + driverId + "`");
        }
        Map<String, Object> raw = mapper.convertValue(
            params.path("config"),
            new TypeReference<Map<String, Object>>() {
            }
        );
        return OscarConfig.fromWire(raw);
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
        return name;
    }

    private static String qualifiedIdentifier(String database, String schema, String name) {
        List<String> parts = new ArrayList<String>();
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

    private byte[] exportBytes(String format, QueryResult query, JsonNode options) throws SQLException {
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
            String delimiterValue = optionalText(options, "delimiter", ",");
            String quoteValue = optionalText(options, "quote", "\"");
            String nullString = optionalText(options, "null_string", "\\N");
            String delimiter = delimiterValue.isEmpty() ? "," : delimiterValue.substring(0, 1);
            char delimiterChar = delimiter.charAt(0);
            char quote = quoteValue.isEmpty() ? '"' : quoteValue.charAt(0);
            StringBuilder out = new StringBuilder();
            List<String> names = columnNames(query);
            if (options.path("header").asBoolean(true)) {
                List<String> header = new ArrayList<String>();
                for (String name : names) {
                    header.add(csvCell(name, nullString, delimiterChar, quote));
                }
                out.append(join(header, delimiter)).append('\n');
            }
            for (Map<String, Object> row : rowsAsObjects(query)) {
                List<String> cells = new ArrayList<String>();
                for (String name : names) {
                    cells.add(csvCell(row.get(name), nullString, delimiterChar, quote));
                }
                out.append(join(cells, delimiter)).append('\n');
            }
            return out.toString().getBytes(StandardCharsets.UTF_8);
        }
        throw new SQLException("export format `" + format + "` is not supported");
    }

    static String csvCell(Object value, String nullString, char delimiter, char quote) {
        if (value == null) {
            return nullString;
        }
        String text = String.valueOf(value);
        boolean quoteField = text.isEmpty()
            || text.equals(nullString)
            || text.indexOf(delimiter) >= 0
            || text.indexOf(quote) >= 0
            || text.indexOf('\n') >= 0
            || text.indexOf('\r') >= 0;
        if (!quoteField) {
            return text;
        }
        String quoteText = String.valueOf(quote);
        return quoteText + text.replace(quoteText, quoteText + quoteText) + quoteText;
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

    private List<List<String>> hostRows(QueryResult query) {
        List<List<String>> rows = new ArrayList<List<String>>();
        for (List<Map<String, Object>> row : query.getRows()) {
            List<String> out = new ArrayList<String>();
            for (int i = 0; i < query.getColumns().size(); i++) {
                Object value = rowValue(row, i);
                out.add(value == null ? null : String.valueOf(value));
            }
            rows.add(out);
        }
        return rows;
    }

    private List<Map<String, Object>> hostColumnMeta(QueryResult query) {
        List<Map<String, Object>> out = new ArrayList<Map<String, Object>>();
        for (Map<String, Object> column : query.getColumns()) {
            String name = String.valueOf(column.get("name"));
            String dbType = String.valueOf(column.get("type"));
            Map<String, Object> meta = new LinkedHashMap<String, Object>();
            meta.put("name", name);
            meta.put("db_type", dbType);
            meta.put("field_type", hostFieldType(dbType));
            Object nullable = column.get("nullable");
            meta.put("nullable", nullable instanceof Boolean ? nullable : Boolean.TRUE);
            out.add(meta);
        }
        return out;
    }

    private List<String> columnNames(QueryResult query) {
        List<String> names = new ArrayList<String>();
        for (Map<String, Object> column : query.getColumns()) {
            names.add(String.valueOf(column.get("name")));
        }
        return names;
    }

    private String hostFieldType(String dbType) {
        String upper = dbType == null ? "" : dbType.toUpperCase();
        if (upper.contains("INT") || upper.contains("SERIAL")) {
            return "Integer";
        }
        if (upper.contains("DECIMAL") || upper.contains("NUMERIC") || upper.contains("FLOAT")
            || upper.contains("DOUBLE") || upper.contains("REAL") || upper.contains("MONEY")) {
            return "Decimal";
        }
        if (upper.contains("BOOLEAN") || upper.contains("BOOL")) {
            return "Boolean";
        }
        if (upper.contains("DATE") && !upper.contains("TIME")) {
            return "Date";
        }
        if (upper.contains("TIME") && !upper.contains("DATE")) {
            return "Time";
        }
        if (upper.contains("DATETIME") || upper.contains("TIMESTAMP")) {
            return "DateTime";
        }
        if (upper.contains("BLOB") || upper.contains("BYTE") || upper.contains("BINARY")) {
            return "Binary";
        }
        if (upper.contains("TEXT") || upper.contains("CLOB")) {
            return "LongText";
        }
        if (upper.contains("CHAR") || upper.contains("VARCHAR") || upper.contains("LVARCHAR")) {
            return "Text";
        }
        return "Unknown";
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

    private String emptyIfNull(String value) {
        return value == null ? "" : value;
    }

    private String textOrEmpty(JsonNode node) {
        return node == null || node.isNull() ? "" : node.asText("").trim();
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

    private JsonNode sqlError(JsonNode id, int code, SQLException exception) {
        ObjectNode response = (ObjectNode) error(id, code, sqlErrorMessage(exception));
        ((ObjectNode) response.get("error")).set("data", mapper.valueToTree(sqlErrorData(exception)));
        return response;
    }

    private Map<String, Object> sqlErrorData(SQLException exception) {
        Map<String, Object> data = new LinkedHashMap<String, Object>();
        if (exception.getSQLState() != null && !exception.getSQLState().isEmpty()) {
            data.put("sqlstate", exception.getSQLState());
        }
        data.put("vendor_code", Integer.valueOf(exception.getErrorCode()));

        List<Map<String, Object>> chain = new ArrayList<Map<String, Object>>();
        SQLException current = exception;
        for (int depth = 0; current != null && depth < 32; depth++) {
            Map<String, Object> item = new LinkedHashMap<String, Object>();
            item.put("message", current.getMessage() == null ? "" : current.getMessage());
            if (current.getSQLState() != null && !current.getSQLState().isEmpty()) {
                item.put("sqlstate", current.getSQLState());
            }
            item.put("vendor_code", Integer.valueOf(current.getErrorCode()));
            chain.add(item);
            SQLException next = current.getNextException();
            if (next == current) {
                break;
            }
            current = next;
        }
        Map<String, Object> extra = new LinkedHashMap<String, Object>();
        extra.put("chain", chain);
        data.put("extra", extra);
        return data;
    }

    private String sqlErrorMessage(SQLException exception) {
        StringBuilder message = new StringBuilder();
        SQLException current = exception;
        for (int depth = 0; current != null && depth < 32; depth++) {
            if (message.length() > 0) {
                message.append("\nCaused by: ");
            }
            String currentMessage = current.getMessage();
            message.append(currentMessage == null || currentMessage.isEmpty()
                ? current.getClass().getName()
                : currentMessage);
            if (current.getSQLState() != null && !current.getSQLState().isEmpty()) {
                message.append(" [SQLSTATE ").append(current.getSQLState()).append(']');
            }
            message.append(" [vendor code ").append(current.getErrorCode()).append(']');
            SQLException next = current.getNextException();
            if (next == current) {
                break;
            }
            current = next;
        }
        return message.toString();
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

    private static final class PrimaryKeyInfo {
        private String name = "";
        private final List<String> columns = new ArrayList<String>();

        private boolean matches(String indexName, List<String> indexColumns) {
            if (!name.isEmpty() && name.equalsIgnoreCase(indexName)) {
                return true;
            }
            if (columns.isEmpty() || indexColumns == null || columns.size() != indexColumns.size()) {
                return false;
            }
            for (int i = 0; i < columns.size(); i++) {
                if (!columns.get(i).equalsIgnoreCase(indexColumns.get(i))) {
                    return false;
                }
            }
            return true;
        }
    }

    private static final class ConnectionState {
        private final OscarConfig config;
        private final Connection connection;
        private boolean originalAutoCommit = true;
        private String activeTxId;

        private ConnectionState(OscarConfig config, Connection connection) {
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
