package cmd

import (
	"fmt"
	"os"
	"strings"

	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var serviceListCmd = &cobra.Command{
	Use:   "service:list",
	Short: "List all services",
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

		var rows [][]string
		for key, svc := range svcs {
			// Determine service name from key.
			svcName := key
			if idx := strings.Index(key, ":"); idx > 0 {
				svcName = key[:idx]
			}

			status := "added"
			if svc.ContainerID != "" {
				status = "running"
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

func init() {
	rootCmd.AddCommand(serviceListCmd)
}
