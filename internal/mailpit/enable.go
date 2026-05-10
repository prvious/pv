package mailpit

import (
	"fmt"
	"time"

	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
)

// SetEnabled flips the registry Enabled flag for mailpit and signals the
// daemon to reconcile. Used by start (enabled=true) and stop
// (enabled=false). Returns an error if the service is not registered or
// if the daemon could not be signaled (so the exit code reflects
// whether the supervisor actually picked up the change).
//
// Reports outcome via the ui helpers — callers don't need to print
// anything on success.
func SetEnabled(enabled bool) error {
	reg, err := registry.Load()
	if err != nil {
		return fmt.Errorf("cannot load registry: %w", err)
	}
	inst, ok := reg.Services[serviceKey]
	if !ok {
		return fmt.Errorf("%s not registered (run `pv %s:install` first)", serviceKey, Binary().Name)
	}

	flag := enabled
	inst.Enabled = &flag
	if err := reg.Save(); err != nil {
		return fmt.Errorf("cannot save registry: %w", err)
	}

	verb := "enabled"
	if !enabled {
		verb = "disabled"
	}

	if !server.IsRunning() {
		ui.Success(fmt.Sprintf("%s %s", DisplayName(), verb))
		if enabled {
			ui.Subtle("daemon not running — service will start on next `pv start`")
		}
		return nil
	}

	if err := server.SignalDaemon(); err != nil {
		ui.Subtle("Run `pv restart` to load the change.")
		return fmt.Errorf("%s %s in registry, but could not signal daemon: %w", DisplayName(), verb, err)
	}
	ui.Success(fmt.Sprintf("%s %s; daemon reconciled", DisplayName(), verb))
	return nil
}

// Restart toggles mailpit off, waits for the supervisor to confirm the
// process has exited, and toggles it back on. Without the wait, two
// SignalDaemon calls in quick succession can be coalesced by the
// supervisor into a no-op — the second reconcile sees the final
// wanted-state as enabled and never stops the process.
func Restart() error {
	if err := SetEnabled(false); err != nil {
		return err
	}
	if server.IsRunning() {
		if err := WaitStopped(30 * time.Second); err != nil {
			return fmt.Errorf("waiting for %s to stop: %w", DisplayName(), err)
		}
	}
	return SetEnabled(true)
}
