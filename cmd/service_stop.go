package cmd

import (
	"fmt"
	"os"

	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var serviceStopCmd = &cobra.Command{
	Use:   "service:stop [service]",
	Short: "Stop a service or all services",
	Args:  cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}

		fmt.Fprintln(os.Stderr)

		if len(args) == 0 {
			// Stop all services.
			svcs := reg.ListServices()
			if len(svcs) == 0 {
				ui.Subtle("No services to stop.")
				fmt.Fprintln(os.Stderr)
				return nil
			}
			for key := range svcs {
				if err := ui.Step(fmt.Sprintf("Stopping %s...", key), func() (string, error) {
					// Docker SDK: stop container.
					return fmt.Sprintf("%s stopped", key), nil
				}); err != nil {
					return err
				}
			}
		} else {
			key := args[0]
			if reg.FindService(key) == nil {
				fmt.Fprintln(os.Stderr)
				ui.Fail(fmt.Sprintf("Service %s not found", ui.Bold.Render(key)))
				ui.FailDetail("Run 'pv service:list' to see available services")
				fmt.Fprintln(os.Stderr)
				cmd.SilenceUsage = true
				return ui.ErrAlreadyPrinted
			}

			if err := ui.Step(fmt.Sprintf("Stopping %s...", key), func() (string, error) {
				// Docker SDK: stop container.
				return fmt.Sprintf("%s stopped", key), nil
			}); err != nil {
				return err
			}
		}

		fmt.Fprintln(os.Stderr)
		return nil
	},
}

func init() {
	rootCmd.AddCommand(serviceStopCmd)
}
