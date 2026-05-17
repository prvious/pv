package mysql

import (
	"fmt"
	"net"
	"os"
	"testing"
	"time"
)

func TestWaitStopped_NoPortNoPID_ReturnsImmediately(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	start := time.Now()
	if err := WaitStopped("8.4", 5*time.Second); err != nil {
		t.Fatalf("WaitStopped: %v", err)
	}
	if time.Since(start) > 1*time.Second {
		t.Errorf("WaitStopped took too long: %s", time.Since(start))
	}
}

func TestWaitStopped_PortOpen_TimesOut(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	port, _ := PortFor("8.4")
	ln, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", port))
	if err != nil {
		t.Skipf("cannot bind port %d: %v", port, err)
	}
	defer ln.Close()

	start := time.Now()
	if err := WaitStopped("8.4", 500*time.Millisecond); err == nil {
		t.Fatal("expected timeout")
	}
	if time.Since(start) < 400*time.Millisecond {
		t.Errorf("expected to wait at least 400ms, got %s", time.Since(start))
	}
}

func TestWaitStopped_PortClosed_PIDFileWithDeadProcess_Returns(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	pidFile := "/tmp/pv-mysql-8.4.pid"
	os.WriteFile(pidFile, []byte("999999"), 0o644)
	defer os.Remove(pidFile)

	start := time.Now()
	if err := WaitStopped("8.4", 5*time.Second); err != nil {
		t.Fatalf("WaitStopped: %v", err)
	}
	if time.Since(start) > 1*time.Second {
		t.Errorf("WaitStopped took too long: %s", time.Since(start))
	}
}
