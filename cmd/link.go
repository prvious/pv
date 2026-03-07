package cmd

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/caddy"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/detection"
	"github.com/prvious/pv/internal/phpenv"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var linkName string

var linkCmd = &cobra.Command{
	Use:   "link [path]",
	Short: "Link a project directory",
	Example: `# Link the current directory
pv link

# Link a specific path
pv link ~/Code/myapp

# Link with a custom name
pv link --name=myapp ~/Code/myapp`,
	Args: cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		path := "."
		if len(args) > 0 {
			path = args[0]
		}

		absPath, err := filepath.Abs(path)
		if err != nil {
			return fmt.Errorf("cannot resolve path: %w", err)
		}

		info, err := os.Stat(absPath)
		if err != nil {
			return fmt.Errorf("path does not exist: %w", err)
		}
		if !info.IsDir() {
			return fmt.Errorf("%s is not a directory", absPath)
		}

		name := linkName
		if name == "" {
			name = filepath.Base(absPath)
		}

		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}

		projectType := detection.Detect(absPath)

		// Resolve PHP version for this project.
		settings, err := config.LoadSettings()
		if err != nil {
			return fmt.Errorf("cannot load settings: %w", err)
		}
		globalPHP := settings.GlobalPHP

		phpVersion := globalPHP
		if v, err := phpenv.ResolveVersion(absPath); err == nil && v != "" {
			phpVersion = v
		}

		project := registry.Project{Name: name, Path: absPath, Type: projectType, PHP: phpVersion}

		if existing := reg.Find(name); existing != nil {
			return fmt.Errorf("%s is already linked at %s\nTo re-link, run: pv unlink %s && pv link %s", name, existing.Path, name, path)
		}
		if err := reg.Add(project); err != nil {
			return err
		}

		if err := reg.Save(); err != nil {
			return fmt.Errorf("cannot save registry: %w", err)
		}

		if err := caddy.GenerateSiteConfig(project, globalPHP); err != nil {
			return fmt.Errorf("cannot generate site config: %w", err)
		}
		if err := caddy.GenerateCaddyfile(); err != nil {
			return fmt.Errorf("cannot generate Caddyfile: %w", err)
		}

		typeLabel := projectType
		if typeLabel == "" {
			typeLabel = "unknown"
		}

		domain := "https://" + name + "." + settings.TLD

		fmt.Fprintln(os.Stderr)
		ui.Success(fmt.Sprintf("Linked %s", ui.Purple.Bold(true).Render(domain)))
		fmt.Fprintln(os.Stderr)
		fmt.Fprintf(os.Stderr, "  %s  %s\n", ui.Muted.Render("Path"), absPath)
		fmt.Fprintf(os.Stderr, "  %s  %s\n", ui.Muted.Render("Type"), typeLabel)
		fmt.Fprintf(os.Stderr, "  %s   %s\n", ui.Muted.Render("PHP"), ui.Green.Render(phpVersion))
		fmt.Fprintln(os.Stderr)

		// Detect and bind services.
		detectAndBindServices(absPath, name, reg)

		// Save again in case services were bound.
		if err := reg.Save(); err != nil {
			return fmt.Errorf("cannot save registry: %w", err)
		}

		if server.IsRunning() {
			if err := server.ReconfigureServer(); err != nil {
				fmt.Fprintf(os.Stderr, "  %s %s\n", ui.Red.Render("!"), ui.Muted.Render(fmt.Sprintf("Could not reconfigure server: %v", err)))
			}
			if phpVersion != "" && phpVersion != globalPHP {
				ui.Subtle("Restart the server to serve this project: pv stop && pv start")
			}
		}

		return nil
	},
}

func init() {
	linkCmd.Flags().StringVar(&linkName, "name", "", "Custom name for the project")
	rootCmd.AddCommand(linkCmd)
}
