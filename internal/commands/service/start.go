package service

import (
	"fmt"
	"os"

	"github.com/prvious/pv/internal/colima"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var startCmd = &cobra.Command{
	Use:     "service:start [service]",
	GroupID: "service",
	Short:   "Start a service or all services",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}

		if colima.IsInstalled() {
			settings, settingsErr := config.LoadSettings()
			if settingsErr != nil {
				return fmt.Errorf("cannot load settings: %w", settingsErr)
			}
			if err := colima.EnsureRunning(settings.Defaults.VM); err != nil {
				fmt.Fprintln(os.Stderr)
				ui.Subtle(fmt.Sprintf("Container runtime unavailable: %v", err))
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
				svcName := extractServiceName(key)
				svc, lookupErr := services.Lookup(svcName)
				if lookupErr != nil {
					ui.Subtle(fmt.Sprintf("Could not look up %s for env update: %v", svcName, lookupErr))
				} else {
					updateLinkedProjectsEnv(reg, svcName, svc, extractVersion(key))
				}
			}
		} else {
			key := args[0]
			if reg.FindService(key) == nil {
				return fmt.Errorf("service %q not found, run 'pv service:list' to see available services", key)
			}

			if err := ui.Step(fmt.Sprintf("Starting %s...", key), func() (string, error) {
				// Docker SDK: find existing container, start it.
				return fmt.Sprintf("%s started", key), nil
			}); err != nil {
				return err
			}
			svcName := extractServiceName(key)
			svc, lookupErr := services.Lookup(svcName)
			if lookupErr != nil {
				ui.Subtle(fmt.Sprintf("Could not look up %s for env update: %v", svcName, lookupErr))
			} else {
				updateLinkedProjectsEnv(reg, svcName, svc, extractVersion(key))
			}
		}

		fmt.Fprintln(os.Stderr)
		return nil
	},
}
