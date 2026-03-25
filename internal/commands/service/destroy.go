package service

import (
	"fmt"
	"os"
	"strings"

	"github.com/prvious/pv/internal/caddy"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/container"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var destroyCmd = &cobra.Command{
	Use:     "service:destroy <service>",
	GroupID: "service",
	Short:   "Stop, remove container, and delete all data for a service",
	Args:    cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		key := args[0]

		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}
		var resolveErr error
		key, resolveErr = reg.ResolveServiceKey(key)
		if resolveErr != nil {
			return resolveErr
		}

		svc := reg.FindService(key)
		if svc == nil {
			return fmt.Errorf("service %q not found", key)
		}

		svcName := extractServiceName(key)
		version := extractVersion(key)

		// Stop + remove container + delete data.
		if err := ui.Step(fmt.Sprintf("Destroying %s...", key), func() (string, error) {
			svcDef, lookupErr := services.Lookup(svcName)
			if lookupErr == nil {
				engine, engineErr := container.NewEngine(config.ColimaSocketPath())
				if engineErr == nil {
					defer engine.Close()
					containerName := svcDef.ContainerName(version)
					if stopErr := engine.Stop(cmd.Context(), containerName); stopErr != nil {
						ui.Subtle(fmt.Sprintf("Warning: could not stop container: %v", stopErr))
					}
					if removeErr := engine.Remove(cmd.Context(), containerName); removeErr != nil {
						return "", fmt.Errorf("cannot remove container %s (data not deleted): %w", containerName, removeErr)
					}
				}
			}

			// Delete data directory.
			dataDir := config.ServiceDataDir(svcName, version)
			if err := os.RemoveAll(dataDir); err != nil {
				return "", fmt.Errorf("cannot delete data: %w", err)
			}

			return fmt.Sprintf("%s destroyed", key), nil
		}); err != nil {
			return err
		}

		// Unbind from all projects.
		projects := reg.ProjectsUsingService(svcName)
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

		if len(projects) > 0 {
			fmt.Fprintf(os.Stderr, "  %s Unbound from: %s\n",
				ui.Muted.Render("!"),
				strings.Join(projects, ", "),
			)
		}
		fmt.Fprintln(os.Stderr)

		return nil
	},
}
