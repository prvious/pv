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
// enabled, running, PID. Returns no error: a missing-daemon snapshot
// just suppresses the running/PID rows.
func PrintStatus(reg *registry.Registry, svc services.BinaryService) {
	name := svc.Name()
	inst, ok := reg.Services[name]
	enabled := true
	registered := ok
	if ok && inst.Enabled != nil {
		enabled = *inst.Enabled
	}

	running := false
	pid := 0
	if snap, err := server.ReadDaemonStatus(); err == nil {
		if st, exists := snap.Supervised[svc.Binary().Name]; exists {
			running = st.Running
			pid = st.PID
		}
	}

	fmt.Fprintln(os.Stderr)
	fmt.Fprintf(os.Stderr, "  %s  %s\n", ui.Muted.Render("Service"), svc.DisplayName())
	fmt.Fprintf(os.Stderr, "  %s  %s\n", ui.Muted.Render("Kind"), "binary")
	fmt.Fprintf(os.Stderr, "  %s  %v\n", ui.Muted.Render("Registered"), registered)
	fmt.Fprintf(os.Stderr, "  %s  %v\n", ui.Muted.Render("Enabled"), enabled)
	fmt.Fprintf(os.Stderr, "  %s  %v\n", ui.Muted.Render("Running"), running)
	if pid > 0 {
		fmt.Fprintf(os.Stderr, "  %s  %d\n", ui.Muted.Render("PID"), pid)
	}
	fmt.Fprintln(os.Stderr)
}
