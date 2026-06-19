package com.onetcli.gbase8s.server;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.onetcli.gbase8s.jdbc.GBase8sConfig;
import org.junit.Test;

import java.sql.Connection;
import java.sql.DriverManager;
import java.sql.Statement;
import java.util.concurrent.atomic.AtomicInteger;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

public class GBase8sIpcServerTest {
    private final ObjectMapper mapper = new ObjectMapper();

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
        assertEquals("0.1.0", init.get("result").get("extension_version").asText());
        assertEquals("gbase8s", init.get("result").get("drivers_ready").get(0).asText());
        assertTrue(init.get("result").get("methods").toString().contains("schema/object_view"));

        JsonNode unknown = server.handle(request(2, "sql/format", "{\"sql\":\"select 1\"}"));
        assertEquals(-32601, unknown.get("error").get("code").asInt());
    }

    @Test
    public void connectionQueryCursorExecAndShutdownFlow() throws Exception {
        GBase8sIpcServer server = newServer();
        server.handle(request(1, "init", "{}"));

        JsonNode open = server.handle(request(2, "conn/open", "{\"driver_id\":\"gbase8s\",\"config\":" + configJson() + "}"));
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
    public void schemaMethodsReadGBase8sCatalogRows() throws Exception {
        GBase8sIpcServer server = newServer();
        server.handle(request(1, "init", "{}"));
        long connId = server.handle(request(2, "conn/open", "{\"driver_id\":\"gbase8s\",\"config\":" + configJson() + "}"))
            .get("result")
            .get("conn_id")
            .asLong();

        JsonNode schemas = server.handle(request(3, "schema/schemas", "{\"conn_id\":" + connId + ",\"database\":\"stores\"}"));
        assertEquals("gbasedbt", schemas.get("result").get(0).get("name").asText());
        assertEquals("gbasedbt", schemas.get("result").get(0).get("owner").asText());

        JsonNode objects = server.handle(request(4, "schema/objects", "{\"conn_id\":" + connId + ",\"database\":\"stores\",\"schema\":\"gbasedbt\",\"kinds\":[\"table\"]}"));
        assertEquals("sample", objects.get("result").get(0).get("name").asText());
        assertEquals("table", objects.get("result").get(0).get("kind").asText());

        JsonNode views = server.handle(request(5, "schema/views", "{\"conn_id\":" + connId + ",\"database\":\"stores\",\"schema\":\"gbasedbt\"}"));
        assertEquals("v_sample", views.get("result").get(0).get("name").asText());
        assertEquals("view", views.get("result").get(0).get("kind").asText());
        assertEquals("", views.get("result").get(0).get("definition_sql").asText());

        JsonNode columns = server.handle(request(6, "schema/columns", "{\"conn_id\":" + connId + ",\"database\":\"stores\",\"schema\":\"gbasedbt\",\"table\":\"sample\"}"));
        assertEquals(1, columns.get("result").get(0).get("ordinal").asInt());
        assertEquals("id", columns.get("result").get(0).get("name").asText());
        assertEquals(false, columns.get("result").get(0).get("nullable").asBoolean());

        JsonNode columnView = server.handle(request(7, "schema/object_view", "{\"conn_id\":" + connId + ",\"view\":\"columns\",\"database\":\"stores\",\"schema\":\"gbasedbt\",\"table\":\"sample\"}"));
        assertEquals("Columns", columnView.get("result").get("title").asText());
        assertEquals("name", columnView.get("result").get("columns").get(0).get("key").asText());
        assertEquals("Field", columnView.get("result").get("columns").get(0).get("name").asText());
        assertEquals(220, columnView.get("result").get("columns").get(0).get("width_px").asInt());
        assertEquals("id", columnView.get("result").get("rows").get(0).get(0).asText());
        assertEquals("INTEGER", columnView.get("result").get("rows").get(0).get(1).asText());

        JsonNode tableView = server.handle(request(8, "schema/object_view", "{\"conn_id\":" + connId + ",\"view\":\"tables\",\"database\":\"stores\",\"schema\":\"gbasedbt\"}"));
        assertEquals("Tables", tableView.get("result").get("title").asText());
        assertEquals("name", tableView.get("result").get("columns").get(0).get("key").asText());
        assertEquals(220, tableView.get("result").get("columns").get(0).get("width_px").asInt());
    }

    private GBase8sIpcServer newServer() {
        final AtomicInteger counter = new AtomicInteger();
        return new GBase8sIpcServer(new JdbcConnectionFactory() {
            @Override
            public Connection open(GBase8sConfig config) throws Exception {
                Connection connection = DriverManager.getConnection("jdbc:h2:mem:gbase8s_server_" + counter.incrementAndGet());
                Statement statement = connection.createStatement();
                statement.execute("CREATE TABLE sample (id BIGINT PRIMARY KEY, name VARCHAR(64))");
                statement.execute("INSERT INTO sample VALUES (1, 'alpha')");
                statement.execute("INSERT INTO sample VALUES (2, 'beta')");
                statement.execute("CREATE TABLE sysusers (username VARCHAR(64))");
                statement.execute("INSERT INTO sysusers VALUES ('gbasedbt')");
                statement.execute("CREATE TABLE systables (tabid INT, tabname VARCHAR(64), tabtype CHAR(1))");
                statement.execute("INSERT INTO systables VALUES (100, 'sample', 'T')");
                statement.execute("INSERT INTO systables VALUES (101, 'v_sample', 'V')");
                statement.execute("CREATE TABLE syscolumns (tabid INT, colno INT, colname VARCHAR(64), coltype INT)");
                statement.execute("INSERT INTO syscolumns VALUES (100, 0, 'id', 258)");
                statement.execute("INSERT INTO syscolumns VALUES (100, 1, 'name', 13)");
                statement.close();
                return connection;
            }
        });
    }

    private JsonNode request(int id, String method, String params) throws Exception {
        return mapper.readTree("{\"jsonrpc\":\"2.0\",\"id\":" + id + ",\"method\":\"" + method + "\",\"params\":" + params + "}");
    }

    private static String configJson() {
        return "{\"host\":\"127.0.0.1\",\"username\":\"gbasedbt\",\"password\":\"secret\",\"database\":\"stores\",\"extra_params\":{\"GBASEDBTSERVER\":\"gbase01\",\"PROTOCOL\":\"onsoctcp\"}}";
    }
}
