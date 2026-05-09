package service

import (
	"fmt"

	"github.com/prvious/pv/internal/caddy"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/container"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/svchooks"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var removeCmd = &cobra.Command{
	Use:     "service:remove <service>",
	GroupID: "service",
	Short:   "Stop and remove a service container (data preserved)",
	Args:    cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		if err := redirectIfBinary(args[0], "uninstall"); err != nil {
			return err
		}

		key := args[0]

		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}

		var resolveKeyErr error
		key, resolveKeyErr = reg.ResolveServiceKey(key)
		if resolveKeyErr != nil {
			return resolveKeyErr
		}

		svc, findErr := reg.FindService(key)
		if findErr != nil {
			return findErr
		}
		if svc == nil {
			return fmt.Errorf("service %q not found", key)
		}

		if err := ui.Step(fmt.Sprintf("Removing %s...", key), func() (string, error) {
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
			if stopErr := engine.Stop(cmd.Context(), containerName); stopErr != nil {
				ui.Subtle(fmt.Sprintf("Warning: graceful stop failed: %v", stopErr))
			}
			if err := engine.Remove(cmd.Context(), containerName); err != nil {
				return "", fmt.Errorf("cannot remove %s: %w", key, err)
			}
			return fmt.Sprintf("%s removed", key), nil
		}); err != nil {
			return err
		}

		// Apply fallbacks and unbind before removing from registry.
		svcName, _ := services.ParseServiceKey(key)
		svchooks.ApplyFallbacksToLinkedProjects(reg, svcName)
		reg.UnbindService(svcName)

		if err := reg.RemoveService(key); err != nil {
			return err
		}
		if err := reg.Save(); err != nil {
			return fmt.Errorf("cannot save registry: %w", err)
		}

		// Regenerate Caddy configs for service consoles.
		if err := caddy.GenerateServiceSiteConfigs(reg); err != nil {
			ui.Subtle(fmt.Sprintf("Could not regenerate service site configs: %v", err))
		}
		if server.IsRunning() {
			if err := server.SignalDaemon(); err != nil {
				ui.Subtle(fmt.Sprintf("Could not signal daemon: %v", err))
			}
		}

		// Determine data path for the message.
		_, version := services.ParseServiceKey(key)
		dataDir := config.ServiceDataDir(svcName, version)

		ui.Subtle(fmt.Sprintf("Data preserved at %s", dataDir))
		ui.Subtle(fmt.Sprintf("Run 'pv service:add %s %s' to start it again.", svcName, version))

		return nil
	},
}
