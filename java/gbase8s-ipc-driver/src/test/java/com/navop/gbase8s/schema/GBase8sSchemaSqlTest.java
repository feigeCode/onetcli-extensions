package com.navop.gbase8s.schema;

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
            "SELECT t.tabname, CASE t.tabtype WHEN 'T' THEN 'table' WHEN 'V' THEN 'view' ELSE 'table' END, COALESCE(c.comments, '') FROM systables t LEFT JOIN syscomms c ON c.tabid = t.tabid WHERE t.tabid >= 100 AND TRIM(t.owner) = 'gbasedbt' AND t.tabtype IN ('T', 'V') ORDER BY t.tabname",
            GBase8sSchemaSql.objectsSql("stores", "gbasedbt", Arrays.asList("table", "view"))
        );
    }

    @Test
    public void columnsSqlEscapesTableName() {
        assertEquals(
            "SELECT c.colno, c.colname, c.coltype, CASE WHEN BITAND(c.coltype, 256) = 256 THEN 'NO' ELSE 'YES' END, d.default, COALESCE(cm.comments, ''), c.collength, CAST(ce.coltypename2 AS VARCHAR(128)) FROM syscolumns c JOIN systables t ON c.tabid = t.tabid LEFT JOIN sysdefaults d ON d.tabid = c.tabid AND d.colno = c.colno LEFT JOIN syscolcomms cm ON cm.tabid = c.tabid AND cm.colno = c.colno LEFT JOIN syscolumnsext ce ON ce.tabid = c.tabid AND ce.colno = c.colno WHERE t.tabname = 'order''items' AND TRIM(t.owner) = 'gbasedbt' ORDER BY c.colno",
            GBase8sSchemaSql.columnsSql("stores", "gbasedbt", "order'items")
        );
    }

    @Test
    public void viewsSqlIncludesTableComment() {
        assertEquals(
            "SELECT t.tabname, 'view', COALESCE(c.comments, '') FROM systables t LEFT JOIN syscomms c ON c.tabid = t.tabid WHERE t.tabid >= 100 AND t.tabtype = 'V' AND TRIM(t.owner) = 'gbasedbt' ORDER BY t.tabname",
            GBase8sSchemaSql.viewsSql("stores", "gbasedbt")
        );
    }

    @Test
    public void dumpDdlSqlCallsGetDdlWithEscapedOwnerAndTable() {
        assertEquals(
            "EXECUTE PROCEDURE get_ddl('table', 'gbasedbt', 'order''items', 0)",
            GBase8sSchemaSql.dumpDdlSql("gbasedbt", "order'items")
        );
    }

    @Test
    public void normalizeGetDdlScriptTerminatesCreateTableBeforeComment() {
        String raw = "CREATE TABLE demo_child (\n"
            + "    id INTEGER NOT NULL,\n"
            + "    parent_id INTEGER,\n"
            + "    name VARCHAR(40),\n"
            + "    PRIMARY KEY (id)\n"
            + ")\n"
            + "COMMENT ON TABLE demo_child IS 'demo表';\n"
            + "COMMENT ON COLUMN demo_child.id IS '主键';\n"
            + "COMMENT ON COLUMN demo_child.parent_id IS '父级Id'";
        String expected = "CREATE TABLE demo_child (\n"
            + "    id INTEGER NOT NULL,\n"
            + "    parent_id INTEGER,\n"
            + "    name VARCHAR(40),\n"
            + "    PRIMARY KEY (id)\n"
            + ");\n"
            + "COMMENT ON TABLE demo_child IS 'demo表';\n"
            + "COMMENT ON COLUMN demo_child.id IS '主键';\n"
            + "COMMENT ON COLUMN demo_child.parent_id IS '父级Id'";
        assertEquals(expected, GBase8sSchemaSql.normalizeGetDdlScript(raw));
    }

    @Test
    public void normalizeGetDdlScriptLeavesAlreadyTerminatedCreateTableUntouched() {
        String raw = "CREATE TABLE demo_edit_items (\n"
            + "    id INTEGER NOT NULL,\n"
            + "    name VARCHAR(40),\n"
            + "    PRIMARY KEY (id)\n"
            + ");";
        assertEquals(raw, GBase8sSchemaSql.normalizeGetDdlScript(raw));
    }

    @Test
    public void normalizeGetDdlScriptTerminatesCreateTableBeforeIndexAndBlankLines() {
        String raw = "CREATE TABLE t (\n"
            + "    id INTEGER NOT NULL,\n"
            + "    amount DECIMAL(10,2),\n"
            + "    PRIMARY KEY (id)\n"
            + ")\n"
            + "\n"
            + "CREATE UNIQUE INDEX ix_t ON t (id);\n"
            + "COMMENT ON TABLE t IS 'with index'";
        String expected = "CREATE TABLE t (\n"
            + "    id INTEGER NOT NULL,\n"
            + "    amount DECIMAL(10,2),\n"
            + "    PRIMARY KEY (id)\n"
            + ");\n"
            + "\n"
            + "CREATE UNIQUE INDEX ix_t ON t (id);\n"
            + "COMMENT ON TABLE t IS 'with index'";
        assertEquals(expected, GBase8sSchemaSql.normalizeGetDdlScript(raw));
    }

    @Test
    public void normalizeGetDdlScriptHandlesNullAndSingleLine() {
        assertEquals(null, GBase8sSchemaSql.normalizeGetDdlScript(null));
        assertEquals("CREATE TABLE t (id INT);", GBase8sSchemaSql.normalizeGetDdlScript("CREATE TABLE t (id INT);"));
    }
}
