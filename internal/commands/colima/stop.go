package colima

import (
	"fmt"

	internalcolima "github.com/prvious/pv/internal/colima"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

// StopServicesFunc is set at init time by the cmd layer to break the
// import cycle between commands/colima and commands/service.
var StopServicesFunc func() error

var stopCmd = &cobra.Command{
	Use:     "colima:stop",
	GroupID: "colima",
	Short:   "Stop all services and shut down the Colima VM",
	RunE: func(cmd *cobra.Command, args []string) error {
		if !internalcolima.IsRunning() {
			ui.Subtle("Colima is not running")
			return nil
		}

		// Stop all service containers first. Non-fatal — if Docker is
		// unreachable the VM stop will still clean everything up.
		if StopServicesFunc != nil {
			if err := StopServicesFunc(); err != nil {
				ui.Fail(fmt.Sprintf("Could not stop services: %v", err))
			}
		}

		return ui.Step("Stopping Colima VM...", func() (string, error) {
			if err := internalcolima.Stop(); err != nil {
				return "", fmt.Errorf("cannot stop Colima VM: %w", err)
			}
			return "Colima stopped", nil
		})
	},
}
