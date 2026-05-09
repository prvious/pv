package svchooks

import (
	"fmt"

	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/ui"
)

// SetEnabled flips the registry Enabled flag for svc and signals the
// daemon to reconcile. Used by start (enabled=true) and stop
// (enabled=false). Returns an error if the service is not registered,
// if its registry entry is from a legacy docker-shaped install, or if
// the daemon could not be signaled (so the exit code reflects whether
// the supervisor actually picked up the change).
//
// Reports outcome via the ui helpers — callers don't need to print
// anything on success.
func SetEnabled(reg *registry.Registry, svc services.BinaryService, enabled bool) error {
	inst, err := requireBinaryEntry(reg, svc)
	if err != nil {
		return err
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
		ui.Success(fmt.Sprintf("%s %s", svc.DisplayName(), verb))
		if enabled {
			ui.Subtle("daemon not running — service will start on next `pv start`")
		}
		return nil
	}

	if err := server.SignalDaemon(); err != nil {
		ui.Subtle("Run `pv restart` to load the change.")
		return fmt.Errorf("%s %s in registry, but could not signal daemon: %w", svc.DisplayName(), verb, err)
	}
	ui.Success(fmt.Sprintf("%s %s; daemon reconciled", svc.DisplayName(), verb))
	return nil
}
