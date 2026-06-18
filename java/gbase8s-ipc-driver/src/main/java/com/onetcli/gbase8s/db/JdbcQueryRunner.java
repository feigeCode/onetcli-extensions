package com.onetcli.gbase8s.db;

import java.math.BigDecimal;
import java.sql.Connection;
import java.sql.Date;
import java.sql.PreparedStatement;
import java.sql.ResultSet;
import java.sql.ResultSetMetaData;
import java.sql.SQLException;
import java.sql.Time;
import java.sql.Timestamp;
import java.sql.Types;
import java.util.ArrayList;
import java.util.Base64;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

public final class JdbcQueryRunner {
    public QueryResult queryBuffered(
        Connection connection,
        String sql,
        List<Map<String, Object>> params,
        Integer maxRows
    ) throws SQLException {
        PreparedStatement statement = connection.prepareStatement(sql);
        try {
            bindParams(statement, params);
            ResultSet resultSet = statement.executeQuery();
            try {
                ResultSetMetaData meta = resultSet.getMetaData();
                List<Map<String, Object>> columns = columns(meta);
                List<List<Map<String, Object>>> rows = rows(resultSet, meta, maxRows);
                return new QueryResult(columns, rows);
            } finally {
                resultSet.close();
            }
        } finally {
            statement.close();
        }
    }

    public long execRun(Connection connection, String sql, List<Map<String, Object>> params) throws SQLException {
        PreparedStatement statement = connection.prepareStatement(sql);
        try {
            bindParams(statement, params);
            return statement.executeUpdate();
        } finally {
            statement.close();
        }
    }

    private static void bindParams(PreparedStatement statement, List<Map<String, Object>> params) throws SQLException {
        if (params == null) {
            return;
        }
        for (int i = 0; i < params.size(); i++) {
            Map<String, Object> param = params.get(i);
            statement.setObject(i + 1, paramValue(param));
        }
    }

    private static Object paramValue(Map<String, Object> param) {
        if (param == null) {
            return null;
        }
        String type = stringValue(param.get("type"));
        Object value = param.get("value");
        if ("null".equals(type) || value == null) {
            return null;
        }
        if ("decimal".equals(type)) {
            return new BigDecimal(String.valueOf(value));
        }
        if ("i64".equals(type)) {
            return Long.valueOf(String.valueOf(value));
        }
        if ("u64".equals(type)) {
            return new BigDecimal(String.valueOf(value));
        }
        if ("f64".equals(type)) {
            return Double.valueOf(String.valueOf(value));
        }
        if ("bool".equals(type)) {
            return Boolean.valueOf(String.valueOf(value));
        }
        if ("bytes".equals(type)) {
            return Base64.getDecoder().decode(String.valueOf(value));
        }
        if ("date".equals(type)) {
            return Date.valueOf(String.valueOf(value));
        }
        if ("time".equals(type)) {
            return Time.valueOf(String.valueOf(value));
        }
        if ("datetime".equals(type)) {
            return Timestamp.valueOf(String.valueOf(value).replace('T', ' ').replace("Z", ""));
        }
        return String.valueOf(value);
    }

    private static List<Map<String, Object>> columns(ResultSetMetaData meta) throws SQLException {
        int count = meta.getColumnCount();
        List<Map<String, Object>> out = new ArrayList<Map<String, Object>>(count);
        for (int i = 1; i <= count; i++) {
            Map<String, Object> column = new LinkedHashMap<String, Object>();
            String type = defaultString(meta.getColumnTypeName(i), "unknown");
            column.put("name", defaultString(meta.getColumnLabel(i), "col_" + i));
            column.put("type", type);
            column.put("type_kind", typeKind(type, meta.getColumnType(i)));
            int nullable = meta.isNullable(i);
            if (nullable != ResultSetMetaData.columnNullableUnknown) {
                column.put("nullable", nullable == ResultSetMetaData.columnNullable);
            }
            out.add(column);
        }
        return out;
    }

    private static List<List<Map<String, Object>>> rows(ResultSet resultSet, ResultSetMetaData meta, Integer maxRows)
        throws SQLException {
        int count = meta.getColumnCount();
        int limit = maxRows == null ? Integer.MAX_VALUE : Math.max(0, maxRows.intValue());
        List<List<Map<String, Object>>> out = new ArrayList<List<Map<String, Object>>>();
        while (resultSet.next() && out.size() < limit) {
            List<Map<String, Object>> row = new ArrayList<Map<String, Object>>(count);
            for (int i = 1; i <= count; i++) {
                row.add(cell(resultSet.getObject(i), meta.getColumnType(i)));
            }
            out.add(row);
        }
        return out;
    }

    private static Map<String, Object> cell(Object value, int sqlType) {
        Map<String, Object> cell = new LinkedHashMap<String, Object>();
        if (value == null) {
            cell.put("type", "null");
            return cell;
        }
        if (value instanceof Boolean) {
            cell.put("type", "bool");
            cell.put("value", value);
            return cell;
        }
        if (value instanceof Byte || value instanceof Short || value instanceof Integer || value instanceof Long) {
            cell.put("type", "i64");
            cell.put("value", Long.valueOf(((Number) value).longValue()));
            return cell;
        }
        if (value instanceof Float || value instanceof Double) {
            cell.put("type", "f64");
            cell.put("value", Double.valueOf(((Number) value).doubleValue()));
            return cell;
        }
        if (value instanceof BigDecimal) {
            cell.put("type", "decimal");
            cell.put("value", ((BigDecimal) value).toPlainString());
            return cell;
        }
        if (value instanceof byte[]) {
            cell.put("type", "bytes");
            cell.put("value", Base64.getEncoder().encodeToString((byte[]) value));
            return cell;
        }
        if (value instanceof Date && sqlType == Types.DATE) {
            cell.put("type", "date");
            cell.put("value", value.toString());
            return cell;
        }
        if (value instanceof Time) {
            cell.put("type", "time");
            cell.put("value", value.toString());
            return cell;
        }
        if (value instanceof Timestamp) {
            cell.put("type", "datetime");
            cell.put("value", ((Timestamp) value).toInstant().toString());
            return cell;
        }
        cell.put("type", "text");
        cell.put("value", String.valueOf(value));
        return cell;
    }

    private static String typeKind(String rawType, int sqlType) {
        switch (sqlType) {
            case Types.BOOLEAN:
            case Types.BIT:
                return "bool";
            case Types.TINYINT:
            case Types.SMALLINT:
            case Types.INTEGER:
            case Types.BIGINT:
                return "i64";
            case Types.NUMERIC:
            case Types.DECIMAL:
                return "decimal";
            case Types.FLOAT:
            case Types.REAL:
            case Types.DOUBLE:
                return "f64";
            case Types.DATE:
                return "date";
            case Types.TIME:
                return "time";
            case Types.TIMESTAMP:
            case -101:
            case -102:
                return "datetime";
            case Types.BINARY:
            case Types.VARBINARY:
            case Types.LONGVARBINARY:
            case Types.BLOB:
                return "bytes";
            case Types.CHAR:
            case Types.VARCHAR:
            case Types.LONGVARCHAR:
            case Types.CLOB:
            case Types.NCHAR:
            case Types.NVARCHAR:
            case Types.LONGNVARCHAR:
            case Types.NCLOB:
                return "text";
            default:
                return typeKindFromName(rawType);
        }
    }

    private static String typeKindFromName(String rawType) {
        String type = rawType == null ? "" : rawType.toLowerCase();
        if (type.indexOf("bool") >= 0) {
            return "bool";
        }
        if (type.indexOf("int") >= 0 || type.indexOf("serial") >= 0) {
            return "i64";
        }
        if (type.indexOf("decimal") >= 0 || type.indexOf("numeric") >= 0 || type.indexOf("money") >= 0) {
            return "decimal";
        }
        if (type.indexOf("float") >= 0 || type.indexOf("double") >= 0 || type.indexOf("real") >= 0) {
            return "f64";
        }
        if (type.indexOf("date") >= 0 && type.indexOf("time") < 0) {
            return "date";
        }
        if (type.indexOf("time") >= 0) {
            return "datetime";
        }
        if (type.indexOf("blob") >= 0 || type.indexOf("byte") >= 0 || type.indexOf("binary") >= 0) {
            return "bytes";
        }
        if (type.indexOf("char") >= 0 || type.indexOf("text") >= 0 || type.indexOf("clob") >= 0) {
            return "text";
        }
        return "unknown";
    }

    private static String defaultString(String value, String defaultValue) {
        return value == null || value.isEmpty() ? defaultValue : value;
    }

    private static String stringValue(Object value) {
        return value == null ? "" : String.valueOf(value);
    }
}
