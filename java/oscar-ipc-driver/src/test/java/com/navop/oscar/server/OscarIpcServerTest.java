package com.navop.oscar.server;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.navop.oscar.jdbc.OscarConfig;
import org.junit.Test;

import java.sql.Connection;
import java.sql.DriverManager;
import java.sql.SQLException;
import java.sql.Statement;
import java.lang.reflect.InvocationHandler;
import java.lang.reflect.Method;
import java.lang.reflect.Proxy;
import java.util.concurrent.atomic.AtomicInteger;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

public class OscarIpcServerTest {
    private static final AtomicInteger SERVER_DB_COUNTER = new AtomicInteger();
    private final ObjectMapper mapper = new ObjectMapper();

    @Test
    public void csvCellsDistinguishNullEmptyAndLiteralNullMarker() {
        assertEquals("\\N", OscarIpcServer.csvCell(null, "\\N", ',', '"'));
        assertEquals("\"\"", OscarIpcServer.csvCell("", "\\N", ',', '"'));
        assertEquals("NULL", OscarIpcServer.csvCell("NULL", "\\N", ',', '"'));
        assertEquals("\"\\N\"", OscarIpcServer.csvCell("\\N", "\\N", ',', '"'));
    }

    @Test
    public void businessMethodsRequireInit() throws Exception {
        OscarIpcServer server = newServer();

        JsonNode response = server.handle(request(1, "conn/ping", "{\"conn_id\":1}"));

        assertEquals(-32001, response.get("error").get("code").asInt());
        assertTrue(response.get("error").get("message").asText().contains("init"));
    }

    @Test
    public void initReturnsFeaturesAndUnknownMethodReturnsMethodNotFound() throws Exception {
        OscarIpcServer server = newServer();

        JsonNode init = server.handle(request(1, "init", "{\"host_version\":\"1.0.0\",\"api_offered\":{\"database\":\"1.0\"},\"instance_id\":\"test\",\"config\":{}}"));
        assertEquals("0.1.4", init.get("result").get("extension_version").asText());
        assertEquals("oscar", init.get("result").get("drivers_ready").get(0).asText());
        assertTrue(init.get("result").get("methods").toString().contains("schema/object_view"));
        assertFalse(init.get("result").get("methods").toString().contains("oscar/table_data"));

        JsonNode unknown = server.handle(request(2, "sql/format", "{\"sql\":\"select 1\"}"));
        assertEquals(-32601, unknown.get("error").get("code").asInt());
    }

    @Test
    public void incompatibleInitDoesNotInitializeAndCompatibleRetrySucceeds() throws Exception {
        OscarIpcServer server = newServer();

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
        OscarIpcServer server = newServer();
        server.handle(request(1, "init", "{\"host_version\":\"0.10.0\"}"));

        JsonNode open = server.handle(request(2, "conn/open", "{\"driver_id\":\"oscar\",\"config\":" + configJson() + "}"));
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
        OscarIpcServer server = newServer();
        server.handle(request(1, "init", "{\"host_version\":\"0.10.0\"}"));
        JsonNode open = server.handle(request(2, "conn/open", "{\"driver_id\":\"oscar\",\"config\":" + configJson() + "}"));
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
    public void schemaDumpDdlEmitsFullTableStructure() throws Exception {
        OscarIpcServer server = newServer();
        server.handle(request(1, "init", "{\"host_version\":\"0.10.0\"}"));
        JsonNode open = server.handle(request(2, "conn/open", "{\"driver_id\":\"oscar\",\"config\":" + configJson() + "}"));
        long connId = open.get("result").get("conn_id").asLong();

        JsonNode dump = server.handle(request(3, "schema/dump_ddl", "{\"conn_id\":" + connId + ",\"objects\":[{\"kind\":\"table\",\"name\":\"sample\",\"schema\":\"SYSDBA\",\"database\":\"OSRDB\"}],\"options\":{}}"));

        assertTrue(dump.toString(), dump.has("result"));
        JsonNode statements = dump.get("result").get("statements");
        assertEquals(
            "CREATE TABLE SYSDBA.sample (id BIGINT NOT NULL, name VARCHAR(64) DEFAULT 'abc', ts TIMESTAMP(6), amount DECIMAL(12,2), PRIMARY KEY (id))",
            statements.get(0).asText()
        );
        assertEquals("CREATE INDEX idx_sample_name ON SYSDBA.sample (name)", statements.get(1).asText());
        assertEquals("CREATE INDEX zz_sample_name_id ON SYSDBA.sample (name, id)", statements.get(2).asText());
        assertEquals(
            "ALTER TABLE SYSDBA.sample ADD CONSTRAINT fk_sample_parent FOREIGN KEY (id) REFERENCES SYSDBA.parent_sample (id) ON UPDATE RESTRICT ON DELETE RESTRICT",
            statements.get(3).asText()
        );

        // View/sequence objects fall through to the host shared builder: no DDL here.
        JsonNode viewDump = server.handle(request(4, "schema/dump_ddl", "{\"conn_id\":" + connId + ",\"objects\":[{\"kind\":\"view\",\"name\":\"v_sample\",\"schema\":\"SYSDBA\",\"database\":\"OSRDB\"}],\"options\":{}}"));
        assertEquals(0, viewDump.get("result").get("statements").size());

        // Without a table object the handler returns an empty statement list.
        JsonNode empty = server.handle(request(5, "schema/dump_ddl", "{\"conn_id\":" + connId + ",\"objects\":[],\"options\":{}}"));
        assertEquals(0, empty.get("result").get("statements").size());
    }

    @Test
    public void schemaMethodsReadJdbcMetadataRows() throws Exception {
        OscarIpcServer server = newServer();
        server.handle(request(1, "init", "{\"host_version\":\"0.10.0\"}"));
        JsonNode open = server.handle(request(2, "conn/open", "{\"driver_id\":\"oscar\",\"config\":" + configJson() + "}"));
        assertTrue(open.toString(), open.has("result"));
        long connId = open.get("result")
            .get("conn_id")
            .asLong();

        JsonNode schemas = server.handle(request(3, "schema/schemas", "{\"conn_id\":" + connId + ",\"database\":\"OSRDB\"}"));
        JsonNode sysdbaSchema = findByName(schemas.get("result"), "SYSDBA");
        assertEquals("SYSDBA", sysdbaSchema.get("name").asText());
        assertEquals("SYSDBA", sysdbaSchema.get("owner").asText());

        JsonNode objects = server.handle(request(4, "schema/objects", "{\"conn_id\":" + connId + ",\"database\":\"OSRDB\",\"schema\":\"SYSDBA\",\"kinds\":[\"table\"]}"));
        JsonNode sampleObject = findByName(objects.get("result"), "sample");
        assertEquals("OSRDB", sampleObject.get("database").asText());
        assertEquals("SYSDBA", sampleObject.get("schema").asText());
        assertEquals("sample", sampleObject.get("name").asText());
        assertEquals("table", sampleObject.get("kind").asText());

        JsonNode views = server.handle(request(5, "schema/views", "{\"conn_id\":" + connId + ",\"database\":\"OSRDB\",\"schema\":\"SYSDBA\"}"));
        assertEquals("OSRDB", views.get("result").get(0).get("database").asText());
        assertEquals("SYSDBA", views.get("result").get(0).get("schema").asText());
        assertEquals("v_sample", views.get("result").get(0).get("name").asText());
        assertEquals("view", views.get("result").get(0).get("kind").asText());
        assertEquals("", views.get("result").get(0).get("definition_sql").asText());

        JsonNode columns = server.handle(request(6, "schema/columns", "{\"conn_id\":" + connId + ",\"database\":\"OSRDB\",\"schema\":\"SYSDBA\",\"table\":\"sample\"}"));
        assertEquals(1, columns.get("result").get(0).get("ordinal").asInt());
        assertEquals("id", columns.get("result").get(0).get("name").asText());
        assertEquals(true, columns.get("result").get(0).get("is_primary").asBoolean());
        assertEquals(false, columns.get("result").get(0).get("nullable").asBoolean());
        assertTrue(columns.get("result").get(0).get("default").isNull());
        assertEquals("abc", columns.get("result").get(1).get("default").asText());
        assertEquals("BIGINT", columns.get("result").get(0).get("raw_type").asText());
        assertEquals("VARCHAR(64)", columns.get("result").get(1).get("raw_type").asText());

        JsonNode indexes = server.handle(request(7, "schema/indexes", "{\"conn_id\":" + connId + ",\"database\":\"OSRDB\",\"schema\":\"SYSDBA\",\"table\":\"sample\"}"));
        JsonNode primaryIndex = findPrimaryIndex(indexes.get("result"));
        assertEquals("id", primaryIndex.get("columns").get(0).asText());
        assertEquals(true, primaryIndex.get("is_primary").asBoolean());
        assertEquals(true, primaryIndex.get("is_unique").asBoolean());
        JsonNode orderedIndex = findByName(indexes.get("result"), "zz_sample_name_id");
        assertEquals("name", orderedIndex.get("columns").get(0).asText());
        assertEquals("id", orderedIndex.get("columns").get(1).asText());

        JsonNode foreignKeys = server.handle(request(8, "schema/foreign_keys", "{\"conn_id\":" + connId + ",\"database\":\"OSRDB\",\"schema\":\"SYSDBA\",\"table\":\"sample\"}"));
        assertEquals(1, foreignKeys.get("result").size());
        assertEquals("fk_sample_parent", foreignKeys.get("result").get(0).get("name").asText());
        assertEquals("sample", foreignKeys.get("result").get(0).get("from_table").asText());
        assertEquals("id", foreignKeys.get("result").get(0).get("from_columns").get(0).asText());
        assertEquals("parent_sample", foreignKeys.get("result").get(0).get("to_table").asText());
        assertEquals("id", foreignKeys.get("result").get(0).get("to_columns").get(0).asText());

        JsonNode checks = server.handle(request(9, "schema/checks", "{\"conn_id\":" + connId + ",\"database\":\"OSRDB\",\"schema\":\"SYSDBA\",\"table\":\"sample\"}"));
        assertEquals(0, checks.get("result").size());

        JsonNode functions = server.handle(request(10, "schema/functions", "{\"conn_id\":" + connId + ",\"database\":\"OSRDB\",\"schema\":\"SYSDBA\"}"));
        assertTrue(functions.toString(), functions.has("result"));

        JsonNode procedures = server.handle(request(11, "schema/procedures", "{\"conn_id\":" + connId + ",\"database\":\"OSRDB\",\"schema\":\"SYSDBA\"}"));
        assertTrue(procedures.toString(), procedures.has("result"));

        JsonNode columnView = server.handle(request(12, "schema/object_view", "{\"conn_id\":" + connId + ",\"view\":\"columns\",\"database\":\"OSRDB\",\"schema\":\"SYSDBA\",\"table\":\"sample\"}"));
        assertEquals("Columns", columnView.get("result").get("title").asText());
        assertEquals("name", columnView.get("result").get("columns").get(0).get("key").asText());
        assertEquals("Field", columnView.get("result").get("columns").get(0).get("name").asText());
        assertEquals(220, columnView.get("result").get("columns").get(0).get("width_px").asInt());
        assertEquals("id", columnView.get("result").get("rows").get(0).get(0).asText());
        assertEquals("BIGINT", columnView.get("result").get("rows").get(0).get(1).asText());
        assertEquals("", columnView.get("result").get("rows").get(0).get(3).asText());
        assertEquals("abc", columnView.get("result").get("rows").get(1).get(3).asText());

        JsonNode tableView = server.handle(request(13, "schema/object_view", "{\"conn_id\":" + connId + ",\"view\":\"tables\",\"database\":\"OSRDB\",\"schema\":\"SYSDBA\"}"));
        assertEquals("Tables", tableView.get("result").get("title").asText());
        assertEquals("name", tableView.get("result").get("columns").get(0).get("key").asText());
        assertEquals(220, tableView.get("result").get("columns").get(0).get("width_px").asInt());
        assertEquals("sample", findObjectViewRow(tableView.get("result").get("rows"), "sample").get(0).asText());

        JsonNode indexView = server.handle(request(14, "schema/object_view", "{\"conn_id\":" + connId + ",\"view\":\"indexes\",\"database\":\"OSRDB\",\"schema\":\"SYSDBA\",\"table\":\"sample\"}"));
        assertEquals("Indexes", indexView.get("result").get("title").asText());
        assertTrue(indexView.toString(), indexView.get("result").get("rows").isArray());

        JsonNode functionView = server.handle(request(15, "schema/object_view", "{\"conn_id\":" + connId + ",\"view\":\"functions\",\"database\":\"OSRDB\",\"schema\":\"SYSDBA\"}"));
        assertEquals("Functions", functionView.get("result").get("title").asText());
        assertTrue(functionView.toString(), functionView.get("result").get("rows").isArray());

        JsonNode procedureView = server.handle(request(16, "schema/object_view", "{\"conn_id\":" + connId + ",\"view\":\"procedures\",\"database\":\"OSRDB\",\"schema\":\"SYSDBA\"}"));
        assertEquals("Procedures", procedureView.get("result").get("title").asText());
        assertTrue(procedureView.toString(), procedureView.get("result").get("rows").isArray());
    }

    @Test
    public void metadataOnlySchemaMethodsDoNotRequireOscarSysCatalogTables() throws Exception {
        OscarIpcServer server = metadataOnlyServer();
        server.handle(request(1, "init", "{\"host_version\":\"0.10.0\"}"));
        JsonNode open = server.handle(request(2, "conn/open", "{\"driver_id\":\"oscar\",\"config\":" + configJson() + "}"));
        assertTrue(open.toString(), open.has("result"));
        long connId = open.get("result").get("conn_id").asLong();

        JsonNode schemas = server.handle(request(3, "schema/schemas", "{\"conn_id\":" + connId + ",\"database\":\"OSRDB\"}"));
        assertTrue(schemas.toString(), schemas.has("result"));
        assertEquals("SYSDBA", findByName(schemas.get("result"), "SYSDBA").get("name").asText());

        JsonNode objects = server.handle(request(4, "schema/objects", "{\"conn_id\":" + connId + ",\"database\":\"OSRDB\",\"schema\":\"SYSDBA\",\"kinds\":[\"table\"]}"));
        assertTrue(objects.toString(), objects.has("result"));
        JsonNode sampleObject = findByName(objects.get("result"), "sample");
        assertEquals("sample", sampleObject.get("name").asText());
        assertEquals("table", sampleObject.get("kind").asText());

        JsonNode columns = server.handle(request(5, "schema/columns", "{\"conn_id\":" + connId + ",\"database\":\"OSRDB\",\"schema\":\"SYSDBA\",\"table\":\"sample\"}"));
        assertTrue(columns.toString(), columns.has("result"));
        assertEquals("id", columns.get("result").get(0).get("name").asText());
        assertEquals(true, columns.get("result").get(0).get("is_primary").asBoolean());
        assertEquals(false, columns.get("result").get(0).get("nullable").asBoolean());
        assertEquals("name", columns.get("result").get(1).get("name").asText());
        assertEquals("abc", columns.get("result").get(1).get("default").asText());

        JsonNode indexes = server.handle(request(6, "schema/indexes", "{\"conn_id\":" + connId + ",\"database\":\"OSRDB\",\"schema\":\"SYSDBA\",\"table\":\"sample\"}"));
        assertTrue(indexes.toString(), indexes.has("result"));
        JsonNode sampleNameIndex = findByName(indexes.get("result"), "idx_sample_name");
        assertEquals("name", sampleNameIndex.get("columns").get(0).asText());

        JsonNode foreignKeys = server.handle(request(7, "schema/foreign_keys", "{\"conn_id\":" + connId + ",\"database\":\"OSRDB\",\"schema\":\"SYSDBA\",\"table\":\"sample\"}"));
        assertTrue(foreignKeys.toString(), foreignKeys.has("result"));
        assertEquals("fk_sample_parent", foreignKeys.get("result").get(0).get("name").asText());
        assertEquals("parent_sample", foreignKeys.get("result").get(0).get("to_table").asText());

        JsonNode views = server.handle(request(8, "schema/views", "{\"conn_id\":" + connId + ",\"database\":\"OSRDB\",\"schema\":\"SYSDBA\"}"));
        assertTrue(views.toString(), views.has("result"));
        assertEquals("v_sample", views.get("result").get(0).get("name").asText());

        JsonNode tableView = server.handle(request(9, "schema/object_view", "{\"conn_id\":" + connId + ",\"view\":\"tables\",\"database\":\"OSRDB\",\"schema\":\"SYSDBA\"}"));
        assertTrue(tableView.toString(), tableView.has("result"));
        assertEquals("sample", findObjectViewRow(tableView.get("result").get("rows"), "sample").get(0).asText());
    }

    @Test
    public void ddlBuildersReturnOscarSql() throws Exception {
        OscarIpcServer server = newServer();
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
    public void ddlBuildCreateAndDropDatabase() throws Exception {
        OscarIpcServer server = newServer();
        server.handle(request(1, "init", "{\"host_version\":\"0.10.0\"}"));

        JsonNode create = server.handle(request(
            2,
            "ddl/build",
            "{\"op\":\"create_database\",\"payload\":{\"database_name\":\"sales\",\"field_values\":{}}}"
        ));
        assertEquals("CREATE DATABASE sales", create.get("result").get("sql").asText());
        assertEquals("CREATE DATABASE sales", create.get("result").get("statements").get(0).asText());

        JsonNode drop = server.handle(request(
            3,
            "ddl/build",
            "{\"op\":\"drop_database\",\"payload\":{\"name\":\"sales\",\"database_name\":\"sales\"}}"
        ));
        assertEquals("DROP DATABASE sales", drop.get("result").get("sql").asText());
        assertEquals("DROP DATABASE sales", drop.get("result").get("statements").get(0).asText());
    }

    @Test
    public void ddlBuildCreateTableEmitsTableAndColumnComments() throws Exception {
        OscarIpcServer server = newServer();
        server.handle(request(1, "init", "{\"host_version\":\"0.10.0\"}"));

        JsonNode create = server.handle(request(
            2,
            "ddl/build_create_table",
            "{\"spec\":{\"schema\":\"testuser\",\"name\":\"commented_table\",\"comment\":\"用户表\",\"columns\":[{\"name\":\"id\",\"type\":\"INT\",\"nullable\":false,\"comment\":\"主键\"},{\"name\":\"name\",\"type\":\"VARCHAR(20)\",\"nullable\":true,\"comment\":\"it's a name\"},{\"name\":\"age\",\"type\":\"INT\",\"nullable\":true}]},\"options\":{}}"
        ));
        assertEquals(
            "CREATE TABLE testuser.commented_table (id INT NOT NULL, name VARCHAR(20), age INT)",
            create.get("result").get("sql").asText()
        );
        assertEquals(4, create.get("result").get("statements").size());
        assertEquals(
            "COMMENT ON TABLE testuser.commented_table IS '用户表'",
            create.get("result").get("statements").get(1).asText()
        );
        assertEquals(
            "COMMENT ON COLUMN testuser.commented_table.id IS '主键'",
            create.get("result").get("statements").get(2).asText()
        );
        assertEquals(
            "COMMENT ON COLUMN testuser.commented_table.name IS 'it''s a name'",
            create.get("result").get("statements").get(3).asText()
        );
    }

    @Test
    public void ddlBuildCreateTableSkipsCommentsWhenDisabled() throws Exception {
        OscarIpcServer server = newServer();
        server.handle(request(1, "init", "{\"host_version\":\"0.10.0\"}"));

        JsonNode create = server.handle(request(
            2,
            "ddl/build_create_table",
            "{\"spec\":{\"schema\":\"testuser\",\"name\":\"commented_table\",\"comment\":\"用户表\",\"columns\":[{\"name\":\"id\",\"type\":\"INT\",\"comment\":\"主键\"}]},\"options\":{\"with_comments\":false}}"
        ));
        assertEquals(1, create.get("result").get("statements").size());
        assertEquals(
            "CREATE TABLE testuser.commented_table (id INT)",
            create.get("result").get("statements").get(0).asText()
        );
    }

    @Test
    public void ddlBuildAlterTableEmitsCommentChanges() throws Exception {
        OscarIpcServer server = newServer();
        server.handle(request(1, "init", "{\"host_version\":\"0.10.0\"}"));

        JsonNode alter = server.handle(request(
            2,
            "ddl/build_alter_table",
            "{\"from_spec\":{\"schema\":\"testuser\",\"name\":\"probe_table\",\"comment\":\"old comment\",\"columns\":[{\"name\":\"id\",\"type\":\"INT\",\"nullable\":false,\"comment\":\"旧主键\"},{\"name\":\"name\",\"type\":\"VARCHAR(20)\",\"nullable\":true,\"comment\":\"unchanged\"}]},\"to_spec\":{\"schema\":\"testuser\",\"name\":\"probe_table\",\"comment\":\"new comment\",\"columns\":[{\"name\":\"id\",\"type\":\"INT\",\"nullable\":false,\"comment\":\"新主键\"},{\"name\":\"name\",\"type\":\"VARCHAR(20)\",\"nullable\":true,\"comment\":\"unchanged\"}]},\"column_renames\":[],\"options\":{\"with_rollback\":true}}"
        ));
        JsonNode statements = alter.get("result").get("statements");
        assertEquals(2, statements.size());
        assertEquals(
            "COMMENT ON TABLE testuser.probe_table IS 'new comment'",
            statements.get(0).asText()
        );
        assertEquals(
            "COMMENT ON COLUMN testuser.probe_table.id IS '新主键'",
            statements.get(1).asText()
        );
        JsonNode rollback = alter.get("result").get("rollback_statements");
        assertEquals(2, rollback.size());
        assertEquals(
            "COMMENT ON COLUMN testuser.probe_table.id IS '旧主键'",
            rollback.get(0).asText()
        );
        assertEquals(
            "COMMENT ON TABLE testuser.probe_table IS 'old comment'",
            rollback.get(1).asText()
        );
    }

    @Test
    public void ddlBuildAlterTableEmitsCommentClearAndSkipsUnchanged() throws Exception {
        OscarIpcServer server = newServer();
        server.handle(request(1, "init", "{\"host_version\":\"0.10.0\"}"));

        JsonNode alter = server.handle(request(
            2,
            "ddl/build_alter_table",
            "{\"from_spec\":{\"schema\":\"testuser\",\"name\":\"probe_table\",\"comment\":\"remove me\",\"columns\":[{\"name\":\"id\",\"type\":\"INT\",\"comment\":\"keep\"}]},\"to_spec\":{\"schema\":\"testuser\",\"name\":\"probe_table\",\"comment\":\"\",\"columns\":[{\"name\":\"id\",\"type\":\"INT\",\"comment\":\"keep\"}]},\"column_renames\":[],\"options\":{}}"
        ));
        JsonNode statements = alter.get("result").get("statements");
        assertEquals(1, statements.size());
        assertEquals(
            "COMMENT ON TABLE testuser.probe_table IS ''",
            statements.get(0).asText()
        );
        assertFalse(alter.get("result").get("rollback_statements").has(0));
    }

    @Test
    public void ddlBuildAlterTableEmitsCommentForNewlyAddedColumn() throws Exception {
        OscarIpcServer server = newServer();
        server.handle(request(1, "init", "{\"host_version\":\"0.10.0\"}"));

        JsonNode alter = server.handle(request(
            2,
            "ddl/build_alter_table",
            "{\"from_spec\":{\"schema\":\"testuser\",\"name\":\"probe_table\",\"columns\":[{\"name\":\"id\",\"type\":\"INT\"}]},\"to_spec\":{\"schema\":\"testuser\",\"name\":\"probe_table\",\"columns\":[{\"name\":\"id\",\"type\":\"INT\"},{\"name\":\"new_col\",\"type\":\"VARCHAR(20)\",\"comment\":\"新列注释\"}]},\"column_renames\":[],\"options\":{\"with_rollback\":true}}"
        ));
        JsonNode statements = alter.get("result").get("statements");
        assertEquals(2, statements.size());
        assertEquals(
            "ALTER TABLE testuser.probe_table ADD new_col VARCHAR(20)",
            statements.get(0).asText()
        );
        assertEquals(
            "COMMENT ON COLUMN testuser.probe_table.new_col IS '新列注释'",
            statements.get(1).asText()
        );
        JsonNode rollback = alter.get("result").get("rollback_statements");
        assertEquals(1, rollback.size());
        assertEquals(
            "ALTER TABLE testuser.probe_table DROP new_col",
            rollback.get(0).asText()
        );
    }

    @Test
    public void schemaDatabasesUsesConnectionConfigWithoutCatalogSql() throws Exception {
        OscarIpcServer server = new OscarIpcServer(new JdbcConnectionFactory() {
            @Override
            public Connection open(OscarConfig config) {
                return catalogConnection();
            }
        });
        server.handle(request(1, "init", "{\"host_version\":\"0.10.0\"}"));
        long connId = server.handle(request(2, "conn/open", "{\"driver_id\":\"oscar\",\"config\":" + configJson() + "}"))
            .get("result")
            .get("conn_id")
            .asLong();

        JsonNode databases = server.handle(request(3, "schema/databases", "{\"conn_id\":" + connId + "}"));

        assertTrue(databases.toString(), databases.has("result"));
        assertEquals("OSRDB", databases.get("result").get(0).get("name").asText());

        JsonNode databaseView = server.handle(request(4, "schema/object_view", "{\"conn_id\":" + connId + ",\"view\":\"databases\"}"));
        assertEquals("OSRDB", databaseView.get("result").get("rows").get(0).get(0).asText());
    }

    private OscarIpcServer newServer() {
        return new OscarIpcServer(new JdbcConnectionFactory() {
            @Override
            public Connection open(OscarConfig config) throws Exception {
                Connection connection = DriverManager.getConnection("jdbc:h2:mem:oscar_server_" + SERVER_DB_COUNTER.incrementAndGet() + ";DATABASE_TO_UPPER=FALSE");
                Statement statement = connection.createStatement();
                statement.execute("CREATE TABLE sample (id BIGINT, name VARCHAR(64))");
                statement.execute("INSERT INTO sample VALUES (1, 'alpha')");
                statement.execute("INSERT INTO sample VALUES (2, 'beta')");
                statement.execute("CREATE SCHEMA SYSDBA");
                statement.execute("CREATE TABLE SYSDBA.parent_sample (id BIGINT PRIMARY KEY)");
                statement.execute("CREATE TABLE SYSDBA.sample (id BIGINT NOT NULL, name VARCHAR(64) DEFAULT 'abc', ts TIMESTAMP(6), amount DECIMAL(12,2), CONSTRAINT pk_sample PRIMARY KEY (id), CONSTRAINT fk_sample_parent FOREIGN KEY (id) REFERENCES SYSDBA.parent_sample(id))");
                statement.execute("INSERT INTO SYSDBA.parent_sample VALUES (1)");
                statement.execute("INSERT INTO SYSDBA.parent_sample VALUES (2)");
                statement.execute("INSERT INTO SYSDBA.sample VALUES (1, 'alpha', TIMESTAMP '2024-01-01 12:00:00.123456', 123.45)");
                statement.execute("INSERT INTO SYSDBA.sample VALUES (2, 'beta', TIMESTAMP '2024-02-01 00:00:00.000000', 678.90)");
                statement.execute("CREATE INDEX idx_sample_name ON SYSDBA.sample(name)");
                statement.execute("CREATE INDEX zz_sample_name_id ON SYSDBA.sample(name, id)");
                statement.execute("CREATE VIEW SYSDBA.v_sample AS SELECT id, name FROM SYSDBA.sample");
                statement.close();
                return connection;
            }
        });
    }

    private OscarIpcServer metadataOnlyServer() {
        return new OscarIpcServer(new JdbcConnectionFactory() {
            @Override
            public Connection open(OscarConfig config) throws Exception {
                Connection connection = DriverManager.getConnection("jdbc:h2:mem:oscar_metadata_" + SERVER_DB_COUNTER.incrementAndGet() + ";DATABASE_TO_UPPER=FALSE");
                Statement statement = connection.createStatement();
                statement.execute("CREATE SCHEMA SYSDBA");
                statement.execute("CREATE TABLE SYSDBA.parent_sample (id BIGINT PRIMARY KEY)");
                statement.execute("CREATE TABLE SYSDBA.sample (id BIGINT NOT NULL, name VARCHAR(64) DEFAULT 'abc', CONSTRAINT pk_sample PRIMARY KEY (id), CONSTRAINT fk_sample_parent FOREIGN KEY (id) REFERENCES SYSDBA.parent_sample(id))");
                statement.execute("CREATE INDEX idx_sample_name ON SYSDBA.sample(name)");
                statement.execute("CREATE VIEW SYSDBA.v_sample AS SELECT id, name FROM SYSDBA.sample");
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

    private JsonNode findPrimaryIndex(JsonNode rows) {
        for (JsonNode row : rows) {
            if (row.get("is_primary").asBoolean()) {
                return row;
            }
        }
        throw new AssertionError("missing primary index: " + rows);
    }

    private JsonNode findObjectViewRow(JsonNode rows, String firstCell) {
        for (JsonNode row : rows) {
            if (firstCell.equals(row.get(0).asText())) {
                return row;
            }
        }
        throw new AssertionError("missing object view row starting with " + firstCell + ": " + rows);
    }

    private static String configJson() {
        return "{\"host\":\"127.0.0.1\",\"username\":\"SYSDBA\",\"password\":\"secret\",\"database\":\"OSRDB\"}";
    }

    private static Connection catalogConnection() {
        return (Connection) Proxy.newProxyInstance(
            OscarIpcServerTest.class.getClassLoader(),
            new Class<?>[]{Connection.class},
            new InvocationHandler() {
                @Override
                public Object invoke(Object proxy, Method method, Object[] args) throws Throwable {
                    String name = method.getName();
                    if ("isValid".equals(name)) {
                        return Boolean.TRUE;
                    }
                    if ("createStatement".equals(name)) {
                        throw new SQLException("statement catalog query is not supported");
                    }
                    if ("prepareStatement".equals(name)) {
                        throw new SQLException("prepared catalog query is not supported");
                    }
                    if ("close".equals(name)) {
                        return null;
                    }
                    throw new UnsupportedOperationException(name);
                }
            }
        );
    }
}
