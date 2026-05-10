package rustfs

import (
	"fmt"
	"time"

	"github.com/prvious/pv/internal/server"
)

// WaitStopped polls until rustfs is no longer running. Called before
// destructive on-disk ops and between the disable/enable halves of a
// restart to avoid coalescing two reconcile signals into a no-op.
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
