package cmd

import (
	"fmt"
	"os"
	"strings"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var serviceStatusCmd = &cobra.Command{
	Use:     "service:status <service>",
	GroupID: "service",
	Short: "Show detailed status for a service",
	Args:  cobra.ExactArgs(1),
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
		if instance.ContainerID != "" {
			status = "running"
		}

		dataDir := config.ServiceDataDir(svcName, version)
		projects := reg.ProjectsUsingService(svcName)

		fmt.Fprintln(os.Stderr)
		fmt.Fprintf(os.Stderr, "  %s\n", ui.Purple.Bold(true).Render(fmt.Sprintf("%s %s", svc.DisplayName(), version)))
		fmt.Fprintf(os.Stderr, "    %s     %s\n", ui.Muted.Render("Status"), status)
		fmt.Fprintf(os.Stderr, "    %s  %s\n", ui.Muted.Render("Container"), svc.ContainerName(version))
		fmt.Fprintf(os.Stderr, "    %s       :%d\n", ui.Muted.Render("Port"), instance.Port)
		if instance.ConsolePort > 0 {
			fmt.Fprintf(os.Stderr, "    %s    :%d\n", ui.Muted.Render("Console"), instance.ConsolePort)
		}
		fmt.Fprintf(os.Stderr, "    %s       %s\n", ui.Muted.Render("Data"), dataDir)

		if len(projects) > 0 {
			fmt.Fprintf(os.Stderr, "    %s   %s\n", ui.Muted.Render("Projects"), strings.Join(projects, ", "))
		}
		fmt.Fprintln(os.Stderr)

		return nil
	},
}

func init() {
	rootCmd.AddCommand(serviceStatusCmd)
}
