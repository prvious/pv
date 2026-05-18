package mailpit

import (
	"fmt"
	"net"
	"time"
)

// WaitStopped polls the Mailpit TCP port until it refuses connections
// or the timeout expires. It returns nil as soon as the port is
// unreachable, or an error if the timeout is exceeded.
func WaitStopped(version string, timeout time.Duration) error {
	if err := ValidateVersion(version); err != nil {
		return err
	}
	addr := fmt.Sprintf("127.0.0.1:%d", Port())
	deadline := time.Now().Add(timeout)
	for time.Now().Before(deadline) {
		c, err := net.DialTimeout("tcp", addr, 200*time.Millisecond)
		if err != nil {
			return nil
		}
		c.Close()
		time.Sleep(200 * time.Millisecond)
	}
	return fmt.Errorf("mailpit %s did not stop within %s", version, timeout)
}
