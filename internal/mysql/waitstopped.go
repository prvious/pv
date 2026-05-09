package mysql

import (
	"fmt"
	"net"
	"time"
)

// WaitStopped polls the mysql version's TCP port until connections are
// refused, or until timeout. Used by uninstall/update before destructive
// on-disk operations: a fixed sleep doesn't account for InnoDB redo-log
// flush, large buffer pool, etc., so we verify shutdown directly.
func WaitStopped(version string, timeout time.Duration) error {
	port, err := PortFor(version)
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
	return fmt.Errorf("mysql %s did not stop within %s", version, timeout)
}
