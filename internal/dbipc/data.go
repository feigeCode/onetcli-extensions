package dbipc

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/base64"
	"encoding/csv"
	"encoding/json"
	"fmt"
	"io"
	"strings"
	"time"
)

type streamState struct {
	data   []byte
	offset int
}

type importState struct {
	connID   uint64
	table    string
	schema   string
	database string
	columns  []string
	format   string
	started  time.Time
	inserted uint64
	failed   []map[string]any
}

func exportRows(ctx context.Context, queryer queryExecutor, sqlText, format string, args []any) ([]byte, map[string]any, uint64, error) {
	rows, err := queryer.QueryContext(ctx, sqlText, args...)
	if err != nil {
		return nil, nil, 0, err
	}
	defer rows.Close()

	columns, err := rows.Columns()
	if err != nil {
		return nil, nil, 0, err
	}
	metadata := map[string]any{"columns": columns, "format": format}
	switch strings.ToLower(format) {
	case "json":
		data, count, err := exportRowsJSON(rows, columns, false)
		return data, metadata, count, err
	case "ndjson":
		data, count, err := exportRowsJSON(rows, columns, true)
		return data, metadata, count, err
	case "csv":
		data, count, err := exportRowsCSV(rows, columns)
		return data, metadata, count, err
	default:
		return nil, nil, 0, fmt.Errorf("export format %q is not supported by the generic database/sql driver", format)
	}
}

func exportRowsJSON(rows *sql.Rows, columns []string, ndjson bool) ([]byte, uint64, error) {
	var out bytes.Buffer
	var all []map[string]any
	var count uint64
	for rows.Next() {
		row, err := rowObject(rows, columns)
		if err != nil {
			return nil, 0, err
		}
		count++
		if ndjson {
			raw, err := json.Marshal(row)
			if err != nil {
				return nil, 0, err
			}
			out.Write(raw)
			out.WriteByte('\n')
		} else {
			all = append(all, row)
		}
	}
	if err := rows.Err(); err != nil {
		return nil, 0, err
	}
	if ndjson {
		return out.Bytes(), count, nil
	}
	raw, err := json.Marshal(all)
	return raw, count, err
}

func exportRowsCSV(rows *sql.Rows, columns []string) ([]byte, uint64, error) {
	var out bytes.Buffer
	writer := csv.NewWriter(&out)
	if err := writer.Write(columns); err != nil {
		return nil, 0, err
	}
	var count uint64
	for rows.Next() {
		values := make([]any, len(columns))
		ptrs := make([]any, len(columns))
		for i := range values {
			ptrs[i] = &values[i]
		}
		if err := rows.Scan(ptrs...); err != nil {
			return nil, 0, err
		}
		record := make([]string, len(values))
		for i, value := range values {
			if value == nil {
				record[i] = ""
			} else if raw, ok := value.([]byte); ok {
				record[i] = string(raw)
			} else {
				record[i] = fmt.Sprint(value)
			}
		}
		if err := writer.Write(record); err != nil {
			return nil, 0, err
		}
		count++
	}
	if err := rows.Err(); err != nil {
		return nil, 0, err
	}
	writer.Flush()
	return out.Bytes(), count, writer.Error()
}

func rowObject(rows *sql.Rows, columns []string) (map[string]any, error) {
	values := make([]any, len(columns))
	ptrs := make([]any, len(columns))
	for i := range values {
		ptrs[i] = &values[i]
	}
	if err := rows.Scan(ptrs...); err != nil {
		return nil, err
	}
	row := make(map[string]any, len(columns))
	for i, name := range columns {
		value := values[i]
		if raw, ok := value.([]byte); ok {
			if json.Valid(raw) {
				var decoded any
				if err := json.Unmarshal(raw, &decoded); err == nil {
					row[name] = decoded
					continue
				}
			}
			row[name] = string(raw)
			continue
		}
		row[name] = value
	}
	return row, nil
}

func streamReadChunk(stream *streamState, maxBytes uint32) map[string]any {
	if maxBytes == 0 {
		maxBytes = 64 * 1024
	}
	end := stream.offset + int(maxBytes)
	if end > len(stream.data) {
		end = len(stream.data)
	}
	chunk := stream.data[stream.offset:end]
	stream.offset = end
	return map[string]any{
		"data": base64.StdEncoding.EncodeToString(chunk),
		"done": stream.offset >= len(stream.data),
	}
}

func buildExportSQL(spec DriverSpec, table, schema, database, sqlText, whereClause string, includeColumns, excludeColumns []string) (string, error) {
	if strings.TrimSpace(sqlText) != "" {
		return sqlText, nil
	}
	if strings.TrimSpace(table) == "" {
		return "", fmt.Errorf("data/export requires either sql or table")
	}
	columns := "*"
	if len(includeColumns) > 0 {
		filtered := make([]string, 0, len(includeColumns))
		excluded := map[string]bool{}
		for _, column := range excludeColumns {
			excluded[column] = true
		}
		for _, column := range includeColumns {
			if !excluded[column] {
				filtered = append(filtered, column)
			}
		}
		if len(filtered) > 0 {
			columns = quoteIdentifierList(spec, filtered)
		}
	}
	query := "SELECT " + columns + " FROM " + qualifiedTableName(spec, database, schema, table)
	if strings.TrimSpace(whereClause) != "" {
		query += " WHERE " + whereClause
	}
	return query, nil
}

func buildInsertSQL(spec DriverSpec, imp *importState) (string, error) {
	if imp.table == "" {
		return "", fmt.Errorf("data import table is required")
	}
	if len(imp.columns) == 0 {
		return "", fmt.Errorf("data import requires explicit columns")
	}
	placeholders := make([]string, len(imp.columns))
	for i := range placeholders {
		placeholders[i] = "?"
	}
	return "INSERT INTO " + qualifiedTableName(spec, imp.database, imp.schema, imp.table) +
		" (" + quoteIdentifierList(spec, imp.columns) + ") VALUES (" + strings.Join(placeholders, ", ") + ")", nil
}

func cellsToArgs(row []cellValue) ([]any, error) {
	args := make([]any, 0, len(row))
	for _, cell := range row {
		value, err := paramFromWire(cell)
		if err != nil {
			return nil, err
		}
		args = append(args, value)
	}
	return args, nil
}

func readAllString(reader io.Reader) string {
	if reader == nil {
		return ""
	}
	raw, _ := io.ReadAll(reader)
	return string(raw)
}
