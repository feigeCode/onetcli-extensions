package ipc

import (
	"encoding/binary"
	"encoding/json"
	"fmt"
	"io"
)

const (
	JSONRPCVersion = "2.0"
	MaxFrameBytes  = 64 * 1024 * 1024
)

type Message struct {
	JSONRPC string          `json:"jsonrpc"`
	ID      json.RawMessage `json:"id,omitempty"`
	Method  string          `json:"method,omitempty"`
	Params  json.RawMessage `json:"params,omitempty"`
	Result  json.RawMessage `json:"result,omitempty"`
	Error   *ProtocolError  `json:"error,omitempty"`
}

type ProtocolError struct {
	Code    int32           `json:"code"`
	Message string          `json:"message"`
	Data    json.RawMessage `json:"data,omitempty"`
}

type ErrorData struct {
	StartOffset *uint32        `json:"start_offset,omitempty"`
	EndOffset   *uint32        `json:"end_offset,omitempty"`
	Database    string         `json:"database,omitempty"`
	Schema      string         `json:"schema,omitempty"`
	Table       string         `json:"table,omitempty"`
	Column      string         `json:"column,omitempty"`
	Constraint  string         `json:"constraint,omitempty"`
	SQLState    string         `json:"sqlstate,omitempty"`
	VendorCode  *int64         `json:"vendor_code,omitempty"`
	Retryable   *bool          `json:"retryable,omitempty"`
	Extra       map[string]any `json:"extra,omitempty"`
}

func ReadFrame(r io.Reader) (Message, error) {
	var prefix [4]byte
	if _, err := io.ReadFull(r, prefix[:]); err != nil {
		return Message{}, err
	}

	n := binary.LittleEndian.Uint32(prefix[:])
	if n > MaxFrameBytes {
		return Message{}, fmt.Errorf("frame length %d exceeds limit %d", n, MaxFrameBytes)
	}

	payload := make([]byte, n)
	if _, err := io.ReadFull(r, payload); err != nil {
		return Message{}, err
	}

	var msg Message
	if err := json.Unmarshal(payload, &msg); err != nil {
		return Message{}, err
	}
	return msg, nil
}

func WriteFrame(w io.Writer, msg Message) error {
	if msg.JSONRPC == "" {
		msg.JSONRPC = JSONRPCVersion
	}

	payload, err := json.Marshal(msg)
	if err != nil {
		return err
	}
	if len(payload) > MaxFrameBytes {
		return fmt.Errorf("payload length %d exceeds limit %d", len(payload), MaxFrameBytes)
	}

	var prefix [4]byte
	binary.LittleEndian.PutUint32(prefix[:], uint32(len(payload)))
	if _, err := w.Write(prefix[:]); err != nil {
		return err
	}
	_, err = w.Write(payload)
	return err
}
