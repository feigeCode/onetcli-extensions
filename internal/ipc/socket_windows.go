//go:build windows

package ipc

import (
	"net"

	"github.com/Microsoft/go-winio"
)

func dialWindowsPipe(socketName string) (net.Conn, error) {
	return winio.DialPipe(windowsPipePath(socketName), nil)
}
