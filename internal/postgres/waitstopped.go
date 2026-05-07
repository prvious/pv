package postgres

import (
	"fmt"
	"net"
	"time"
)

// WaitStopped polls the postgres major's TCP port until connections are
// refused, or until timeout. Used by uninstall/update/restart before
// destructive on-disk operations: a fixed sleep doesn't account for WAL
// flush, large shared_buffers, etc., so we verify shutdown directly.
func WaitStopped(major string, timeout time.Duration) error {
	port, err := PortFor(major)
	if err != nil {
		return err
	}
	addr := fmt.Sprintf("127.0.0.1:%d", port)
	deadline := time.Now().Add(timeout)
	for time.Now().Before(deadline) {
		c, err := net.DialTimeout("tcp", addr, 200*time.Millisecond)
		if err != nil {
			return nil
		}
		c.Close()
		time.Sleep(200 * time.Millisecond)
	}
	return fmt.Errorf("postgres %s did not stop within %s", major, timeout)
}
