package cmd

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/caddy"
	"github.com/prvious/pv/internal/certs"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var unlinkCmd = &cobra.Command{
	Use:     "unlink [name]",
	GroupID: "core",
	Short:   "Unlink a project",
	Example: `# Unlink by name
pv unlink myapp

# Unlink the current directory
pv unlink`,
	Args: cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}

		var name string
		if len(args) > 0 {
			name = args[0]
		} else {
			cwd, err := os.Getwd()
			if err != nil {
				return fmt.Errorf("cannot get working directory: %w", err)
			}
			absPath, _ := filepath.Abs(cwd)
			p := reg.FindByPath(absPath)
			if p == nil {
				return fmt.Errorf("current directory is not a linked project")
			}
			name = p.Name
		}

		// Check project exists before removing and capture details for cleanup.
		project := reg.Find(name)
		if project == nil {
			return fmt.Errorf("project %q is not linked", name)
		}
		projectPath := project.Path

		if err := reg.Remove(name); err != nil {
			return err
		}

		server.UnwatchProject(projectPath)

		if err := reg.Save(); err != nil {
			return fmt.Errorf("cannot save registry: %w", err)
		}

		if err := caddy.RemoveSiteConfig(name); err != nil {
			return fmt.Errorf("cannot remove site config: %w", err)
		}
		if err := caddy.GenerateCaddyfile(); err != nil {
			return fmt.Errorf("cannot generate Caddyfile: %w", err)
		}

		settings, settingsErr := config.LoadSettings()
		tld := "test"
		if settingsErr != nil {
			ui.Subtle(fmt.Sprintf("Could not load settings: %v", settingsErr))
		}
		if settings != nil {
			tld = settings.Defaults.TLD
		}

		// Remove TLS certificate for Vite dev server.
		if err := certs.RemoveSiteTLS(name + "." + tld); err != nil {
			ui.Subtle(fmt.Sprintf("Could not remove Vite TLS certs: %v", err))
		}

		domain := "https://" + name + "." + tld

		fmt.Fprintln(os.Stderr)
		ui.Success(fmt.Sprintf("Unlinked %s", ui.Accent.Bold(true).Render(domain)))

		// Signal the daemon to reconcile — it will stop orphaned secondaries.
		if server.IsRunning() {
			if err := server.SignalDaemon(); err != nil {
				ui.Subtle(fmt.Sprintf("Could not signal daemon: %v", err))
			}
		}

		return nil
	},
}

func init() {
	rootCmd.AddCommand(unlinkCmd)
}
