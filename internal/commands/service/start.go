package service

import (
	"fmt"
	"os"

	"github.com/prvious/pv/internal/colima"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/container"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
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
			for key, inst := range svcs {
				if inst.Kind == "binary" {
					// Binary services are managed by the daemon; nothing to do here.
					// A subsequent SignalDaemon call will reconcile them.
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
					return fmt.Errorf("%s not registered; run `pv service:add %s` first", name, name)
				}
				tru := true
				inst.Enabled = &tru
				if err := reg.Save(); err != nil {
					return fmt.Errorf("cannot save registry: %w", err)
				}
				if server.IsRunning() {
					if err := server.SignalDaemon(); err != nil {
						ui.Subtle(fmt.Sprintf("Could not signal daemon: %v", err))
					}
					ui.Success(fmt.Sprintf("%s enabled; daemon reconciled", binSvc.DisplayName()))
				} else {
					ui.Success(fmt.Sprintf("%s enabled", binSvc.DisplayName()))
					ui.Subtle("daemon not running — service will start on next `pv start`")
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
