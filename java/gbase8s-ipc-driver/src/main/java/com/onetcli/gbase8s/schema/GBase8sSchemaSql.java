package com.onetcli.gbase8s.schema;

import java.util.List;

public final class GBase8sSchemaSql {
    private GBase8sSchemaSql() {
    }

    public static String databasesSql() {
        return "SELECT name FROM sysmaster:sysdatabases ORDER BY name";
    }

    public static String schemasSql(String database) {
        return "SELECT username, username FROM sysusers ORDER BY username";
    }

    public static String objectsSql(String database, String schema, List<String> kinds) {
        return "SELECT tabname, CASE tabtype WHEN 'T' THEN 'table' WHEN 'V' THEN 'view' ELSE 'table' END, '' FROM systables WHERE tabid >= 100 ORDER BY tabname";
    }

    public static String columnsSql(String database, String schema, String table) {
        return "SELECT c.colno + 1, c.colname, c.coltype, CASE WHEN BITAND(c.coltype, 256) = 256 THEN 'NO' ELSE 'YES' END, '' FROM syscolumns c JOIN systables t ON c.tabid = t.tabid WHERE t.tabname = '" + escapeSql(table) + "' ORDER BY c.colno";
    }

    private static String escapeSql(String value) {
        return value == null ? "" : value.replace("'", "''");
    }
}
