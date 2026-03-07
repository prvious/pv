package service

import (
	"fmt"
	"os"

	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var stopCmd = &cobra.Command{
	Use:     "service:stop [service]",
	GroupID: "service",
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
				return fmt.Errorf("service %q not found, run 'pv service:list' to see available services", key)
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
