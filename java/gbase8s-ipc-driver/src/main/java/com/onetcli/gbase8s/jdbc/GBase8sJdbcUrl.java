package com.onetcli.gbase8s.jdbc;

import java.util.Map;
import java.util.TreeMap;

public final class GBase8sJdbcUrl {
    private GBase8sJdbcUrl() {
    }

    public static String build(GBase8sConfig config) {
        validateUrlPart("host", config.getHost());
        validateUrlPart("database", config.getDatabase());

        StringBuilder url = new StringBuilder();
        url.append("jdbc:gbasedbt-sqli://")
            .append(config.getHost())
            .append(':')
            .append(config.getPort())
            .append('/')
            .append(config.getDatabase())
            .append(':');

        Map<String, String> sorted = new TreeMap<String, String>(config.getExtraParams());
        for (Map.Entry<String, String> entry : sorted.entrySet()) {
            appendProperty(url, entry.getKey(), entry.getValue());
        }
        return url.toString();
    }

    private static void appendProperty(StringBuilder url, String key, String value) {
        validatePropertyKey(key);
        validatePropertyValue(key, value);
        url.append(key).append('=').append(value).append(';');
    }

    private static void validateUrlPart(String name, String value) {
        if (value == null || value.trim().isEmpty()) {
            throw new IllegalArgumentException(name + " is required");
        }
        if (value.indexOf(';') >= 0 || value.indexOf('\n') >= 0 || value.indexOf('\r') >= 0) {
            throw new IllegalArgumentException(name + " contains invalid JDBC URL characters");
        }
    }

    private static void validatePropertyKey(String key) {
        if (key == null || key.trim().isEmpty()) {
            throw new IllegalArgumentException("JDBC property key is required");
        }
        if (key.indexOf(';') >= 0 || key.indexOf('=') >= 0 || key.indexOf('\n') >= 0 || key.indexOf('\r') >= 0) {
            throw new IllegalArgumentException("JDBC property " + key + " contains invalid characters");
        }
    }

    private static void validatePropertyValue(String key, String value) {
        if (value == null) {
            throw new IllegalArgumentException("JDBC property " + key + " value is required");
        }
        if (value.indexOf(';') >= 0 || value.indexOf('\n') >= 0 || value.indexOf('\r') >= 0) {
            throw new IllegalArgumentException("JDBC property " + key + " contains invalid characters");
        }
    }
}
