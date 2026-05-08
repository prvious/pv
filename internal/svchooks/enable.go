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
// (enabled=false). Returns an error if the service is not registered.
//
// Reports outcome via the ui helpers — callers don't need to print
// anything on success.
func SetEnabled(reg *registry.Registry, svc services.BinaryService, enabled bool) error {
	name := svc.Name()
	inst, ok := reg.Services[name]
	if !ok {
		return fmt.Errorf("%s not registered (run `pv %s:install` first)", name, svc.Binary().Name)
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
		// Registry is already updated but the daemon did not pick it up.
		// Don't claim "reconciled" — that would lie to the user about
		// the actual supervisor state.
		ui.Fail(fmt.Sprintf("%s %s in registry, but could not signal daemon: %v", svc.DisplayName(), verb, err))
		ui.Subtle("Run `pv restart` to load the change.")
		return nil
	}
	ui.Success(fmt.Sprintf("%s %s; daemon reconciled", svc.DisplayName(), verb))
	return nil
}
