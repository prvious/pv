package cmd

import (
	"fmt"
	"os"
	"strings"

	"github.com/prvious/pv/internal/caddy"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var serviceDestroyCmd = &cobra.Command{
	Use:   "service:destroy <service>",
	Short: "Stop, remove container, and delete all data for a service",
	Args:  cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		key := args[0]

		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}

		svc := reg.FindService(key)
		if svc == nil {
			fmt.Fprintln(os.Stderr)
			ui.Fail(fmt.Sprintf("Service %s not found", ui.Bold.Render(key)))
			fmt.Fprintln(os.Stderr)
			cmd.SilenceUsage = true
			return ui.ErrAlreadyPrinted
		}

		fmt.Fprintln(os.Stderr)

		// Determine service name and version from key.
		svcName := key
		version := "latest"
		if idx := strings.Index(key, ":"); idx > 0 {
			svcName = key[:idx]
			version = key[idx+1:]
		}

		// Stop + remove container.
		if err := ui.Step(fmt.Sprintf("Destroying %s...", key), func() (string, error) {
			// Docker SDK: stop + remove container.

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
		_ = caddy.GenerateServiceSiteConfigs(reg)

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

func init() {
	rootCmd.AddCommand(serviceDestroyCmd)
}
