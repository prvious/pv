package redis

import (
	"fmt"
	"net"
	"time"
)

func WaitStopped(version string, timeout time.Duration) error {
	if err := ValidateVersion(version); err != nil {
		return err
	}
	addr := fmt.Sprintf("127.0.0.1:%d", PortFor(version))
	deadline := time.Now().Add(timeout)
	for time.Now().Before(deadline) {
		c, err := net.DialTimeout("tcp", addr, 200*time.Millisecond)
		if err != nil {
			return nil
		}
		c.Close()
		time.Sleep(200 * time.Millisecond)
	}
	return fmt.Errorf("redis %s did not stop within %s", version, timeout)
}
