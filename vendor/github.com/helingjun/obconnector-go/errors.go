package oceanbase

import (
	"database/sql/driver"
	"encoding/binary"
	"errors"
	"fmt"
	"io"
	"net"
	"os"
)

type ServerError struct {
	Number   uint16
	SQLState string
	Message  string
}

func (e *ServerError) Error() string {
	if e.SQLState == "" {
		return fmt.Sprintf("oceanbase: error %d: %s", e.Number, e.Message)
	}
	return fmt.Sprintf("oceanbase: error %d (%s): %s", e.Number, e.SQLState, e.Message)
}

func parseServerError(packet []byte) error {
	if len(packet) < 3 || packet[0] != 0xff {
		return fmt.Errorf("malformed error packet")
	}
	err := &ServerError{Number: binary.LittleEndian.Uint16(packet[1:3])}
	pos := 3
	if len(packet) > pos && packet[pos] == '#' && len(packet) >= pos+6 {
		err.SQLState = string(packet[pos+1 : pos+6])
		pos += 6
	}
	if len(packet) > pos {
		err.Message = string(packet[pos:])
	}
	return err
}

func isBadConnError(err error) bool {
	if err == nil {
		return false
	}
	if errors.Is(err, driver.ErrBadConn) ||
		errors.Is(err, io.EOF) ||
		errors.Is(err, io.ErrUnexpectedEOF) ||
		errors.Is(err, net.ErrClosed) ||
		errors.Is(err, os.ErrDeadlineExceeded) {
		return true
	}
	var netErr net.Error
	return errors.As(err, &netErr)
}
