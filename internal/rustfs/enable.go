package rustfs

import (
	"fmt"
	"time"

	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
)

// SetEnabled flips the registry Enabled flag and signals the daemon to
// reconcile. Returns an error if the daemon could not be signaled so
// that the exit code reflects whether the supervisor picked up the change.
func SetEnabled(enabled bool) error {
	reg, err := registry.Load()
	if err != nil {
		return fmt.Errorf("cannot load registry: %w", err)
	}
	inst, ok := reg.Services[ServiceKey()]
	if !ok {
		return fmt.Errorf("%s not registered (run `pv %s:install` first)", ServiceKey(), Binary().Name)
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

// Restart waits for the process to exit before re-enabling. Without the
// wait, two SignalDaemon calls can be coalesced into a no-op by the supervisor.
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
