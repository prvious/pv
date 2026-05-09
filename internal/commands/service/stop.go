package service

import (
	"fmt"
	"os"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/container"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/svchooks"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var stopCmd = &cobra.Command{
	Use:     "service:stop [service]",
	GroupID: "service",
	Short:   "Stop a docker-backed service or all of them",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		if len(args) > 0 {
			if err := redirectIfBinary(args[0], "stop"); err != nil {
				return err
			}
		}

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
			for key, inst := range svcs {
				if inst.Kind == "binary" {
					// Binary services are owned by rustfs:* / mailpit:* now.
					continue
				}
				if err := ui.Step(fmt.Sprintf("Stopping %s...", key), func() (string, error) {
					svcName, version := services.ParseServiceKey(key)
					svcDef, lookupErr := services.Lookup(svcName)
					if lookupErr != nil {
						return "", lookupErr
					}

					engine, err := container.NewEngine(config.ColimaSocketPath())
					if err != nil {
						return "", fmt.Errorf("cannot connect to Docker: %w", err)
					}
					defer engine.Close()

					if err := engine.Stop(cmd.Context(), svcDef.ContainerName(version)); err != nil {
						return "", fmt.Errorf("cannot stop %s: %w", key, err)
					}
					return fmt.Sprintf("%s stopped", key), nil
				}); err != nil {
					return err
				}
			}
			// Apply fallbacks for each stopped Docker service. Binary services
			// are skipped by applyStopAllFallbacks because they were not stopped.
			applyStopAllFallbacks(reg)
		} else {
			key := args[0]
			var resolveErr error
			key, resolveErr = reg.ResolveServiceKey(key)
			if resolveErr != nil {
				return resolveErr
			}
			if svc, findErr := reg.FindService(key); findErr != nil {
				return findErr
			} else if svc == nil {
				return fmt.Errorf("service %q not found, run 'pv service:list' to see available services", key)
			}

			if err := ui.Step(fmt.Sprintf("Stopping %s...", key), func() (string, error) {
				svcName, version := services.ParseServiceKey(key)
				svcDef, lookupErr := services.Lookup(svcName)
				if lookupErr != nil {
					return "", lookupErr
				}

				engine, err := container.NewEngine(config.ColimaSocketPath())
				if err != nil {
					return "", fmt.Errorf("cannot connect to Docker: %w", err)
				}
				defer engine.Close()

				if err := engine.Stop(cmd.Context(), svcDef.ContainerName(version)); err != nil {
					return "", fmt.Errorf("cannot stop %s: %w", key, err)
				}
				return fmt.Sprintf("%s stopped", key), nil
			}); err != nil {
				return err
			}
			// Apply fallbacks for the stopped service.
			svcName, _ := services.ParseServiceKey(key)
			svchooks.ApplyFallbacksToLinkedProjects(reg, svcName)
		}

		fmt.Fprintln(os.Stderr)
		return nil
	},
}
