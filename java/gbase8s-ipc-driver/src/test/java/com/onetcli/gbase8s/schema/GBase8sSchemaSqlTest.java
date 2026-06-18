package com.onetcli.gbase8s.schema;

import org.junit.Test;

import java.util.Arrays;

import static org.junit.Assert.assertEquals;

public class GBase8sSchemaSqlTest {
    @Test
    public void databaseSqlUsesSysmasterDatabases() {
        assertEquals(
            "SELECT name FROM sysmaster:sysdatabases ORDER BY name",
            GBase8sSchemaSql.databasesSql()
        );
    }

    @Test
    public void schemasSqlUsesSysusers() {
        assertEquals(
            "SELECT username, username FROM sysusers ORDER BY username",
            GBase8sSchemaSql.schemasSql("stores")
        );
    }

    @Test
    public void objectsSqlMapsTablesAndViews() {
        assertEquals(
            "SELECT tabname, CASE tabtype WHEN 'T' THEN 'table' WHEN 'V' THEN 'view' ELSE 'table' END, '' FROM systables WHERE tabid >= 100 ORDER BY tabname",
            GBase8sSchemaSql.objectsSql("stores", "gbasedbt", Arrays.asList("table", "view"))
        );
    }

    @Test
    public void columnsSqlEscapesTableName() {
        assertEquals(
            "SELECT c.colno + 1, c.colname, c.coltype, CASE WHEN BITAND(c.coltype, 256) = 256 THEN 'NO' ELSE 'YES' END, '' FROM syscolumns c JOIN systables t ON c.tabid = t.tabid WHERE t.tabname = 'order''items' ORDER BY c.colno",
            GBase8sSchemaSql.columnsSql("stores", "gbasedbt", "order'items")
        );
    }
}
