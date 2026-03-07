package cmd

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var serviceEnvCmd = &cobra.Command{
	Use:     "service:env [service]",
	GroupID: "service",
	Short: "Print environment variables for a service",
	Args:  cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}

		// Determine project name from current directory.
		cwd, _ := os.Getwd()
		projectName := sanitizeProjectName(filepath.Base(cwd))

		if len(args) == 0 {
			// Print env for all services.
			svcs := reg.ListServices()
			if len(svcs) == 0 {
				fmt.Fprintln(os.Stderr)
				ui.Subtle("No services configured.")
				fmt.Fprintln(os.Stderr)
				return nil
			}

			fmt.Fprintln(os.Stderr)
			for key, instance := range svcs {
				svcName := key
				if idx := strings.Index(key, ":"); idx > 0 {
					svcName = key[:idx]
				}
				svc, err := services.Lookup(svcName)
				if err != nil {
					continue
				}
				envVars := svc.EnvVars(projectName, instance.Port)
				printEnvVars(key, envVars)
			}
			return nil
		}

		key := args[0]
		instance := reg.FindService(key)
		if instance == nil {
			return fmt.Errorf("service %q not found", key)
		}

		svcName := key
		if idx := strings.Index(key, ":"); idx > 0 {
			svcName = key[:idx]
		}
		svc, err := services.Lookup(svcName)
		if err != nil {
			return err
		}

		envVars := svc.EnvVars(projectName, instance.Port)
		fmt.Fprintln(os.Stderr)
		printEnvVars(key, envVars)

		return nil
	},
}

func printEnvVars(key string, envVars map[string]string) {
	fmt.Fprintf(os.Stderr, "  %s\n", ui.Muted.Render("# "+key))
	for k, v := range envVars {
		fmt.Fprintf(os.Stderr, "  %s=%s\n", k, v)
	}
	fmt.Fprintln(os.Stderr)
}

// sanitizeProjectName converts a directory name to a database-safe name.
func sanitizeProjectName(name string) string {
	return strings.ReplaceAll(name, "-", "_")
}

func init() {
	rootCmd.AddCommand(serviceEnvCmd)
}
