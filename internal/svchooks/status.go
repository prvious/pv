package svchooks

import (
	"fmt"
	"os"

	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/ui"
)

// PrintStatus writes a stderr-side detail block for svc — registered,
// enabled, running, PID. Returns no error: an unreadable daemon
// snapshot is reported via a Subtle line and the running row falls
// back to "unknown" so the user can tell that case apart from a real
// stopped state.
func PrintStatus(reg *registry.Registry, svc services.BinaryService) {
	name := svc.Name()
	inst, ok := reg.Services[name]
	enabled := true
	registered := ok
	if ok && inst.Enabled != nil {
		enabled = *inst.Enabled
	}

	runningLabel := "false"
	pid := 0
	snap, err := server.ReadDaemonStatus()
	switch {
	case err != nil && !os.IsNotExist(err):
		runningLabel = "unknown"
		ui.Subtle(fmt.Sprintf("Could not read daemon status: %v", err))
	case err == nil:
		if st, exists := snap.Supervised[svc.Binary().Name]; exists {
			if st.Running {
				runningLabel = "true"
			}
			pid = st.PID
		}
	}

	fmt.Fprintln(os.Stderr)
	fmt.Fprintf(os.Stderr, "  %s  %s\n", ui.Muted.Render("Service"), svc.DisplayName())
	fmt.Fprintf(os.Stderr, "  %s  %s\n", ui.Muted.Render("Kind"), "binary")
	fmt.Fprintf(os.Stderr, "  %s  %v\n", ui.Muted.Render("Registered"), registered)
	fmt.Fprintf(os.Stderr, "  %s  %v\n", ui.Muted.Render("Enabled"), enabled)
	fmt.Fprintf(os.Stderr, "  %s  %s\n", ui.Muted.Render("Running"), runningLabel)
	if pid > 0 {
		fmt.Fprintf(os.Stderr, "  %s  %d\n", ui.Muted.Render("PID"), pid)
	}
	fmt.Fprintln(os.Stderr)
}
