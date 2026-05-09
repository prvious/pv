package svchooks

import (
	"fmt"
	"time"

	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/services"
)

// WaitStopped polls daemon-status.json until svc is no longer running,
// or until timeout. Used before destructive on-disk operations during
// uninstall and between the disable/enable halves of a restart so we
// don't yank a binary out from under a live process or coalesce two
// reconcile signals into a no-op. Returns nil when the daemon snapshot
// reports the binary is not running, or when the snapshot is missing
// (daemon stopped — already stopped from our perspective).
func WaitStopped(svc services.BinaryService, timeout time.Duration) error {
	binaryName := svc.Binary().Name
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
	return fmt.Errorf("%s did not stop within %s", svc.DisplayName(), timeout)
}

// requireBinaryEntry returns the registry entry for svc, or an error
// pointing the user at the correct next command. Catches both the
// not-installed case and a legacy docker-shaped entry left behind by a
// pv version that predated the binary-service migration.
func requireBinaryEntry(reg *registry.Registry, svc services.BinaryService) (*registry.ServiceInstance, error) {
	name := svc.Name()
	inst, ok := reg.Services[name]
	if !ok {
		return nil, fmt.Errorf("%s not registered (run `pv %s:install` first)", name, svc.Binary().Name)
	}
	if inst.Kind != "binary" {
		return nil, fmt.Errorf(
			"%s is registered as %q from a previous pv version. "+
				"Run `pv uninstall && pv setup` to reset",
			name, inst.Kind,
		)
	}
	return inst, nil
}
