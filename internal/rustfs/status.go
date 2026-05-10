package rustfs

import (
	"fmt"
	"os"

	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
)

// PrintStatus writes a stderr-side detail block — registered, enabled,
// running, PID. Returns no error: an unreadable daemon snapshot is
// reported via a Subtle line and the running row falls back to "unknown"
// so the user can tell that case apart from a real stopped state.
func PrintStatus() error {
	reg, err := registry.Load()
	if err != nil {
		return fmt.Errorf("cannot load registry: %w", err)
	}

	inst, ok := reg.Services[ServiceKey()]
	enabled := true
	registered := ok
	if ok && inst.Enabled != nil {
		enabled = *inst.Enabled
	}

	runningLabel := "false"
	pid := 0
	snap, statErr := server.ReadDaemonStatus()
	switch {
	case statErr != nil && !os.IsNotExist(statErr):
		runningLabel = "unknown"
		ui.Subtle(fmt.Sprintf("Could not read daemon status: %v", statErr))
	case statErr == nil:
		if st, exists := snap.Supervised[Binary().Name]; exists {
			if st.Running {
				runningLabel = "true"
			}
			pid = st.PID
		}
	}

	fmt.Fprintln(os.Stderr)
	fmt.Fprintf(os.Stderr, "  %s  %s\n", ui.Muted.Render("Service"), DisplayName())
	fmt.Fprintf(os.Stderr, "  %s  %v\n", ui.Muted.Render("Registered"), registered)
	fmt.Fprintf(os.Stderr, "  %s  %v\n", ui.Muted.Render("Enabled"), enabled)
	fmt.Fprintf(os.Stderr, "  %s  %s\n", ui.Muted.Render("Running"), runningLabel)
	if pid > 0 {
		fmt.Fprintf(os.Stderr, "  %s  %d\n", ui.Muted.Render("PID"), pid)
	}
	fmt.Fprintln(os.Stderr)
	return nil
}
