package com.onetcli.gbase8s.db;

import org.junit.After;
import org.junit.Before;
import org.junit.Test;

import java.math.BigDecimal;
import java.sql.Connection;
import java.sql.DriverManager;
import java.sql.PreparedStatement;
import java.sql.ResultSet;
import java.sql.Statement;
import java.util.List;
import java.util.Map;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

public class JdbcQueryRunnerTest {
    private Connection connection;
    private JdbcQueryRunner runner;

    @Before
    public void setUp() throws Exception {
        connection = DriverManager.getConnection("jdbc:h2:mem:gbase8s_query_test_" + System.nanoTime());
        runner = new JdbcQueryRunner();
        Statement statement = connection.createStatement();
        statement.execute("CREATE TABLE sample (" +
            "id BIGINT PRIMARY KEY, " +
            "name VARCHAR(64), " +
            "amount DECIMAL(10, 2), " +
            "created_on DATE, " +
            "payload BINARY(3))");
        statement.execute("INSERT INTO sample VALUES (1, 'alpha', 12.30, DATE '2026-06-05', X'010203')");
        statement.execute("INSERT INTO sample VALUES (2, 'beta', 45.67, DATE '2026-06-06', X'040506')");
        statement.close();
    }

    @After
    public void tearDown() throws Exception {
        if (connection != null) {
            connection.close();
        }
    }

    @Test
    public void queryBufferedMapsColumnsAndCells() throws Exception {
        QueryResult result = runner.queryBuffered(
            connection,
            "SELECT id, name, amount, created_on, payload FROM sample ORDER BY id",
            null,
            Integer.valueOf(1)
        );

        assertEquals(5, result.getColumns().size());
        assertColumn(result.getColumns().get(0), "ID", "i64");
        assertColumn(result.getColumns().get(1), "NAME", "text");
        assertColumn(result.getColumns().get(2), "AMOUNT", "decimal");
        assertColumn(result.getColumns().get(3), "CREATED_ON", "date");
        assertColumn(result.getColumns().get(4), "PAYLOAD", "bytes");

        assertEquals(1, result.getRows().size());
        List<Map<String, Object>> row = result.getRows().get(0);
        assertCell(row.get(0), "i64", Long.valueOf(1));
        assertCell(row.get(1), "text", "alpha");
        assertCell(row.get(2), "decimal", "12.30");
        assertCell(row.get(3), "date", "2026-06-05");
        assertCell(row.get(4), "bytes", "AQID");
    }

    @Test
    public void queryBufferedBindsParameters() throws Exception {
        QueryResult result = runner.queryBuffered(
            connection,
            "SELECT name FROM sample WHERE amount > ? ORDER BY id",
            java.util.Collections.<Map<String, Object>>singletonList(decimalParam("20.00")),
            null
        );

        assertEquals(1, result.getRows().size());
        assertCell(result.getRows().get(0).get(0), "text", "beta");
    }

    @Test
    public void execRunReturnsAffectedRows() throws Exception {
        long affected = runner.execRun(
            connection,
            "UPDATE sample SET name = ? WHERE id = ?",
            java.util.Arrays.<Map<String, Object>>asList(textParam("renamed"), i64Param(2))
        );

        assertEquals(1L, affected);

        PreparedStatement statement = connection.prepareStatement("SELECT name FROM sample WHERE id = 2");
        ResultSet rows = statement.executeQuery();
        assertTrue(rows.next());
        assertEquals("renamed", rows.getString(1));
        rows.close();
        statement.close();
    }

    private static void assertColumn(Map<String, Object> column, String name, String typeKind) {
        assertEquals(name, column.get("name"));
        assertEquals(typeKind, column.get("type_kind"));
    }

    private static void assertCell(Map<String, Object> cell, String type, Object value) {
        assertEquals(type, cell.get("type"));
        assertEquals(value, cell.get("value"));
    }

    private static Map<String, Object> decimalParam(String value) {
        java.util.Map<String, Object> param = new java.util.LinkedHashMap<String, Object>();
        param.put("type", "decimal");
        param.put("value", value);
        return param;
    }

    private static Map<String, Object> textParam(String value) {
        java.util.Map<String, Object> param = new java.util.LinkedHashMap<String, Object>();
        param.put("type", "text");
        param.put("value", value);
        return param;
    }

    private static Map<String, Object> i64Param(long value) {
        java.util.Map<String, Object> param = new java.util.LinkedHashMap<String, Object>();
        param.put("type", "i64");
        param.put("value", value);
        return param;
    }
}
