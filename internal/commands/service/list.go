package service

import (
	"fmt"
	"os"
	"strings"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/container"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var listCmd = &cobra.Command{
	Use:     "service:list",
	GroupID: "service",
	Short:   "List all services",
	RunE: func(cmd *cobra.Command, args []string) error {
		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}

		svcs := reg.ListServices()
		if len(svcs) == 0 {
			fmt.Fprintln(os.Stderr)
			ui.Subtle("No services configured. Run 'pv service:add mysql' to get started.")
			fmt.Fprintln(os.Stderr)
			return nil
		}

		fmt.Fprintln(os.Stderr)

		engine, engineErr := container.NewEngine(config.ColimaSocketPath())
		if engineErr == nil {
			defer engine.Close()
		} else {
			ui.Subtle(fmt.Sprintf("Cannot connect to Docker: %v", engineErr))
		}

		var rows [][]string
		for key, svc := range svcs {
			svcName, version := services.ParseServiceKey(key)

			status := "added"
			if engine != nil {
				svcDef, lookupErr := services.Lookup(svcName)
				if lookupErr != nil {
					ui.Subtle(fmt.Sprintf("Unknown service type %q — cannot check status", svcName))
				} else {
					running, runErr := engine.IsRunning(cmd.Context(), svcDef.ContainerName(version))
					if runErr != nil {
						status = "unknown"
					} else if running {
						status = "running"
					}
				}
			} else {
				status = "unknown"
			}

			portStr := fmt.Sprintf(":%d", svc.Port)
			if svc.ConsolePort > 0 {
				portStr += fmt.Sprintf(", :%d", svc.ConsolePort)
			}

			projects := reg.ProjectsUsingService(svcName)
			projectStr := "-"
			if len(projects) > 0 {
				projectStr = strings.Join(projects, ", ")
			}

			rows = append(rows, []string{key, status, portStr, projectStr})
		}

		ui.Table([]string{"Service", "Status", "Port", "Projects"}, rows)
		fmt.Fprintln(os.Stderr)

		return nil
	},
}
