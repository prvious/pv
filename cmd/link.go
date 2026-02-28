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
	"github.com/spf13/cobra"
)

var linkName string

var linkCmd = &cobra.Command{
	Use:   "link [path]",
	Short: "Link a project directory",
	Args:  cobra.MaximumNArgs(1),
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
		phpLabel := ""
		if phpVersion != "" && phpVersion != globalPHP {
			phpLabel = fmt.Sprintf(", PHP %s", phpVersion)
		}
		fmt.Printf("Linked %s → %s (%s%s)\n", name, absPath, typeLabel, phpLabel)

		if server.IsRunning() {
			if err := server.ReconfigureServer(); err != nil {
				fmt.Fprintf(os.Stderr, "Warning: could not reconfigure server: %v\n", err)
			}
			// If this project uses a non-global PHP version, secondary processes
			// need a server restart to pick up the new project.
			if phpVersion != "" && phpVersion != globalPHP {
				fmt.Println("Note: restart the server to serve this project (pv stop && pv start)")
			}
		}

		return nil
	},
}

func init() {
	linkCmd.Flags().StringVar(&linkName, "name", "", "Custom name for the project")
	rootCmd.AddCommand(linkCmd)
}
