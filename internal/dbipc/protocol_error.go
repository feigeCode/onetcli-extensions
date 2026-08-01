package dbipc

import (
	"encoding/json"
	"errors"
	"fmt"
	"strconv"
	"strings"

	gokb "gitea.com/kingbase/gokb"
	dm "gitee.com/chunanyong/dm"
	mysql "github.com/go-sql-driver/mysql"
	"github.com/sijms/go-ora/v2/network"

	"navop-db-ipc-drivers/internal/ipc"
)

func protocolErrorFromError(code int32, err error) *ipc.ProtocolError {
	if err == nil {
		return &ipc.ProtocolError{Code: code}
	}

	message, data := databaseErrorDetails(err)
	raw, marshalErr := json.Marshal(data)
	if marshalErr != nil {
		return &ipc.ProtocolError{
			Code:    code,
			Message: message,
		}
	}
	return &ipc.ProtocolError{
		Code:    code,
		Message: message,
		Data:    raw,
	}
}

func databaseErrorDetails(err error) (string, ipc.ErrorData) {
	data := ipc.ErrorData{
		Extra: map[string]any{
			"chain":  errorChain(err),
			"source": err.Error(),
		},
	}
	message := err.Error()

	var kingbaseError gokb.Error
	if errors.As(err, &kingbaseError) {
		return addKingbaseErrorDetails(kingbaseError, data)
	}

	var kingbaseErrorPointer *gokb.Error
	if errors.As(err, &kingbaseErrorPointer) && kingbaseErrorPointer != nil {
		return addKingbaseErrorDetails(*kingbaseErrorPointer, data)
	}

	var mysqlError *mysql.MySQLError
	if errors.As(err, &mysqlError) && mysqlError != nil {
		vendorCode := int64(mysqlError.Number)
		data.VendorCode = &vendorCode
		if mysqlError.SQLState != [5]byte{} {
			data.SQLState = string(mysqlError.SQLState[:])
		}
		putNonEmpty(data.Extra, "server_message", mysqlError.Message)
		return message, data
	}

	var dmError *dm.DmError
	if errors.As(err, &dmError) && dmError != nil {
		vendorCode := int64(dmError.ErrCode)
		data.VendorCode = &vendorCode
		putNonEmpty(data.Extra, "server_message", dmError.ErrText)
		return message, data
	}

	var oracleError *network.OracleError
	if errors.As(err, &oracleError) && oracleError != nil {
		vendorCode := int64(oracleError.ErrCode)
		data.VendorCode = &vendorCode
		putNonEmpty(data.Extra, "server_message", oracleError.ErrMsg)
		if position := oracleError.ErrPos(); position >= 0 {
			data.Extra["position"] = position
		}
		return message, data
	}

	return message, data
}

func addKingbaseErrorDetails(err gokb.Error, data ipc.ErrorData) (string, ipc.ErrorData) {
	data.SQLState = string(err.Code)
	data.Schema = err.Schema
	data.Table = err.Table
	data.Column = err.Column
	data.Constraint = err.Constraint
	putNonEmpty(data.Extra, "severity", err.Severity)
	putNonEmpty(data.Extra, "detail", err.Detail)
	putNonEmpty(data.Extra, "hint", err.Hint)
	putNonEmpty(data.Extra, "position", err.Position)
	putNonEmpty(data.Extra, "internal_position", err.InternalPosition)
	putNonEmpty(data.Extra, "internal_query", err.InternalQuery)
	putNonEmpty(data.Extra, "where", err.Where)
	putNonEmpty(data.Extra, "datatype", err.DataTypeName)
	putNonEmpty(data.Extra, "file", err.File)
	putNonEmpty(data.Extra, "line", err.Line)
	putNonEmpty(data.Extra, "routine", err.Routine)
	return kingbaseErrorMessage(err), data
}

func kingbaseErrorMessage(err gokb.Error) string {
	parts := make([]string, 0, 8)
	if err.Message != "" {
		parts = append(parts, "kb: "+err.Message)
	} else {
		parts = append(parts, err.Error())
	}
	appendLabel := func(label, value string) {
		if value != "" {
			parts = append(parts, label+": "+value)
		}
	}
	appendLabel("DETAIL", err.Detail)
	appendLabel("HINT", err.Hint)
	appendLabel("WHERE", err.Where)
	appendLabel("SCHEMA", err.Schema)
	appendLabel("TABLE", err.Table)
	appendLabel("COLUMN", err.Column)
	appendLabel("CONSTRAINT", err.Constraint)
	if err.Code != "" {
		appendLabel("SQLSTATE", string(err.Code))
	}
	return strings.Join(parts, "\n")
}

func errorChain(err error) []string {
	var chain []string
	var visit func(error, int)
	visit = func(current error, depth int) {
		if current == nil || depth >= 64 {
			return
		}
		chain = append(chain, current.Error())
		switch unwrapped := current.(type) {
		case interface{ Unwrap() []error }:
			for _, child := range unwrapped.Unwrap() {
				visit(child, depth+1)
			}
		case interface{ Unwrap() error }:
			visit(unwrapped.Unwrap(), depth+1)
		}
	}
	visit(err, 0)
	return chain
}

func putNonEmpty(extra map[string]any, key, value string) {
	if strings.TrimSpace(value) != "" {
		extra[key] = value
	}
}

func joinCleanupError(existing error, operation string, err error) error {
	if err == nil {
		return existing
	}
	wrapped := fmt.Errorf("%s: %w", operation, err)
	if existing == nil {
		return wrapped
	}
	return errors.Join(existing, wrapped)
}

func rowsAffected(result interface{ RowsAffected() (int64, error) }) (uint64, error) {
	affected, err := result.RowsAffected()
	if err != nil {
		return 0, fmt.Errorf("read affected row count: %w", err)
	}
	if affected < 0 {
		return 0, fmt.Errorf("driver returned negative affected row count %s", strconv.FormatInt(affected, 10))
	}
	return uint64(affected), nil
}
