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
		var resolveErr error
		key, resolveErr = reg.ResolveServiceKey(key)
		if resolveErr != nil {
			return resolveErr
		}

		instance, findErr := reg.FindService(key)
		if findErr != nil {
			return findErr
		}
		if instance == nil {
			return fmt.Errorf("service %q not found", key)
		}

		svcName, version := services.ParseServiceKey(key)

		svc, err := services.Lookup(svcName)
		if err != nil {
			return err
		}

		status := "stopped"
		engine, engineErr := container.NewEngine(config.ColimaSocketPath())
		if engineErr != nil {
			status = "unknown"
			ui.Subtle(fmt.Sprintf("Cannot determine container status: %v", engineErr))
		} else {
			defer engine.Close()
			running, runErr := engine.IsRunning(cmd.Context(), svc.ContainerName(version))
			if runErr != nil {
				status = "unknown"
				ui.Subtle(fmt.Sprintf("Cannot check container status: %v", runErr))
			} else if running {
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
