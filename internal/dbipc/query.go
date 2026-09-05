package dbipc

import (
	"context"
	"database/sql"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"math"
	"strconv"
	"strings"
	"time"
	"unicode/utf8"
)

type columnSpec struct {
	Name     string `json:"name"`
	Type     string `json:"type"`
	TypeKind string `json:"type_kind"`
	Nullable *bool  `json:"nullable,omitempty"`
}

type cellValue map[string]any

type queryExecutor interface {
	QueryContext(context.Context, string, ...any) (*sql.Rows, error)
}

func startQuery(ctx context.Context, queryer queryExecutor, sqlText string, args []any) ([]columnSpec, *sql.Rows, error) {
	rows, err := queryer.QueryContext(ctx, sqlText, args...)
	if err != nil {
		return nil, nil, err
	}

	cols, err := rows.Columns()
	if err != nil {
		rows.Close()
		return nil, nil, err
	}
	types, _ := rows.ColumnTypes()
	specs := make([]columnSpec, 0, len(cols))
	for i, name := range cols {
		typeName := "unknown"
		nullable := (*bool)(nil)
		if i < len(types) {
			if t := types[i].DatabaseTypeName(); t != "" {
				typeName = t
			}
			if n, ok := types[i].Nullable(); ok {
				nullable = &n
			}
		}
		specs = append(specs, columnSpec{Name: name, Type: typeName, TypeKind: typeKind(typeName), Nullable: nullable})
	}

	return specs, rows, nil
}

func fetchRows(rows *sql.Rows, kinds []string, n int, maxRows *uint64, fetched uint64) ([][]cellValue, bool, uint64, error) {
	if n <= 0 {
		n = 500
	}
	out := make([][]cellValue, 0)
	for len(out) < n {
		if maxRows != nil && fetched >= *maxRows {
			break
		}
		if !rows.Next() {
			if err := rows.Err(); err != nil {
				return nil, false, fetched, err
			}
			return out, true, fetched, nil
		}
		row, err := scanCurrentRow(rows, kinds)
		if err != nil {
			return nil, false, fetched, err
		}
		out = append(out, row)
		fetched++
	}
	if maxRows != nil && fetched >= *maxRows {
		return out, true, fetched, nil
	}
	return out, false, fetched, nil
}

func scanCurrentRow(rows *sql.Rows, kinds []string) ([]cellValue, error) {
	values := make([]any, len(kinds))
	ptrs := make([]any, len(kinds))
	for i := range values {
		ptrs[i] = &values[i]
	}
	if err := rows.Scan(ptrs...); err != nil {
		return nil, err
	}
	row := make([]cellValue, 0, len(values))
	for i, value := range values {
		if i < len(kinds) {
			row = append(row, toCellForKind(value, kinds[i]))
		} else {
			row = append(row, toCell(value))
		}
	}
	return row, nil
}

func queryObjects(ctx context.Context, db *sql.DB, sqlText string, mapRow func([]any) map[string]any) ([]map[string]any, error) {
	rows, err := db.QueryContext(ctx, sqlText)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	cols, err := rows.Columns()
	if err != nil {
		return nil, err
	}
	out := []map[string]any{}
	for rows.Next() {
		values := make([]any, len(cols))
		ptrs := make([]any, len(cols))
		for i := range values {
			ptrs[i] = &values[i]
		}
		if err := rows.Scan(ptrs...); err != nil {
			return nil, err
		}
		out = append(out, mapRow(values))
	}
	return out, rows.Err()
}

func paramsFromWire(params []cellValue) ([]any, error) {
	if len(params) == 0 {
		return nil, nil
	}
	out := make([]any, 0, len(params))
	for _, param := range params {
		value, err := paramFromWire(param)
		if err != nil {
			return nil, err
		}
		out = append(out, value)
	}
	return out, nil
}

func paramFromWire(param cellValue) (any, error) {
	t, _ := param["type"].(string)
	value := param["value"]
	switch strings.ToLower(t) {
	case "":
		return nil, fmt.Errorf("missing required cell value type")
	case "null":
		return nil, nil
	case "bool":
		return asBool(value)
	case "i64":
		return asInt64(value)
	case "u64":
		return asUint64Param(value)
	case "f64":
		return asFloat64(value)
	case "decimal", "text", "uuid", "date", "time", "datetime", "duration", "geo", "custom":
		return asString(value), nil
	case "bytes":
		raw, err := base64.StdEncoding.DecodeString(asString(value))
		if err != nil {
			return nil, fmt.Errorf("invalid base64 bytes parameter: %w", err)
		}
		return raw, nil
	case "json", "array", "map":
		raw, err := json.Marshal(value)
		if err != nil {
			return nil, err
		}
		return string(raw), nil
	default:
		return value, nil
	}
}

func toCell(value any) cellValue {
	switch v := value.(type) {
	case nil:
		return cellValue{"type": "null"}
	case bool:
		return cellValue{"type": "bool", "value": v}
	case int:
		return cellValue{"type": "i64", "value": int64(v)}
	case int8:
		return cellValue{"type": "i64", "value": int64(v)}
	case int16:
		return cellValue{"type": "i64", "value": int64(v)}
	case int32:
		return cellValue{"type": "i64", "value": int64(v)}
	case int64:
		return cellValue{"type": "i64", "value": v}
	case uint:
		return cellValue{"type": "u64", "value": uint64(v)}
	case uint8:
		return cellValue{"type": "u64", "value": uint64(v)}
	case uint16:
		return cellValue{"type": "u64", "value": uint64(v)}
	case uint32:
		return cellValue{"type": "u64", "value": uint64(v)}
	case uint64:
		return cellValue{"type": "u64", "value": v}
	case float32:
		return cellValue{"type": "f64", "value": float64(v)}
	case float64:
		return cellValue{"type": "f64", "value": v}
	case []byte:
		return cellValue{"type": "bytes", "value": base64.StdEncoding.EncodeToString(v)}
	case string:
		return cellValue{"type": "text", "value": v}
	case time.Time:
		return cellValue{"type": "datetime", "value": v.UTC().Format(time.RFC3339Nano)}
	default:
		return cellValue{"type": "text", "value": fmt.Sprint(v)}
	}
}

// toCellForKind encodes a scanned value using the column's declared type kind.
//
// MySQL-protocol drivers (go-sql-driver/mysql, obconnector-go) return
// VARCHAR/TEXT/DECIMAL/DATETIME/JSON columns as []byte, so the Go runtime type
// alone cannot distinguish text from binary. The column's declared type kind
// is authoritative: text-family bytes become text cells, and binary or
// unresolvable kinds keep the lossless base64 `bytes` encoding. Text bytes
// that are not valid UTF-8 also fall back to `bytes` because the wire cell is
// JSON-encoded; silently replacing them with U+FFFD would lose data.
func toCellForKind(value any, kind string) cellValue {
	raw, ok := value.([]byte)
	if !ok {
		return toCell(value)
	}
	switch kind {
	case "text":
		return textCell(raw)
	case "i64":
		// 文本协议驱动把整数家族也扫描为 []byte；按声明类型还原数值语义，
		// 超出 float64 精度的值保留原始十进制文本以避免精度损失。
		if n, err := strconv.ParseInt(string(raw), 10, 64); err == nil {
			return cellValue{"type": "i64", "value": n}
		}
		if utf8.Valid(raw) {
			return cellValue{"type": "text", "value": string(raw)}
		}
	case "u64":
		if n, err := strconv.ParseUint(string(raw), 10, 64); err == nil {
			return cellValue{"type": "u64", "value": strconv.FormatUint(n, 10)}
		}
		if utf8.Valid(raw) {
			return cellValue{"type": "text", "value": string(raw)}
		}
	case "f64":
		if f, err := strconv.ParseFloat(strings.TrimSpace(string(raw)), 64); err == nil {
			return cellValue{"type": "f64", "value": f}
		}
		if utf8.Valid(raw) {
			return cellValue{"type": "text", "value": string(raw)}
		}
	case "bool":
		if utf8.Valid(raw) {
			switch strings.TrimSpace(string(raw)) {
			case "0", "false", "FALSE":
				return cellValue{"type": "bool", "value": false}
			case "1", "true", "TRUE":
				return cellValue{"type": "bool", "value": true}
			}
		}
	case "decimal":
		if utf8.Valid(raw) {
			return cellValue{"type": "decimal", "value": string(raw)}
		}
	case "date", "time", "datetime":
		if utf8.Valid(raw) {
			return cellValue{"type": kind, "value": string(raw)}
		}
	case "json":
		var j any
		if err := json.Unmarshal(raw, &j); err == nil {
			return cellValue{"type": "json", "value": j}
		}
		if utf8.Valid(raw) {
			return cellValue{"type": "text", "value": string(raw)}
		}
	}
	return cellValue{"type": "bytes", "value": base64.StdEncoding.EncodeToString(raw)}
}

func textCell(raw []byte) cellValue {
	if utf8.Valid(raw) {
		return cellValue{"type": "text", "value": string(raw)}
	}
	return cellValue{"type": "bytes", "value": base64.StdEncoding.EncodeToString(raw)}
}

func asBool(value any) (bool, error) {
	switch v := value.(type) {
	case bool:
		return v, nil
	case string:
		return strconv.ParseBool(v)
	default:
		return strconv.ParseBool(fmt.Sprint(v))
	}
}

func asInt64(value any) (int64, error) {
	switch v := value.(type) {
	case int64:
		return v, nil
	case int:
		return int64(v), nil
	case float64:
		if math.Trunc(v) != v {
			return 0, fmt.Errorf("i64 parameter has non-integer value %v", v)
		}
		return int64(v), nil
	case string:
		return strconv.ParseInt(v, 10, 64)
	default:
		return strconv.ParseInt(fmt.Sprint(v), 10, 64)
	}
}

func asUint64Param(value any) (any, error) {
	var n uint64
	switch v := value.(type) {
	case uint64:
		n = v
	case uint:
		n = uint64(v)
	case float64:
		if math.Trunc(v) != v {
			return nil, fmt.Errorf("u64 parameter has non-integer value %v", v)
		}
		n = uint64(v)
	case string:
		parsed, err := strconv.ParseUint(v, 10, 64)
		if err != nil {
			return nil, err
		}
		n = parsed
	default:
		parsed, err := strconv.ParseUint(fmt.Sprint(v), 10, 64)
		if err != nil {
			return nil, err
		}
		n = parsed
	}
	if n <= math.MaxInt64 {
		return int64(n), nil
	}
	return strconv.FormatUint(n, 10), nil
}

func asFloat64(value any) (float64, error) {
	switch v := value.(type) {
	case float64:
		return v, nil
	case float32:
		return float64(v), nil
	case string:
		return strconv.ParseFloat(v, 64)
	default:
		return strconv.ParseFloat(fmt.Sprint(v), 64)
	}
}

func asString(value any) string {
	if value == nil {
		return ""
	}
	switch v := value.(type) {
	case string:
		return v
	default:
		return fmt.Sprint(v)
	}
}

func typeKind(raw string) string {
	t := strings.ToLower(raw)
	switch {
	case strings.Contains(t, "bool"), strings.Contains(t, "tinyint(1)"):
		return "bool"
	case strings.Contains(t, "interval"):
		return "text"
	case strings.Contains(t, "unsigned"):
		return "u64"
	case strings.Contains(t, "int"), strings.Contains(t, "serial"):
		return "i64"
	case strings.Contains(t, "numeric"), strings.Contains(t, "decimal"), strings.Contains(t, "number"):
		return "decimal"
	case strings.Contains(t, "float"), strings.Contains(t, "double"), strings.Contains(t, "real"):
		return "f64"
	case strings.Contains(t, "date") && !strings.Contains(t, "time"):
		return "date"
	case strings.Contains(t, "time"), strings.Contains(t, "timestamp"):
		return "datetime"
	case strings.Contains(t, "blob"), strings.Contains(t, "byte"), strings.Contains(t, "binary"):
		return "bytes"
	case strings.Contains(t, "json"):
		return "json"
	case strings.Contains(t, "char"), strings.Contains(t, "text"), strings.Contains(t, "clob"), strings.Contains(t, "varchar"):
		return "text"
	case strings.Contains(t, "uuid"), strings.Contains(t, "xml"):
		return "text"
	default:
		return "unknown"
	}
}

func stringCell(values []any, index int) string {
	if index >= len(values) || values[index] == nil {
		return ""
	}
	switch v := values[index].(type) {
	case []byte:
		return string(v)
	default:
		return fmt.Sprint(v)
	}
}

func nullableString(values []any, index int) *string {
	if index >= len(values) || values[index] == nil {
		return nil
	}
	value := stringCell(values, index)
	return &value
}

func intCell(values []any, index int) int {
	if index >= len(values) || values[index] == nil {
		return 0
	}
	switch v := values[index].(type) {
	case int:
		return v
	case int64:
		return int(v)
	case []byte:
		n, _ := strconv.Atoi(string(v))
		return n
	case string:
		n, _ := strconv.Atoi(v)
		return n
	default:
		n, _ := strconv.Atoi(fmt.Sprint(v))
		return n
	}
}

func boolCell(values []any, index int) bool {
	if index >= len(values) || values[index] == nil {
		return true
	}
	switch v := values[index].(type) {
	case bool:
		return v
	case []byte:
		return parseNullable(string(v))
	case string:
		return parseNullable(v)
	default:
		return parseNullable(fmt.Sprint(v))
	}
}

func splitListCell(values []any, index int) []string {
	raw := stringCell(values, index)
	if strings.TrimSpace(raw) == "" {
		return []string{}
	}
	parts := strings.Split(raw, ",")
	out := make([]string, 0, len(parts))
	for _, part := range parts {
		part = strings.TrimSpace(part)
		if part != "" {
			out = append(out, part)
		}
	}
	return out
}

func parseNullable(value string) bool {
	switch strings.ToUpper(strings.TrimSpace(value)) {
	case "N", "NO", "0", "FALSE":
		return false
	default:
		return true
	}
}
