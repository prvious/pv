package service

import (
	"fmt"
	"os"

	"github.com/prvious/pv/internal/colima"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/container"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

// startService starts an existing container, or recreates it if the container
// does not exist (e.g. service was added while Docker was unavailable).
func startService(cmd *cobra.Command, _ *registry.Registry, key string) (string, error) {
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

	containerName := svcDef.ContainerName(version)
	exists, err := engine.Exists(cmd.Context(), containerName)
	if err != nil {
		return "", fmt.Errorf("cannot check container %s: %w", key, err)
	}

	if exists {
		if err := engine.Start(cmd.Context(), containerName); err != nil {
			return "", fmt.Errorf("cannot start %s: %w", key, err)
		}
	} else {
		// Container doesn't exist — recreate from service definition.
		opts := svcDef.CreateOpts(version)
		dataDir := config.ServiceDataDir(svcName, version)
		if mkErr := os.MkdirAll(dataDir, 0755); mkErr != nil {
			return "", fmt.Errorf("cannot create data directory: %w", mkErr)
		}
		if err := engine.Pull(cmd.Context(), opts.Image); err != nil {
			return "", fmt.Errorf("cannot pull %s: %w", opts.Image, err)
		}
		if _, err := engine.CreateAndStart(cmd.Context(), opts); err != nil {
			return "", fmt.Errorf("cannot create %s: %w", key, err)
		}
	}
	return fmt.Sprintf("%s started", key), nil
}

var startCmd = &cobra.Command{
	Use:     "service:start [service]",
	GroupID: "service",
	Short:   "Start a docker-backed service or all of them",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		if len(args) > 0 {
			if err := redirectIfBinary(args[0], "start"); err != nil {
				return err
			}
		}

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
			for key, inst := range svcs {
				if inst.Kind == "binary" {
					// Binary services are owned by rustfs:* / mailpit:*; the
					// daemon reconciles them on its own ticks.
					continue
				}
				if err := ui.Step(fmt.Sprintf("Starting %s...", key), func() (string, error) {
					return startService(cmd, reg, key)
				}); err != nil {
					return err
				}
				svcName, version := services.ParseServiceKey(key)
				svc, lookupErr := services.Lookup(svcName)
				if lookupErr != nil {
					ui.Subtle(fmt.Sprintf("Could not look up %s for env update: %v", svcName, lookupErr))
				} else {
					updateLinkedProjectsEnv(reg, svcName, svc, version)
				}
			}
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

			if err := ui.Step(fmt.Sprintf("Starting %s...", key), func() (string, error) {
				return startService(cmd, reg, key)
			}); err != nil {
				return err
			}
			svcName, version := services.ParseServiceKey(key)
			svc, lookupErr := services.Lookup(svcName)
			if lookupErr != nil {
				ui.Subtle(fmt.Sprintf("Could not look up %s for env update: %v", svcName, lookupErr))
			} else {
				updateLinkedProjectsEnv(reg, svcName, svc, version)
			}
		}

		fmt.Fprintln(os.Stderr)
		return nil
	},
}
