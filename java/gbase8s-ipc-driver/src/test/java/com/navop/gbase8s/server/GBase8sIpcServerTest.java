package com.navop.gbase8s.server;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.navop.gbase8s.jdbc.GBase8sConfig;
import org.junit.Test;

import java.sql.Connection;
import java.sql.DriverManager;
import java.sql.ResultSet;
import java.sql.ResultSetMetaData;
import java.sql.SQLException;
import java.sql.Statement;
import java.lang.reflect.InvocationHandler;
import java.lang.reflect.Method;
import java.lang.reflect.Proxy;
import java.util.concurrent.atomic.AtomicInteger;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

public class GBase8sIpcServerTest {
    private static final AtomicInteger SERVER_DB_COUNTER = new AtomicInteger();
    private final ObjectMapper mapper = new ObjectMapper();

    @Test
    public void csvCellsDistinguishNullEmptyAndLiteralNullMarker() {
        assertEquals("\\N", GBase8sIpcServer.csvCell(null, "\\N", ',', '"'));
        assertEquals("\"\"", GBase8sIpcServer.csvCell("", "\\N", ',', '"'));
        assertEquals("NULL", GBase8sIpcServer.csvCell("NULL", "\\N", ',', '"'));
        assertEquals("\"\\N\"", GBase8sIpcServer.csvCell("\\N", "\\N", ',', '"'));
    }

    @Test
    public void businessMethodsRequireInit() throws Exception {
        GBase8sIpcServer server = newServer();

        JsonNode response = server.handle(request(1, "conn/ping", "{\"conn_id\":1}"));

        assertEquals(-32001, response.get("error").get("code").asInt());
        assertTrue(response.get("error").get("message").asText().contains("init"));
    }

    @Test
    public void initReturnsFeaturesAndUnknownMethodReturnsMethodNotFound() throws Exception {
        GBase8sIpcServer server = newServer();

        JsonNode init = server.handle(request(1, "init", "{\"host_version\":\"1.0.0\",\"api_offered\":{\"database\":\"1.0\"},\"instance_id\":\"test\",\"config\":{}}"));
        assertEquals("0.1.16", init.get("result").get("extension_version").asText());
        assertEquals("gbase8s", init.get("result").get("drivers_ready").get(0).asText());
        assertTrue(init.get("result").get("methods").toString().contains("schema/object_view"));
        assertFalse(init.get("result").get("methods").toString().contains("gbase8s/table_data"));

        JsonNode unknown = server.handle(request(2, "sql/format", "{\"sql\":\"select 1\"}"));
        assertEquals(-32601, unknown.get("error").get("code").asInt());
    }

    @Test
    public void incompatibleInitDoesNotInitializeAndCompatibleRetrySucceeds() throws Exception {
        GBase8sIpcServer server = newServer();

        JsonNode rejected = server.handle(request(1, "init", "{\"host_version\":\"0.9.9\"}"));
        assertEquals(ProtocolError.SERVER_INCOMPATIBLE, rejected.get("error").get("code").asInt());
        assertTrue(rejected.get("error").get("message").asText().contains(">= 0.10.0"));

        JsonNode prerelease = server.handle(request(2, "init", "{\"host_version\":\"0.10.1-alpha.1\"}"));
        assertEquals(ProtocolError.SERVER_INCOMPATIBLE, prerelease.get("error").get("code").asInt());

        JsonNode beforeRetry = server.handle(request(3, "conn/ping", "{\"conn_id\":1}"));
        assertEquals(ProtocolError.NOT_INITIALIZED, beforeRetry.get("error").get("code").asInt());

        JsonNode accepted = server.handle(request(4, "init", "{\"host_version\":\"0.10.0\"}"));
        assertTrue(accepted.toString(), accepted.has("result"));
    }

    @Test
    public void connectionQueryCursorExecAndShutdownFlow() throws Exception {
        GBase8sIpcServer server = newServer();
        server.handle(request(1, "init", "{\"host_version\":\"0.10.0\"}"));

        JsonNode open = server.handle(request(2, "conn/open", "{\"driver_id\":\"gbase8s\",\"config\":" + configJson() + "}"));
        assertTrue(open.toString(), open.has("result"));
        long connId = open.get("result").get("conn_id").asLong();

        JsonNode query = server.handle(request(3, "query/start", "{\"conn_id\":" + connId + ",\"sql\":\"SELECT id, name FROM sample ORDER BY id\",\"max_rows\":2}"));
        String cursorId = query.get("result").get("cursor_id").asText();
        assertEquals(2, query.get("result").get("columns").size());
        assertEquals(2, query.get("result").get("row_count_estimate").asInt());

        JsonNode fetch = server.handle(request(4, "cursor/fetch", "{\"cursor_id\":\"" + cursorId + "\",\"n\":1}"));
        assertEquals(1, fetch.get("result").get("rows").size());
        assertEquals(false, fetch.get("result").get("done").asBoolean());

        JsonNode exec = server.handle(request(5, "exec/run", "{\"conn_id\":" + connId + ",\"sql\":\"UPDATE sample SET name = ? WHERE id = ?\",\"params\":[{\"type\":\"text\",\"value\":\"changed\"},{\"type\":\"i64\",\"value\":2}]}"));
        assertEquals(1, exec.get("result").get("affected_rows").asInt());

        JsonNode closeCursor = server.handle(request(6, "cursor/close", "{\"cursor_id\":\"" + cursorId + "\"}"));
        assertTrue(closeCursor.get("result").isNull());

        JsonNode closeConn = server.handle(request(7, "conn/close", "{\"conn_id\":" + connId + "}"));
        assertTrue(closeConn.get("result").isNull());

        JsonNode shutdown = server.handle(request(8, "shutdown", "{}"));
        assertTrue(shutdown.get("result").isNull());
    }

    @Test
    public void sqlErrorsPreserveJdbcDetailsForSingleAndBatchExecution() throws Exception {
        GBase8sIpcServer server = newServer();
        server.handle(request(1, "init", "{\"host_version\":\"0.10.0\"}"));
        JsonNode open = server.handle(request(2, "conn/open", "{\"driver_id\":\"gbase8s\",\"config\":" + configJson() + "}"));
        long connId = open.get("result").get("conn_id").asLong();

        JsonNode single = server.handle(request(3, "exec/run", "{\"conn_id\":" + connId + ",\"sql\":\"INSERT INTO missing_table VALUES (1)\"}"));
        assertTrue(single.toString(), single.has("error"));
        assertTrue(single.get("error").get("message").asText().contains("missing_table"));
        assertFalse(single.get("error").get("data").get("sqlstate").asText().isEmpty());
        assertTrue(single.get("error").get("data").get("extra").get("chain").size() >= 1);

        JsonNode batch = server.handle(request(4, "exec/batch", "{\"conn_id\":" + connId + ",\"statements\":[\"INSERT INTO missing_table VALUES (1)\"],\"stop_on_error\":true}"));
        JsonNode batchError = batch.get("result").get("errors").get(0);
        assertTrue(batchError.get("message").asText().contains("missing_table"));
        assertFalse(batchError.get("data").get("sqlstate").asText().isEmpty());
        assertTrue(batchError.get("data").get("extra").get("chain").size() >= 1);
    }

    @Test
    public void schemaMethodsReadGBase8sCatalogRows() throws Exception {
        GBase8sIpcServer server = newServer();
        server.handle(request(1, "init", "{\"host_version\":\"0.10.0\"}"));
        JsonNode open = server.handle(request(2, "conn/open", "{\"driver_id\":\"gbase8s\",\"config\":" + configJson() + "}"));
        assertTrue(open.toString(), open.has("result"));
        long connId = open.get("result")
            .get("conn_id")
            .asLong();

        JsonNode schemas = server.handle(request(3, "schema/schemas", "{\"conn_id\":" + connId + ",\"database\":\"stores\"}"));
        assertEquals("gbasedbt", schemas.get("result").get(0).get("name").asText());
        assertEquals("gbasedbt", schemas.get("result").get(0).get("owner").asText());

        JsonNode objects = server.handle(request(4, "schema/objects", "{\"conn_id\":" + connId + ",\"database\":\"stores\",\"schema\":\"gbasedbt\",\"kinds\":[\"table\"]}"));
        assertEquals("stores", objects.get("result").get(0).get("database").asText());
        assertEquals("gbasedbt", objects.get("result").get(0).get("schema").asText());
        assertEquals("sample", objects.get("result").get(0).get("name").asText());
        assertEquals("table", objects.get("result").get(0).get("kind").asText());

        JsonNode views = server.handle(request(5, "schema/views", "{\"conn_id\":" + connId + ",\"database\":\"stores\",\"schema\":\"gbasedbt\"}"));
        assertEquals("stores", views.get("result").get(0).get("database").asText());
        assertEquals("gbasedbt", views.get("result").get(0).get("schema").asText());
        assertEquals("v_sample", views.get("result").get(0).get("name").asText());
        assertEquals("view", views.get("result").get(0).get("kind").asText());
        assertEquals("", views.get("result").get(0).get("definition_sql").asText());

        JsonNode columns = server.handle(request(6, "schema/columns", "{\"conn_id\":" + connId + ",\"database\":\"stores\",\"schema\":\"gbasedbt\",\"table\":\"sample\"}"));
        assertEquals(1, columns.get("result").get(0).get("ordinal").asInt());
        assertEquals("id", columns.get("result").get(0).get("name").asText());
        assertEquals(true, columns.get("result").get(0).get("is_primary").asBoolean());
        assertEquals(false, columns.get("result").get(0).get("nullable").asBoolean());
        assertTrue(columns.get("result").get(0).get("default").isNull());
        assertEquals("abc", columns.get("result").get(1).get("default").asText());

        JsonNode indexes = server.handle(request(7, "schema/indexes", "{\"conn_id\":" + connId + ",\"database\":\"stores\",\"schema\":\"gbasedbt\",\"table\":\"sample\"}"));
        assertEquals("pk_sample", indexes.get("result").get(0).get("name").asText());
        assertEquals("id", indexes.get("result").get(0).get("columns").get(0).asText());
        assertEquals(true, indexes.get("result").get(0).get("is_primary").asBoolean());
        assertEquals(true, indexes.get("result").get(0).get("is_unique").asBoolean());
        JsonNode orderedIndex = findByName(indexes.get("result"), "zz_sample_name_id");
        assertEquals("name", orderedIndex.get("columns").get(0).asText());
        assertEquals("id", orderedIndex.get("columns").get(1).asText());

        JsonNode foreignKeys = server.handle(request(8, "schema/foreign_keys", "{\"conn_id\":" + connId + ",\"database\":\"stores\",\"schema\":\"gbasedbt\",\"table\":\"sample\"}"));
        assertEquals(1, foreignKeys.get("result").size());
        assertEquals("fk_sample_parent", foreignKeys.get("result").get(0).get("name").asText());
        assertEquals("sample", foreignKeys.get("result").get(0).get("from_table").asText());
        assertEquals("id", foreignKeys.get("result").get(0).get("from_columns").get(0).asText());
        assertEquals("parent_sample", foreignKeys.get("result").get(0).get("to_table").asText());
        assertEquals("id", foreignKeys.get("result").get(0).get("to_columns").get(0).asText());

        JsonNode checks = server.handle(request(9, "schema/checks", "{\"conn_id\":" + connId + ",\"database\":\"stores\",\"schema\":\"gbasedbt\",\"table\":\"sample\"}"));
        assertEquals(1, checks.get("result").size());
        assertEquals("ck_sample_name", checks.get("result").get(0).get("name").asText());
        assertEquals("sample", checks.get("result").get(0).get("table").asText());
        assertEquals("name IS NOT NULL", checks.get("result").get(0).get("definition").asText());

        JsonNode functions = server.handle(request(10, "schema/functions", "{\"conn_id\":" + connId + ",\"database\":\"stores\",\"schema\":\"gbasedbt\"}"));
        assertEquals(1, functions.get("result").size());
        assertEquals("demo_add_one", functions.get("result").get(0).get("name").asText());
        assertEquals("gbasedbt", functions.get("result").get(0).get("schema").asText());
        assertEquals("INTEGER", functions.get("result").get(0).get("return_type").asText());
        assertEquals("SPL", functions.get("result").get(0).get("language").asText());

        JsonNode procedures = server.handle(request(11, "schema/procedures", "{\"conn_id\":" + connId + ",\"database\":\"stores\",\"schema\":\"gbasedbt\"}"));
        assertEquals(1, procedures.get("result").size());
        assertEquals("demo_touch_proc", procedures.get("result").get(0).get("name").asText());
        assertEquals("gbasedbt", procedures.get("result").get(0).get("schema").asText());
        assertEquals("SPL", procedures.get("result").get(0).get("language").asText());

        JsonNode columnView = server.handle(request(12, "schema/object_view", "{\"conn_id\":" + connId + ",\"view\":\"columns\",\"database\":\"stores\",\"schema\":\"gbasedbt\",\"table\":\"sample\"}"));
        assertEquals("Columns", columnView.get("result").get("title").asText());
        assertEquals("name", columnView.get("result").get("columns").get(0).get("key").asText());
        assertEquals("Field", columnView.get("result").get("columns").get(0).get("name").asText());
        assertEquals(220, columnView.get("result").get("columns").get(0).get("width_px").asInt());
        assertEquals("id", columnView.get("result").get("rows").get(0).get(0).asText());
        assertEquals("INTEGER", columnView.get("result").get("rows").get(0).get(1).asText());
        assertEquals("", columnView.get("result").get("rows").get(0).get(3).asText());
        assertEquals("abc", columnView.get("result").get("rows").get(1).get(3).asText());

        JsonNode tableView = server.handle(request(13, "schema/object_view", "{\"conn_id\":" + connId + ",\"view\":\"tables\",\"database\":\"stores\",\"schema\":\"gbasedbt\"}"));
        assertEquals("Tables", tableView.get("result").get("title").asText());
        assertEquals("name", tableView.get("result").get("columns").get(0).get("key").asText());
        assertEquals(220, tableView.get("result").get("columns").get(0).get("width_px").asInt());

        JsonNode indexView = server.handle(request(14, "schema/object_view", "{\"conn_id\":" + connId + ",\"view\":\"indexes\",\"database\":\"stores\",\"schema\":\"gbasedbt\",\"table\":\"sample\"}"));
        assertEquals("Indexes", indexView.get("result").get("title").asText());
        assertEquals("pk_sample", indexView.get("result").get("rows").get(0).get(0).asText());
        assertEquals("id", indexView.get("result").get("rows").get(0).get(1).asText());

        JsonNode functionView = server.handle(request(15, "schema/object_view", "{\"conn_id\":" + connId + ",\"view\":\"functions\",\"database\":\"stores\",\"schema\":\"gbasedbt\"}"));
        assertEquals("Functions", functionView.get("result").get("title").asText());
        assertEquals("demo_add_one", functionView.get("result").get("rows").get(0).get(0).asText());
        assertEquals("INTEGER", functionView.get("result").get("rows").get(0).get(1).asText());

        JsonNode procedureView = server.handle(request(16, "schema/object_view", "{\"conn_id\":" + connId + ",\"view\":\"procedures\",\"database\":\"stores\",\"schema\":\"gbasedbt\"}"));
        assertEquals("Procedures", procedureView.get("result").get("title").asText());
        assertEquals("demo_touch_proc", procedureView.get("result").get("rows").get(0).get(0).asText());
    }

    @Test
    public void ddlBuildersUseUnquotedGBaseIdentifiers() throws Exception {
        GBase8sIpcServer server = newServer();
        server.handle(request(1, "init", "{\"host_version\":\"0.10.0\"}"));

        JsonNode create = server.handle(request(
            2,
            "ddl/build_create_table",
            "{\"spec\":{\"schema\":\"testuser\",\"name\":\"probe_table\",\"columns\":[{\"name\":\"id\",\"type\":\"INT\",\"nullable\":false,\"is_primary\":true},{\"name\":\"name\",\"type\":\"VARCHAR(20)\",\"nullable\":true}],\"primary_key\":[\"id\"]},\"options\":{}}"
        ));
        assertEquals(
            "CREATE TABLE testuser.probe_table (id INT NOT NULL, name VARCHAR(20), PRIMARY KEY (id))",
            create.get("result").get("sql").asText()
        );

        JsonNode alter = server.handle(request(
            3,
            "ddl/build_alter_table",
            "{\"from_spec\":{\"schema\":\"testuser\",\"name\":\"probe_table\",\"columns\":[{\"name\":\"id\",\"type\":\"INT\",\"nullable\":false},{\"name\":\"name\",\"type\":\"VARCHAR(20)\",\"nullable\":true}]},\"to_spec\":{\"schema\":\"testuser\",\"name\":\"probe_table\",\"columns\":[{\"name\":\"id\",\"type\":\"INT\",\"nullable\":false},{\"name\":\"name\",\"type\":\"VARCHAR(20)\",\"nullable\":true},{\"name\":\"age\",\"type\":\"INT\",\"nullable\":true}]},\"column_renames\":[],\"options\":{\"with_rollback\":true}}"
        ));
        assertEquals(
            "ALTER TABLE testuser.probe_table ADD age INT",
            alter.get("result").get("statements").get(0).asText()
        );
        assertEquals(
            "ALTER TABLE testuser.probe_table DROP age",
            alter.get("result").get("rollback_statements").get(0).asText()
        );

        JsonNode drop = server.handle(request(
            4,
            "ddl/build_drop",
            "{\"kind\":\"table\",\"database\":\"testdb\",\"schema\":\"testuser\",\"name\":\"probe_table\"}"
        ));
        assertEquals("DROP TABLE testuser.probe_table", drop.get("result").get("sql").asText());
    }

    @Test
    public void schemaDatabasesUsesStatementForCrossDatabaseCatalogSql() throws Exception {
        GBase8sIpcServer server = new GBase8sIpcServer(new JdbcConnectionFactory() {
            @Override
            public Connection open(GBase8sConfig config) {
                return catalogConnection();
            }
        });
        server.handle(request(1, "init", "{\"host_version\":\"0.10.0\"}"));
        long connId = server.handle(request(2, "conn/open", "{\"driver_id\":\"gbase8s\",\"config\":" + configJson() + "}"))
            .get("result")
            .get("conn_id")
            .asLong();

        JsonNode databases = server.handle(request(3, "schema/databases", "{\"conn_id\":" + connId + "}"));

        assertTrue(databases.toString(), databases.has("result"));
        assertEquals("testdb", databases.get("result").get(0).get("name").asText());

        JsonNode databaseView = server.handle(request(4, "schema/object_view", "{\"conn_id\":" + connId + ",\"view\":\"databases\"}"));
        assertEquals("testdb", databaseView.get("result").get("rows").get(0).get(0).asText());
    }

    private GBase8sIpcServer newServer() {
        return new GBase8sIpcServer(new JdbcConnectionFactory() {
            @Override
            public Connection open(GBase8sConfig config) throws Exception {
                Connection connection = DriverManager.getConnection("jdbc:h2:mem:gbase8s_server_" + SERVER_DB_COUNTER.incrementAndGet());
                Statement statement = connection.createStatement();
                statement.execute("CREATE TABLE sample (id BIGINT, name VARCHAR(64))");
                statement.execute("INSERT INTO sample VALUES (1, 'alpha')");
                statement.execute("INSERT INTO sample VALUES (2, 'beta')");
                statement.execute("CREATE SCHEMA gbasedbt");
                statement.execute("CREATE TABLE gbasedbt.sample (id BIGINT NOT NULL, name VARCHAR(64))");
                statement.execute("INSERT INTO gbasedbt.sample VALUES (1, 'alpha')");
                statement.execute("INSERT INTO gbasedbt.sample VALUES (2, 'beta')");
                statement.execute("CREATE TABLE sysusers (username VARCHAR(64))");
                statement.execute("INSERT INTO sysusers VALUES ('gbasedbt')");
                statement.execute("CREATE TABLE systables (tabid INT, tabname VARCHAR(64), owner VARCHAR(64), tabtype CHAR(1))");
                statement.execute("INSERT INTO systables VALUES (99, 'parent_sample', 'gbasedbt   ', 'T')");
                statement.execute("INSERT INTO systables VALUES (100, 'sample', 'gbasedbt   ', 'T')");
                statement.execute("INSERT INTO systables VALUES (101, 'v_sample', 'gbasedbt   ', 'V')");
                statement.execute("INSERT INTO systables VALUES (102, 'sample', 'otheruser  ', 'T')");
                statement.execute("CREATE TABLE syscolumns (tabid INT, colno INT, colname VARCHAR(64), coltype INT)");
                statement.execute("INSERT INTO syscolumns VALUES (99, 1, 'id', 258)");
                statement.execute("INSERT INTO syscolumns VALUES (100, 1, 'id', 258)");
                statement.execute("INSERT INTO syscolumns VALUES (100, 2, 'name', 13)");
                statement.execute("CREATE TABLE sysdefaults (tabid INT, colno INT, type CHAR(1), default VARCHAR(255), class CHAR(1))");
                statement.execute("INSERT INTO sysdefaults VALUES (100, 2, 'L', 'AAAAAw abc', 'T')");
                statement.execute("CREATE TABLE sysconstraints (constrid INT, constrname VARCHAR(64), owner VARCHAR(64), tabid INT, constrtype CHAR(1), idxname VARCHAR(64), collation VARCHAR(64))");
                statement.execute("INSERT INTO sysconstraints VALUES (10, 'pk_parent_sample', 'gbasedbt', 99, 'P', 'pk_parent_sample', '')");
                statement.execute("INSERT INTO sysconstraints VALUES (1, 'pk_sample', 'gbasedbt', 100, 'P', 'pk_sample', '')");
                statement.execute("INSERT INTO sysconstraints VALUES (2, 'fk_sample_parent', 'gbasedbt', 100, 'R', 'zk_sample_parent', '')");
                statement.execute("INSERT INTO sysconstraints VALUES (3, 'ck_sample_name', 'gbasedbt', 100, 'C', NULL, '')");
                statement.execute("CREATE TABLE sysreferences (constrid INT, primary_id INT, ptabid INT, updrule CHAR(1), delrule CHAR(1), matchtype CHAR(1), pendant CHAR(1))");
                statement.execute("INSERT INTO sysreferences VALUES (2, 10, 99, 'R', 'C', 'N', 'N')");
                statement.execute("CREATE TABLE syschecks (constrid INT, type CHAR(1), seqno INT, checktext VARCHAR(255))");
                statement.execute("INSERT INTO syschecks VALUES (3, 'T', 0, 'name IS NOT NULL')");
                statement.execute("CREATE TABLE sysindexes (idxname VARCHAR(64), owner VARCHAR(64), tabid INT, idxtype CHAR(1), clustered CHAR(1), part1 INT, part2 INT, part3 INT, part4 INT, part5 INT, part6 INT, part7 INT, part8 INT, part9 INT, part10 INT, part11 INT, part12 INT, part13 INT, part14 INT, part15 INT, part16 INT)");
                statement.execute("INSERT INTO sysindexes VALUES ('pk_parent_sample', 'gbasedbt', 99, 'U', '', 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)");
                statement.execute("INSERT INTO sysindexes VALUES ('pk_sample', 'gbasedbt', 100, 'U', '', 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)");
                statement.execute("INSERT INTO sysindexes VALUES ('zk_sample_parent', 'gbasedbt', 100, 'D', '', 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)");
                statement.execute("INSERT INTO sysindexes VALUES ('zz_sample_name_id', 'gbasedbt', 100, 'D', '', 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)");
                statement.execute("CREATE TABLE sysprocedures (procname VARCHAR(64), owner VARCHAR(64), procid INT, isproc CHAR(1))");
                statement.execute("INSERT INTO sysprocedures VALUES ('demo_add_one', 'gbasedbt', 200, 'f')");
                statement.execute("INSERT INTO sysprocedures VALUES ('demo_touch_proc', 'gbasedbt', 201, 't')");
                statement.execute("CREATE TABLE sysproccolumns (procid INT, paramid INT, paramname VARCHAR(64), paramtype INT, paramlen INT, paramxid INT, paramattr INT)");
                statement.execute("INSERT INTO sysproccolumns VALUES (200, 0, NULL, 2, 4, 0, 3)");
                statement.execute("INSERT INTO sysproccolumns VALUES (200, 1, 'p', 2, 4, 0, 1)");
                statement.execute("INSERT INTO sysproccolumns VALUES (201, 0, 'p', 2, 4, 0, 1)");
                statement.execute("CREATE TABLE sysprocbody (procid INT, datakey CHAR(1), seqno INT, data VARCHAR(255))");
                statement.execute("INSERT INTO sysprocbody VALUES (200, 'T', 1, 'CREATE FUNCTION demo_add_one(p INT) RETURNING INT; RETURN p + 1; END FUNCTION;')");
                statement.execute("INSERT INTO sysprocbody VALUES (201, 'T', 1, 'CREATE PROCEDURE demo_touch_proc(p INT); UPDATE sample SET name = name WHERE id = p; END PROCEDURE;')");
                statement.close();
                return connection;
            }
        });
    }

    private JsonNode request(int id, String method, String params) throws Exception {
        return mapper.readTree("{\"jsonrpc\":\"2.0\",\"id\":" + id + ",\"method\":\"" + method + "\",\"params\":" + params + "}");
    }

    private JsonNode findByName(JsonNode rows, String name) {
        for (JsonNode row : rows) {
            if (name.equals(row.get("name").asText())) {
                return row;
            }
        }
        throw new AssertionError("missing row named " + name + ": " + rows);
    }

    private static String configJson() {
        return "{\"host\":\"127.0.0.1\",\"username\":\"gbasedbt\",\"password\":\"secret\",\"database\":\"stores\",\"extra_params\":{\"GBASEDBTSERVER\":\"gbase01\",\"PROTOCOL\":\"onsoctcp\"}}";
    }

    private static Connection catalogConnection() {
        return (Connection) Proxy.newProxyInstance(
            GBase8sIpcServerTest.class.getClassLoader(),
            new Class<?>[]{Connection.class},
            new InvocationHandler() {
                @Override
                public Object invoke(Object proxy, Method method, Object[] args) throws Throwable {
                    String name = method.getName();
                    if ("isValid".equals(name)) {
                        return Boolean.TRUE;
                    }
                    if ("createStatement".equals(name)) {
                        return catalogStatement();
                    }
                    if ("prepareStatement".equals(name)) {
                        throw new SQLException("prepared sysmaster catalog query is not supported");
                    }
                    if ("close".equals(name)) {
                        return null;
                    }
                    throw new UnsupportedOperationException(name);
                }
            }
        );
    }

    private static Statement catalogStatement() {
        return (Statement) Proxy.newProxyInstance(
            GBase8sIpcServerTest.class.getClassLoader(),
            new Class<?>[]{Statement.class},
            new InvocationHandler() {
                @Override
                public Object invoke(Object proxy, Method method, Object[] args) throws Throwable {
                    String name = method.getName();
                    if ("executeQuery".equals(name)) {
                        return singleColumnResultSet("testdb    ");
                    }
                    if ("close".equals(name)) {
                        return null;
                    }
                    throw new UnsupportedOperationException(name);
                }
            }
        );
    }

    private static ResultSet singleColumnResultSet(final String value) {
        return (ResultSet) Proxy.newProxyInstance(
            GBase8sIpcServerTest.class.getClassLoader(),
            new Class<?>[]{ResultSet.class},
            new InvocationHandler() {
                private int index = -1;

                @Override
                public Object invoke(Object proxy, Method method, Object[] args) {
                    String name = method.getName();
                    if ("next".equals(name)) {
                        index++;
                        return Boolean.valueOf(index == 0);
                    }
                    if ("getMetaData".equals(name)) {
                        return singleColumnMetaData();
                    }
                    if ("getObject".equals(name)) {
                        return value;
                    }
                    if ("close".equals(name)) {
                        return null;
                    }
                    throw new UnsupportedOperationException(name);
                }
            }
        );
    }

    private static ResultSetMetaData singleColumnMetaData() {
        return (ResultSetMetaData) Proxy.newProxyInstance(
            GBase8sIpcServerTest.class.getClassLoader(),
            new Class<?>[]{ResultSetMetaData.class},
            new InvocationHandler() {
                @Override
                public Object invoke(Object proxy, Method method, Object[] args) {
                    String name = method.getName();
                    if ("getColumnCount".equals(name)) {
                        return Integer.valueOf(1);
                    }
                    if ("getColumnTypeName".equals(name) || "getColumnLabel".equals(name)) {
                        return "name";
                    }
                    if ("getColumnType".equals(name)) {
                        return Integer.valueOf(java.sql.Types.VARCHAR);
                    }
                    if ("isNullable".equals(name)) {
                        return Integer.valueOf(ResultSetMetaData.columnNullable);
                    }
                    throw new UnsupportedOperationException(name);
                }
            }
        );
    }
}
