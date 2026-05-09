package svchooks

import (
	"fmt"
	"time"

	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/services"
)

// Restart toggles svc off, waits for the supervisor to confirm the
// process has exited, and toggles it back on. Without the wait, two
// SignalDaemon calls in quick succession can be coalesced by the
// supervisor into a no-op — the second reconcile sees the final
// wanted-state as enabled and never stops the process.
func Restart(reg *registry.Registry, svc services.BinaryService) error {
	if err := SetEnabled(reg, svc, false); err != nil {
		return err
	}
	if server.IsRunning() {
		if err := WaitStopped(svc, 30*time.Second); err != nil {
			return fmt.Errorf("waiting for %s to stop: %w", svc.DisplayName(), err)
		}
	}
	// SetEnabled mutated reg via the in-memory pointer. Reload so the
	// second call sees the persisted state and doesn't carry stale
	// pointer aliasing into the enable step.
	reloaded, err := registry.Load()
	if err != nil {
		return fmt.Errorf("cannot reload registry: %w", err)
	}
	return SetEnabled(reloaded, svc, true)
}
