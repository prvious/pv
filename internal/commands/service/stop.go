package service

import (
	"fmt"
	"os"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/container"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var stopCmd = &cobra.Command{
	Use:     "service:stop [service]",
	GroupID: "service",
	Short:   "Stop a service or all services",
	Args:    cobra.MaximumNArgs(1),
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
			for key, inst := range svcs {
				if inst.Kind == "binary" {
					// Binary services are managed by the daemon; nothing to do here.
					// A subsequent SignalDaemon call will reconcile them.
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
			// Apply fallbacks for each stopped service.
			for key := range reg.ListServices() {
				svcName, _ := services.ParseServiceKey(key)
				applyFallbacksToLinkedProjects(reg, svcName)
			}
		} else {
			// Dispatch on service kind. Binary services don't use the versioned-key
			// machinery below; they flip a registry flag and let the daemon reconcile.
			kind, binSvc, _, resErr := resolveKind(reg, args[0])
			if resErr != nil {
				return resErr
			}
			if kind == kindBinary {
				name := binSvc.Name()
				inst, ok := reg.Services[name]
				if !ok {
					return fmt.Errorf("%s not registered", name)
				}
				fls := false
				inst.Enabled = &fls
				if err := reg.Save(); err != nil {
					return fmt.Errorf("cannot save registry: %w", err)
				}
				if server.IsRunning() {
					if err := server.SignalDaemon(); err != nil {
						ui.Subtle(fmt.Sprintf("Could not signal daemon: %v", err))
					}
					ui.Success(fmt.Sprintf("%s disabled; daemon reconciled", binSvc.DisplayName()))
				} else {
					ui.Success(fmt.Sprintf("%s disabled", binSvc.DisplayName()))
				}
				return nil
			}

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
			applyFallbacksToLinkedProjects(reg, svcName)
		}

		fmt.Fprintln(os.Stderr)
		return nil
	},
}
