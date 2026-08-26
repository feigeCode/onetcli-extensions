package com.navop.gbase8s.schema;

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
        return "SELECT t.tabname, CASE t.tabtype WHEN 'T' THEN 'table' WHEN 'V' THEN 'view' ELSE 'table' END, COALESCE(c.comments, '') "
            + "FROM systables t LEFT JOIN syscomms c ON c.tabid = t.tabid "
            + "WHERE t.tabid >= 100"
            + ownerFilter("t", schema)
            + kindFilter(kinds)
            + " ORDER BY t.tabname";
    }

    public static String columnsSql(String database, String schema, String table) {
        return "SELECT c.colno, c.colname, c.coltype, CASE WHEN BITAND(c.coltype, 256) = 256 THEN 'NO' ELSE 'YES' END, d.default, COALESCE(cm.comments, ''), c.collength, CAST(ce.coltypename2 AS VARCHAR(128)) "
            + "FROM syscolumns c "
            + "JOIN systables t ON c.tabid = t.tabid "
            + "LEFT JOIN sysdefaults d ON d.tabid = c.tabid AND d.colno = c.colno "
            + "LEFT JOIN syscolcomms cm ON cm.tabid = c.tabid AND cm.colno = c.colno "
            + "LEFT JOIN syscolumnsext ce ON ce.tabid = c.tabid AND ce.colno = c.colno "
            + "WHERE t.tabname = '" + escapeSql(table) + "'" + ownerFilter("t", schema) + " ORDER BY c.colno";
    }

    public static String primaryKeyColumnsSql(String database, String schema, String table) {
        return "SELECT c.colname FROM syscolumns c "
            + "JOIN systables t ON c.tabid = t.tabid "
            + "JOIN sysconstraints cn ON cn.tabid = t.tabid AND cn.constrtype = 'P' "
            + "JOIN sysindexes i ON i.tabid = t.tabid AND i.idxname = cn.idxname "
            + "WHERE t.tabname = '" + escapeSql(table) + "'" + ownerFilter("t", schema) + " AND ("
            + partEquals("i", "c.colno")
            + ") ORDER BY c.colno";
    }

    public static String indexesSql(String database, String schema, String table) {
        return "SELECT i.idxname, i.idxtype, CASE WHEN cn.constrtype = 'P' THEN 'YES' ELSE 'NO' END, "
            + "c.colname, c.colno, " + partOrderExpression("i", "c.colno") + " "
            + "FROM sysindexes i "
            + "JOIN systables t ON i.tabid = t.tabid "
            + "LEFT JOIN sysconstraints cn ON cn.tabid = i.tabid AND cn.idxname = i.idxname "
            + "JOIN syscolumns c ON c.tabid = i.tabid AND (" + partEquals("i", "c.colno") + ") "
            + "WHERE t.tabname = '" + escapeSql(table) + "'" + ownerFilter("t", schema) + " "
            + "ORDER BY i.idxname, " + partOrderExpression("i", "c.colno");
    }

    public static String foreignKeysSql(String database, String schema, String table) {
        return "SELECT rc.constrname, ct.tabname, cc.colname, pt.tabname, pc_col.colname, "
            + "r.updrule, r.delrule, " + pairedPartOrderExpression("ci", "cc.colno", "pi", "pc_col.colno") + " "
            + "FROM sysconstraints rc "
            + "JOIN systables ct ON ct.tabid = rc.tabid "
            + "JOIN sysreferences r ON r.constrid = rc.constrid "
            + "JOIN systables pt ON pt.tabid = r.ptabid "
            + "JOIN sysconstraints pc ON pc.tabid = pt.tabid AND pc.constrtype = 'P' "
            + "JOIN sysindexes ci ON ci.tabid = ct.tabid AND ci.idxname = rc.idxname "
            + "JOIN sysindexes pi ON pi.tabid = pt.tabid AND pi.idxname = pc.idxname "
            + "JOIN syscolumns cc ON cc.tabid = ct.tabid "
            + "JOIN syscolumns pc_col ON pc_col.tabid = pt.tabid "
            + "WHERE rc.constrtype = 'R' AND ct.tabname = '" + escapeSql(table) + "'" + ownerFilter("ct", schema) + " AND ("
            + pairedPartEquals("ci", "cc.colno", "pi", "pc_col.colno")
            + ") ORDER BY rc.constrname, " + pairedPartOrderExpression("ci", "cc.colno", "pi", "pc_col.colno");
    }

    public static String checksSql(String database, String schema, String table) {
        return "SELECT cn.constrname, t.tabname, ck.checktext, ck.seqno "
            + "FROM sysconstraints cn "
            + "JOIN systables t ON t.tabid = cn.tabid "
            + "JOIN syschecks ck ON ck.constrid = cn.constrid "
            + "WHERE cn.constrtype = 'C' AND ck.type = 'T' AND t.tabname = '" + escapeSql(table) + "'" + ownerFilter("t", schema) + " "
            + "ORDER BY cn.constrname, ck.seqno";
    }

    public static String functionsSql(String database, String schema) {
        return routinesSql(schema, "f");
    }

    public static String proceduresSql(String database, String schema) {
        return routinesSql(schema, "t");
    }

    public static String viewsSql(String database, String schema) {
        return "SELECT t.tabname, 'view', COALESCE(c.comments, '') "
            + "FROM systables t LEFT JOIN syscomms c ON c.tabid = t.tabid "
            + "WHERE t.tabid >= 100 AND t.tabtype = 'V'" + ownerFilter("t", schema) + " ORDER BY t.tabname";
    }

    /**
     * Invokes the official GBase 8s <code>get_ddl</code> SPL recipe to dump a
     * table's full structure. The SPL returns a single lvarchar(32000) blob
     * containing CREATE TABLE, index, PRIMARY KEY and COMMENT statements, so the
     * caller must treat the whole value as one atomic script rather than split it.
     * {@code p_del}=0 emits unquoted identifiers, matching this driver's dialect.
     */
    public static String dumpDdlSql(String owner, String table) {
        return "EXECUTE PROCEDURE get_ddl('table', '" + escapeSql(owner) + "', '" + escapeSql(table) + "', 0)";
    }

    private static String ownerFilter(String tableAlias, String schema) {
        if (schema == null || schema.trim().length() == 0) {
            return "";
        }
        String prefix = tableAlias == null || tableAlias.length() == 0 ? "" : tableAlias + ".";
        return " AND TRIM(" + prefix + "owner) = '" + escapeSql(schema.trim()) + "'";
    }

    private static String kindFilter(List<String> kinds) {
        if (kinds == null || kinds.isEmpty()) {
            return "";
        }
        boolean wantsTable = kinds.contains("table");
        boolean wantsView = kinds.contains("view");
        if (wantsTable && wantsView) {
            return " AND t.tabtype IN ('T', 'V')";
        }
        if (wantsTable) {
            return " AND t.tabtype = 'T'";
        }
        if (wantsView) {
            return " AND t.tabtype = 'V'";
        }
        return "";
    }

    private static String partEquals(String indexAlias, String value) {
        StringBuilder sql = new StringBuilder();
        for (int i = 1; i <= 16; i++) {
            if (i > 1) {
                sql.append(" OR ");
            }
            sql.append(indexAlias).append(".part").append(i).append(" = ").append(value);
        }
        return sql.toString();
    }

    private static String partOrderExpression(String indexAlias, String value) {
        StringBuilder sql = new StringBuilder("CASE ");
        for (int i = 1; i <= 16; i++) {
            sql.append("WHEN ").append(indexAlias).append(".part").append(i).append(" = ").append(value)
                .append(" THEN ").append(i).append(' ');
        }
        sql.append("ELSE 99 END");
        return sql.toString();
    }

    private static String pairedPartEquals(String leftIndexAlias, String leftValue, String rightIndexAlias, String rightValue) {
        StringBuilder sql = new StringBuilder();
        for (int i = 1; i <= 16; i++) {
            if (i > 1) {
                sql.append(" OR ");
            }
            sql.append('(')
                .append(leftIndexAlias).append(".part").append(i).append(" = ").append(leftValue)
                .append(" AND ")
                .append(rightIndexAlias).append(".part").append(i).append(" = ").append(rightValue)
                .append(')');
        }
        return sql.toString();
    }

    private static String pairedPartOrderExpression(String leftIndexAlias, String leftValue, String rightIndexAlias, String rightValue) {
        StringBuilder sql = new StringBuilder("CASE ");
        for (int i = 1; i <= 16; i++) {
            sql.append("WHEN ")
                .append(leftIndexAlias).append(".part").append(i).append(" = ").append(leftValue)
                .append(" AND ")
                .append(rightIndexAlias).append(".part").append(i).append(" = ").append(rightValue)
                .append(" THEN ").append(i).append(' ');
        }
        sql.append("ELSE 99 END");
        return sql.toString();
    }

    private static String routinesSql(String schema, String isProcedure) {
        StringBuilder sql = new StringBuilder();
        sql.append("SELECT p.procname, p.owner, pc.paramtype, 'SPL', '', b.data, b.seqno ")
            .append("FROM sysprocedures p ")
            .append("LEFT JOIN sysproccolumns pc ON pc.procid = p.procid AND pc.paramattr = 3 ")
            .append("LEFT JOIN sysprocbody b ON b.procid = p.procid AND b.datakey = 'T' ")
            .append("WHERE p.isproc = '").append(escapeSql(isProcedure)).append("'");
        if (schema != null && schema.trim().length() > 0) {
            sql.append(" AND p.owner = '").append(escapeSql(schema.trim())).append("'");
        }
        sql.append(" ORDER BY p.procname, b.seqno");
        return sql.toString();
    }

    private static String escapeSql(String value) {
        return value == null ? "" : value.replace("'", "''");
    }
}
