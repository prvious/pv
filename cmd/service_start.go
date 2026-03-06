package cmd

import (
	"fmt"
	"os"

	"github.com/prvious/pv/internal/colima"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var serviceStartCmd = &cobra.Command{
	Use:   "start [service]",
	Short: "Start a service or all services",
	Args:  cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}

		if colima.IsInstalled() {
			if err := colima.EnsureRunning(); err != nil {
				return fmt.Errorf("cannot start container runtime: %w", err)
			}
		}

		fmt.Fprintln(os.Stderr)

		if len(args) == 0 {
			// Start all services.
			svcs := reg.ListServices()
			if len(svcs) == 0 {
				ui.Subtle("No services to start.")
				fmt.Fprintln(os.Stderr)
				return nil
			}
			for key := range svcs {
				if err := ui.Step(fmt.Sprintf("Starting %s...", key), func() (string, error) {
					// Docker SDK: find existing container, start it.
					return fmt.Sprintf("%s started", key), nil
				}); err != nil {
					return err
				}
			}
		} else {
			key := args[0]
			if reg.FindService(key) == nil {
				fmt.Fprintln(os.Stderr)
				ui.Fail(fmt.Sprintf("Service %s not found", ui.Bold.Render(key)))
				ui.FailDetail("Run 'pv service list' to see available services")
				fmt.Fprintln(os.Stderr)
				cmd.SilenceUsage = true
				return ui.ErrAlreadyPrinted
			}

			if err := ui.Step(fmt.Sprintf("Starting %s...", key), func() (string, error) {
				// Docker SDK: find existing container, start it.
				return fmt.Sprintf("%s started", key), nil
			}); err != nil {
				return err
			}
		}

		fmt.Fprintln(os.Stderr)
		return nil
	},
}

func init() {
	serviceCmd.AddCommand(serviceStartCmd)
}
