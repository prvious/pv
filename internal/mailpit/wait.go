package mailpit

import (
	"fmt"
	"time"

	"github.com/prvious/pv/internal/server"
)

// WaitStopped polls daemon-status.json until mailpit is no longer
// running, or until timeout. Used before destructive on-disk operations
// during uninstall and between the disable/enable halves of a restart so
// we don't yank the binary out from under a live process or coalesce
// two reconcile signals into a no-op. Returns nil when the daemon
// snapshot reports the binary is not running, or when the snapshot is
// missing (daemon stopped — already stopped from our perspective).
func WaitStopped(timeout time.Duration) error {
	binaryName := Binary().Name
	deadline := time.Now().Add(timeout)
	for time.Now().Before(deadline) {
		snap, err := server.ReadDaemonStatus()
		if err != nil {
			return nil
		}
		st, ok := snap.Supervised[binaryName]
		if !ok || !st.Running {
			return nil
		}
		time.Sleep(200 * time.Millisecond)
	}
	return fmt.Errorf("%s did not stop within %s", DisplayName(), timeout)
}
