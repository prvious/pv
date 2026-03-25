package service

import (
	"fmt"
	"strings"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/container"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var statusCmd = &cobra.Command{
	Use:     "service:status <service>",
	GroupID: "service",
	Short:   "Show detailed status for a service",
	Args:    cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		key := args[0]

		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}

		instance := reg.FindService(key)
		if instance == nil {
			return fmt.Errorf("service %q not found", key)
		}

		// Parse service name and version from key.
		svcName := key
		version := "latest"
		if idx := strings.Index(key, ":"); idx > 0 {
			svcName = key[:idx]
			version = key[idx+1:]
		}

		svc, err := services.Lookup(svcName)
		if err != nil {
			return err
		}

		status := "stopped"
		engine, engineErr := container.NewEngine(config.ColimaSocketPath())
		if engineErr == nil {
			defer engine.Close()
			if running, err := engine.IsRunning(cmd.Context(), svc.ContainerName(version)); err == nil && running {
				status = "running"
			}
		}

		dataDir := config.ServiceDataDir(svcName, version)
		projects := reg.ProjectsUsingService(svcName)

		rows := [][]string{
			{"Status", status},
			{"Container", svc.ContainerName(version)},
			{"Port", fmt.Sprintf(":%d", instance.Port)},
		}
		if instance.ConsolePort > 0 {
			rows = append(rows, []string{"Console", fmt.Sprintf(":%d", instance.ConsolePort)})
		}
		rows = append(rows, []string{"Data", dataDir})
		if len(projects) > 0 {
			rows = append(rows, []string{"Projects", strings.Join(projects, ", ")})
		}

		ui.Table([]string{svc.DisplayName(), version}, rows)

		return nil
	},
}
