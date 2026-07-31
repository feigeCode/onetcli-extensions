package runner

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"time"

	"navop-db-ipc-drivers/internal/dbipc"
	"navop-db-ipc-drivers/internal/ipc"
)

func TestRunConnectsToHostSocketAndServesLifecycle(t *testing.T) {
	if os.Getenv("DBIPC_RUNNER_CHILD") == "1" {
		if err := Run(dbipc.DriverSpec{
			ID:            "runner-test",
			Name:          "Runner Test",
			SQLDriverName: "runner-test",
			DefaultPort:   1,
			BuildDSN: func(dbipc.Config) (string, error) {
				return "runner-test", nil
			},
		}); err != nil {
			t.Fatalf("Run returned error: %v", err)
		}
		return
	}

	socketName := fmt.Sprintf("navop-runner-test-%d.sock", time.Now().UnixNano())
	listener, cleanup := listenTestSocket(t, socketName)
	defer cleanup()

	cmd := exec.Command(os.Args[0], "-test.run=^TestRunConnectsToHostSocketAndServesLifecycle$")
	cmd.Env = append(os.Environ(), "DBIPC_RUNNER_CHILD=1", ipc.SocketEnvVar+"="+socketName)
	var childOut bytes.Buffer
	cmd.Stdout = &childOut
	cmd.Stderr = &childOut
	if err := cmd.Start(); err != nil {
		t.Fatalf("failed to start child test process: %v", err)
	}

	conn := acceptWithTimeout(t, listener, 5*time.Second)
	defer conn.Close()

	if err := ipc.WriteFrame(conn, ipc.Message{
		ID:     json.RawMessage(`1`),
		Method: "init",
		Params: json.RawMessage(`{
			"host_version":"0.10.0",
			"api_offered":{"database":"1.0"},
			"instance_id":"runner-test",
			"config":{}
		}`),
	}); err != nil {
		t.Fatalf("failed to write init frame: %v", err)
	}
	initResp, err := ipc.ReadFrame(conn)
	if err != nil {
		t.Fatalf("failed to read init response: %v", err)
	}
	if initResp.Error != nil {
		t.Fatalf("init response returned error: %#v", initResp.Error)
	}

	if err := ipc.WriteFrame(conn, ipc.Message{
		ID:     json.RawMessage(`2`),
		Method: "shutdown",
		Params: json.RawMessage(`{"grace_ms":1000}`),
	}); err != nil {
		t.Fatalf("failed to write shutdown frame: %v", err)
	}
	shutdownResp, err := ipc.ReadFrame(conn)
	if err != nil {
		t.Fatalf("failed to read shutdown response: %v", err)
	}
	if shutdownResp.Error != nil {
		t.Fatalf("shutdown response returned error: %#v", shutdownResp.Error)
	}

	waitCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	done := make(chan error, 1)
	go func() {
		done <- cmd.Wait()
	}()
	select {
	case err := <-done:
		if err != nil {
			t.Fatalf("child exited with error: %v\n%s", err, childOut.String())
		}
	case <-waitCtx.Done():
		_ = cmd.Process.Kill()
		t.Fatalf("child did not exit after shutdown\n%s", childOut.String())
	}
}

func listenTestSocket(t *testing.T, socketName string) (net.Listener, func()) {
	t.Helper()
	switch runtime.GOOS {
	case "linux":
		listener, err := net.Listen("unix", "\x00"+socketName)
		if err != nil {
			skipSocketPermissionError(t, err)
			t.Fatalf("failed to listen on abstract unix socket: %v", err)
		}
		return listener, func() { _ = listener.Close() }
	case "darwin", "freebsd", "openbsd", "netbsd":
		path := filepath.Join("/tmp", socketName)
		_ = os.Remove(path)
		listener, err := net.Listen("unix", path)
		if err != nil {
			skipSocketPermissionError(t, err)
			t.Fatalf("failed to listen on unix socket %s: %v", path, err)
		}
		return listener, func() {
			_ = listener.Close()
			_ = os.Remove(path)
		}
	default:
		t.Skipf("local socket test is not supported on %s", runtime.GOOS)
		panic("unreachable")
	}
}

func skipSocketPermissionError(t *testing.T, err error) {
	t.Helper()
	if strings.Contains(err.Error(), "operation not permitted") {
		t.Skipf("local socket bind is not permitted in this environment: %v", err)
	}
}

func acceptWithTimeout(t *testing.T, listener net.Listener, timeout time.Duration) net.Conn {
	t.Helper()
	type acceptResult struct {
		conn net.Conn
		err  error
	}
	done := make(chan acceptResult, 1)
	go func() {
		conn, err := listener.Accept()
		done <- acceptResult{conn: conn, err: err}
	}()
	select {
	case result := <-done:
		if result.err != nil {
			t.Fatalf("failed to accept child connection: %v", result.err)
		}
		return result.conn
	case <-time.After(timeout):
		t.Fatalf("timed out waiting for child socket connection")
		panic("unreachable")
	}
}
