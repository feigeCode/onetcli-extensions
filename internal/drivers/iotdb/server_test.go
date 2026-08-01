package iotdb

import (
	"encoding/json"
	"errors"
	"testing"

	iotdbclient "github.com/apache/iotdb-client-go/client"
	iotdbcommon "github.com/apache/iotdb-client-go/common"

	"navop-db-ipc-drivers/internal/dbipc"
	"navop-db-ipc-drivers/internal/ipc"
)

func TestIoTDBBatchErrorPreservesStatusWithoutMessage(t *testing.T) {
	retryable := true
	batch := iotdbclient.NewBatchError([]*iotdbcommon.TSStatus{{
		Code:      iotdbclient.ExecuteStatementError,
		NeedRetry: &retryable,
	}})

	protocolError := iotdbProtocolError(dbipc.ErrSQLSyntax, batch)
	if protocolError.Message != "IoTDB error code 301" {
		t.Fatalf("unexpected message: %q", protocolError.Message)
	}

	var data ipc.ErrorData
	if err := json.Unmarshal(protocolError.Data, &data); err != nil {
		t.Fatalf("decode error data: %v", err)
	}
	if data.VendorCode == nil || *data.VendorCode != int64(iotdbclient.ExecuteStatementError) {
		t.Fatalf("unexpected vendor code: %#v", data.VendorCode)
	}
	if data.Retryable == nil || !*data.Retryable {
		t.Fatalf("unexpected retryable value: %#v", data.Retryable)
	}
	statuses, ok := data.Extra["statuses"].([]any)
	if !ok || len(statuses) != 1 {
		t.Fatalf("unexpected statuses: %#v", data.Extra["statuses"])
	}
}

func TestIoTDBWrappedErrorPreservesMessageAndChain(t *testing.T) {
	err := errors.New("native IoTDB transport detail")
	wrapped := errors.Join(errors.New("execute failed"), err)

	protocolError := iotdbProtocolError(dbipc.ErrConnectionFailed, wrapped)
	if protocolError.Message == "" || protocolError.Message == "db error" {
		t.Fatalf("unexpected message: %q", protocolError.Message)
	}

	var data ipc.ErrorData
	if err := json.Unmarshal(protocolError.Data, &data); err != nil {
		t.Fatalf("decode error data: %v", err)
	}
	chain, ok := data.Extra["chain"].([]any)
	if !ok || len(chain) == 0 {
		t.Fatalf("missing error chain: %#v", data.Extra)
	}
}

func TestStatusErrorHandlesMultipleErrorWithNilMessage(t *testing.T) {
	status := &iotdbcommon.TSStatus{
		Code: iotdbclient.MultipleError,
		SubStatus: []*iotdbcommon.TSStatus{{
			Code: iotdbclient.ExecuteStatementError,
		}},
	}

	err := statusError(status)
	if err == nil {
		t.Fatal("expected status error")
	}
	protocolError := iotdbProtocolError(dbipc.ErrSQLSyntax, err)
	if protocolError.Message != "IoTDB error code 301" {
		t.Fatalf("unexpected message: %q", protocolError.Message)
	}
}
