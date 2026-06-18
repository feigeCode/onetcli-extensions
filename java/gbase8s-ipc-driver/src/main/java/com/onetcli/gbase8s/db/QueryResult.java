package com.onetcli.gbase8s.db;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Map;

public final class QueryResult {
    private final List<Map<String, Object>> columns;
    private final List<List<Map<String, Object>>> rows;

    public QueryResult(List<Map<String, Object>> columns, List<List<Map<String, Object>>> rows) {
        this.columns = Collections.unmodifiableList(new ArrayList<Map<String, Object>>(columns));
        this.rows = Collections.unmodifiableList(new ArrayList<List<Map<String, Object>>>(rows));
    }

    public List<Map<String, Object>> getColumns() {
        return columns;
    }

    public List<List<Map<String, Object>>> getRows() {
        return rows;
    }
}
