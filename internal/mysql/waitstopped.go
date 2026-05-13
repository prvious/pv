package mysql

import (
	"fmt"
	"net"
	"os"
	"strconv"
	"syscall"
	"time"
)

// WaitStopped polls the mysql version's TCP port until connections are
// refused, then waits for the PID file to disappear (confirming the
// process has fully exited). Used by uninstall/update before destructive
// on-disk operations: mysqld can close its listener while still flushing
// InnoDB redo logs, so the port closing is necessary but not sufficient.
func WaitStopped(version string, timeout time.Duration) error {
	port, err := PortFor(version)
	if err != nil {
		return err
	}
	addr := fmt.Sprintf("127.0.0.1:%d", port)
	deadline := time.Now().Add(timeout)

	// Phase 1: wait for TCP port to close.
	for time.Now().Before(deadline) {
		c, err := net.DialTimeout("tcp", addr, 200*time.Millisecond)
		if err != nil {
			break
		}
		c.Close()
		time.Sleep(200 * time.Millisecond)
	}

	// Phase 2: wait for PID file to disappear or process to exit.
	pidFile := "/tmp/pv-mysql-" + version + ".pid"
	for time.Now().Before(deadline) {
		data, err := os.ReadFile(pidFile)
		if err != nil {
			return nil
		}
		pid, _ := strconv.Atoi(string(data))
		if pid > 0 && !processExists(pid) {
			return nil
		}
		time.Sleep(200 * time.Millisecond)
	}
	return fmt.Errorf("mysql %s did not stop within %s", version, timeout)
}

func processExists(pid int) bool {
	return syscall.Kill(pid, 0) == nil
}
