package dbipc

import (
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"testing"

	gokb "gitea.com/kingbase/gokb"
	dm "gitee.com/chunanyong/dm"
	mysql "github.com/go-sql-driver/mysql"
	"github.com/sijms/go-ora/v2/network"

	"navop-db-ipc-drivers/internal/ipc"
)

func TestProtocolErrorPreservesKingbaseServerFields(t *testing.T) {
	err := fmt.Errorf("execute alter table: %w", gokb.Error{
		Severity:   "ERROR",
		Code:       gokb.ErrorCode("23505"),
		Message:    "duplicate key value violates unique constraint",
		Detail:     "Key (id)=(1) already exists.",
		Hint:       "Choose another id.",
		Schema:     "public",
		Table:      "accounts",
		Column:     "id",
		Constraint: "accounts_pkey",
	})

	protocolError := protocolErrorFromError(ErrSQLSyntax, err)
	data := decodeProtocolErrorData(t, protocolError.Data)

	if protocolError.Message == "kb: duplicate key value violates unique constraint" {
		t.Fatalf("expected detailed Kingbase message, got %q", protocolError.Message)
	}
	if data.SQLState != "23505" || data.Schema != "public" || data.Table != "accounts" ||
		data.Column != "id" || data.Constraint != "accounts_pkey" {
		t.Fatalf("unexpected Kingbase error data: %#v", data)
	}
	if data.Extra["detail"] != "Key (id)=(1) already exists." {
		t.Fatalf("missing Kingbase detail: %#v", data.Extra)
	}
}

func TestProtocolErrorPreservesVendorCodes(t *testing.T) {
	tests := []struct {
		name       string
		err        error
		vendorCode int64
		sqlstate   string
	}{
		{
			name: "mysql",
			err: &mysql.MySQLError{
				Number:   1062,
				SQLState: [5]byte{'2', '3', '0', '0', '0'},
				Message:  "Duplicate entry '1' for key 'PRIMARY'",
			},
			vendorCode: 1062,
			sqlstate:   "23000",
		},
		{
			name:       "dm",
			err:        &dm.DmError{ErrCode: -6602, ErrText: "unique constraint violated"},
			vendorCode: -6602,
		},
		{
			name:       "oracle",
			err:        &network.OracleError{ErrCode: 1, ErrMsg: "ORA-00001: unique constraint violated"},
			vendorCode: 1,
		},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			protocolError := protocolErrorFromError(ErrSQLSyntax, test.err)
			data := decodeProtocolErrorData(t, protocolError.Data)
			if data.VendorCode == nil || *data.VendorCode != test.vendorCode {
				t.Fatalf("unexpected vendor code: %#v", data.VendorCode)
			}
			if data.SQLState != test.sqlstate {
				t.Fatalf("unexpected SQLSTATE: %q", data.SQLState)
			}
			if protocolError.Message == "" {
				t.Fatal("database error message must not be empty")
			}
		})
	}
}

func TestProtocolErrorPreservesUnknownErrorChain(t *testing.T) {
	root := errors.New("server returned a detailed failure")
	err := fmt.Errorf("execute statement: %w", root)

	protocolError := protocolErrorFromError(ErrSQLSyntax, err)
	data := decodeProtocolErrorData(t, protocolError.Data)
	chain, ok := data.Extra["chain"].([]any)
	if !ok || len(chain) < 2 {
		t.Fatalf("expected wrapped error chain, got %#v", data.Extra["chain"])
	}
	if protocolError.Message != "execute statement: server returned a detailed failure" {
		t.Fatalf("unexpected message: %q", protocolError.Message)
	}
}

func TestProtocolErrorHandlesNonComparableErrorValues(t *testing.T) {
	err := sliceBackedError{details: []string{"native", "detail"}}
	protocolError := protocolErrorFromError(ErrSQLSyntax, err)
	data := decodeProtocolErrorData(t, protocolError.Data)
	if protocolError.Message != "native: detail" {
		t.Fatalf("unexpected message: %q", protocolError.Message)
	}
	if data.Extra["source"] != "native: detail" {
		t.Fatalf("missing source detail: %#v", data.Extra)
	}
}

type sliceBackedError struct {
	details []string
}

func (e sliceBackedError) Error() string {
	return strings.Join(e.details, ": ")
}

func decodeProtocolErrorData(t *testing.T, raw json.RawMessage) ipc.ErrorData {
	t.Helper()
	var data ipc.ErrorData
	if err := json.Unmarshal(raw, &data); err != nil {
		t.Fatalf("decode protocol error data: %v", err)
	}
	return data
}
