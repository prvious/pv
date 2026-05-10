package redis

import (
	"fmt"
	"net"
	"time"
)

// WaitStopped polls the redis TCP port until connections are refused,
// or until timeout. Used by uninstall/update before destructive on-disk
// operations. A fixed sleep doesn't account for "redis is in the middle
// of an RDB save" — verify shutdown directly.
//
// 10s is plenty for a typical dev-load redis: even a forced BGSAVE on
// a multi-GB dataset finishes in seconds.
func WaitStopped(timeout time.Duration) error {
	addr := fmt.Sprintf("127.0.0.1:%d", PortFor())
	deadline := time.Now().Add(timeout)
	for time.Now().Before(deadline) {
		c, err := net.DialTimeout("tcp", addr, 200*time.Millisecond)
		if err != nil {
			return nil
		}
		c.Close()
		time.Sleep(200 * time.Millisecond)
	}
	return fmt.Errorf("redis did not stop within %s", timeout)
}
