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

		// Check project exists before removing.
		if reg.Find(name) == nil {
			return fmt.Errorf("project %q is not linked", name)
		}

		if err := reg.Remove(name); err != nil {
			return err
		}

		if err := reg.Save(); err != nil {
			return fmt.Errorf("cannot save registry: %w", err)
		}

		if err := caddy.RemoveSiteConfig(name); err != nil {
			return fmt.Errorf("cannot remove site config: %w", err)
		}
		if err := caddy.GenerateCaddyfile(); err != nil {
			return fmt.Errorf("cannot generate Caddyfile: %w", err)
		}

		settings, _ := config.LoadSettings()
		tld := "test"
		if settings != nil {
			tld = settings.TLD
		}

		// Remove TLS certificate for Vite dev server.
		certs.RemoveSiteTLS(name + "." + tld)

		domain := "https://" + name + "." + tld

		fmt.Fprintln(os.Stderr)
		ui.Success(fmt.Sprintf("Unlinked %s", ui.Purple.Bold(true).Render(domain)))

		if server.IsRunning() {
			if err := server.ReconfigureServer(); err != nil {
				ui.Fail(fmt.Sprintf("Could not reconfigure server: %v", err))
			}
		}

		return nil
	},
}

func init() {
	rootCmd.AddCommand(unlinkCmd)
}
