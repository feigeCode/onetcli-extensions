package ipc

import (
	"errors"
	"fmt"
	"io"
	"net"
	"os"
	"path/filepath"
	"runtime"
)

const SocketEnvVar = "ONETCLI_EXT_SOCKET"

func SocketNameFromEnvOrArg(args []string) string {
	if socket := os.Getenv(SocketEnvVar); socket != "" {
		return socket
	}
	if len(args) > 1 {
		return args[1]
	}
	return ""
}

func DialHostSocket(socketName string) (net.Conn, error) {
	if socketName == "" {
		return nil, errors.New("empty host socket name")
	}

	switch runtime.GOOS {
	case "linux":
		return net.Dial("unix", "\x00"+socketName)
	case "windows":
		return dialWindowsPipe(socketName)
	case "darwin", "freebsd", "openbsd", "netbsd":
		runUser := filepath.Join("/run/user", fmt.Sprint(os.Getuid()), socketName)
		conn, err := net.Dial("unix", runUser)
		if err == nil {
			return conn, nil
		}
		tmp := filepath.Join("/tmp", socketName)
		return net.Dial("unix", tmp)
	default:
		return nil, fmt.Errorf("local socket is not implemented for %s", runtime.GOOS)
	}
}

func windowsPipePath(socketName string) string {
	return `\\.\pipe\` + socketName
}

func ServeConnected(conn net.Conn, handler func(Message) Message) error {
	defer conn.Close()
	for {
		req, err := ReadFrame(conn)
		if err != nil {
			if errors.Is(err, io.EOF) {
				return nil
			}
			return err
		}
		if len(req.ID) == 0 {
			continue
		}
		resp := handler(req)
		if err := WriteFrame(conn, resp); err != nil {
			return err
		}
		if req.Method == "shutdown" {
			return nil
		}
	}
}
