package oceanbase

import (
	"context"
	"database/sql"
	"database/sql/driver"
	"fmt"
	"strings"
)

type bulkExecer interface {
	BulkExecContext(ctx context.Context, argRows [][]driver.NamedValue) (driver.Result, error)
}

// BulkInsert is a helper to perform high-performance multi-row inserts.
// It uses native COM_STMT_BULK_EXECUTE if supported, otherwise rewrites the SQL.
func BulkInsert(ctx context.Context, db *sql.DB, tableName string, columns []string, values [][]any) (sql.Result, error) {
	if len(values) == 0 {
		return result{affectedRows: 0}, nil
	}

	// Try native bulk execution first
	res, err := tryNativeBulkInsert(ctx, db, tableName, columns, values)
	if err == nil {
		return res, nil
	}
	fallbackRes, fallbackErr := bulkInsertRewrite(ctx, db, tableName, columns, values)
	if fallbackErr == nil {
		return fallbackRes, nil
	}
	return nil, fmt.Errorf("native bulk insert failed: %w; rewrite fallback also failed: %v", err, fallbackErr)
}

func quoteIdent(name string) string {
	// Simple alphanumeric+underscore identifiers don't need quoting —
	// this prevents SQL injection while preserving case behavior
	// (Oracle uppercases unquoted identifiers).
	if isSimpleIdent(name) {
		return name
	}
	// For identifiers with special characters, quote with double quotes.
	return `"` + strings.ReplaceAll(name, `"`, `""`) + `"`
}

func isSimpleIdent(name string) bool {
	if len(name) == 0 || !isAlpha(name[0]) {
		return false
	}
	for i := 1; i < len(name); i++ {
		if !isAlnum(name[i]) && name[i] != '_' {
			return false
		}
	}
	return true
}

func isAlpha(c byte) bool {
	return (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z')
}

func isAlnum(c byte) bool {
	return isAlpha(c) || (c >= '0' && c <= '9')
}

func quoteIdentList(names []string) string {
	quoted := make([]string, len(names))
	for i, n := range names {
		quoted[i] = quoteIdent(n)
	}
	return strings.Join(quoted, ", ")
}

func tryNativeBulkInsert(ctx context.Context, db *sql.DB, tableName string, columns []string, values [][]any) (sql.Result, error) {
	columnNames := quoteIdentList(columns)
	placeholders := "(" + strings.Repeat("?, ", len(columns)-1) + "?)"
	query := fmt.Sprintf("INSERT INTO %s (%s) VALUES %s", quoteIdent(tableName), columnNames, placeholders)

	conn, err := db.Conn(ctx)
	if err != nil {
		return nil, err
	}
	defer conn.Close()

	var result sql.Result
	err = conn.Raw(func(driverConn any) error {
		dConn, ok := driverConn.(driver.ConnPrepareContext)
		if !ok {
			return fmt.Errorf("driver does not support ConnPrepareContext")
		}
		stmt, err := dConn.PrepareContext(ctx, query)
		if err != nil {
			return err
		}
		defer stmt.Close()

		bExec, ok := stmt.(bulkExecer)
		if !ok {
			return fmt.Errorf("statement does not support BulkExecContext")
		}

		argRows := make([][]driver.NamedValue, len(values))
		for i, row := range values {
			named := make([]driver.NamedValue, len(row))
			for j, val := range row {
				named[j] = driver.NamedValue{Ordinal: j + 1, Value: val}
			}
			argRows[i] = named
		}

		res, err := bExec.BulkExecContext(ctx, argRows)
		if err != nil {
			return err
		}
		result = res
		return nil
	})

	return result, err
}

func bulkInsertRewrite(ctx context.Context, db *sql.DB, tableName string, columns []string, values [][]any) (sql.Result, error) {
	columnNames := quoteIdentList(columns)
	placeholderRow := "(" + strings.Repeat("?, ", len(columns)-1) + "?)"

	const maxChunkSize = 1000

	var totalAffected int64
	for i := 0; i < len(values); i += maxChunkSize {
		end := i + maxChunkSize
		if end > len(values) {
			end = len(values)
		}

		chunk := values[i:end]
		placeholders := make([]string, len(chunk))
		for j := range chunk {
			placeholders[j] = placeholderRow
		}

		query := fmt.Sprintf("INSERT INTO %s (%s) VALUES %s", quoteIdent(tableName), columnNames, strings.Join(placeholders, ", "))

		flattenedValues := make([]any, 0, len(chunk)*len(columns))
		for _, row := range chunk {
			flattenedValues = append(flattenedValues, row...)
		}

		res, err := db.ExecContext(ctx, query, flattenedValues...)
		if err != nil {
			return nil, err
		}

		affected, _ := res.RowsAffected()
		totalAffected += affected
	}

	return result{affectedRows: totalAffected}, nil
}
