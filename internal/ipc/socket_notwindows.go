//go:build !windows

package ipc

import (
	"fmt"
	"net"
	"runtime"
)

func dialWindowsPipe(string) (net.Conn, error) {
	return nil, fmt.Errorf("windows named pipe is not supported on %s", runtime.GOOS)
}
