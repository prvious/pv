package cmd

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/automation/steps"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/daemon"
	"github.com/prvious/pv/internal/detection"
	"github.com/prvious/pv/internal/laravel"
	"github.com/prvious/pv/internal/phpenv"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var linkName string

var linkCmd = &cobra.Command{
	Use:     "link [path]",
	GroupID: "core",
	Short:   "Link a project directory",
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

		if existing := reg.Find(name); existing != nil {
			return fmt.Errorf("%s is already linked at %s\nTo re-link, run: pv unlink %s && pv link %s", name, existing.Path, name, path)
		}

		projectType := detection.Detect(absPath)

		settings, err := config.LoadSettings()
		if err != nil {
			return fmt.Errorf("cannot load settings: %w", err)
		}
		globalPHP := settings.Defaults.PHP

		phpVersion := globalPHP
		if v, err := phpenv.ResolveVersion(absPath); err == nil && v != "" {
			phpVersion = v
		}

		// Register project.
		project := registry.Project{Name: name, Path: absPath, Type: projectType, PHP: phpVersion}
		if err := reg.Add(project); err != nil {
			return err
		}

		// Build automation context.
		ctx := &automation.Context{
			ProjectPath: absPath,
			ProjectName: name,
			ProjectType: projectType,
			PHPVersion:  phpVersion,
			GlobalPHP:   globalPHP,
			TLD:         settings.Defaults.TLD,
			Registry:    reg,
			Settings:    settings,
			Env:         make(map[string]string),
		}

		// Load existing .env if present.
		if envVars, err := services.ReadDotEnv(filepath.Join(absPath, ".env")); err == nil {
			ctx.Env = envVars
		}

		// Run the full pipeline.
		allSteps := []automation.Step{
			&steps.InstallPHPStep{},
			&laravel.CopyEnvStep{},
			&laravel.ComposerInstallStep{},
			&laravel.GenerateKeyStep{},
			&laravel.InstallOctaneStep{},
			&steps.GenerateSiteConfigStep{},
			&steps.GenerateCaddyfileStep{},
			&steps.GenerateTLSCertStep{},
			&steps.DetectServicesStep{},
			&laravel.DetectServicesStep{},
			&laravel.SetAppURLStep{},
			&laravel.CreateDatabaseStep{},
			&laravel.RunMigrationsStep{},
		}
		if err := automation.RunPipeline(allSteps, ctx); err != nil {
			return err
		}

		// Print success output.
		typeLabel := ctx.ProjectType
		if typeLabel == "" {
			typeLabel = "unknown"
		}

		domain := "https://" + name + "." + settings.Defaults.TLD

		fmt.Fprintln(os.Stderr)
		ui.Success(fmt.Sprintf("Linked %s", ui.Purple.Bold(true).Render(domain)))
		fmt.Fprintln(os.Stderr)
		fmt.Fprintf(os.Stderr, "  %s  %s\n", ui.Muted.Render("Path"), absPath)
		fmt.Fprintf(os.Stderr, "  %s  %s\n", ui.Muted.Render("Type"), typeLabel)
		fmt.Fprintf(os.Stderr, "  %s   %s\n", ui.Muted.Render("PHP"), ui.Green.Render(ctx.PHPVersion))
		fmt.Fprintln(os.Stderr)

		// Reload/restart server if needed.
		if server.IsRunning() {
			needsRestart := phpVersion != "" && phpVersion != globalPHP
			if needsRestart && daemon.IsLoaded() {
				if err := daemon.Restart(); err != nil {
					ui.Fail(fmt.Sprintf("Could not restart daemon: %v — run 'pv restart' manually", err))
				}
			} else {
				if err := server.ReconfigureServer(); err != nil {
					ui.Fail(fmt.Sprintf("Could not reconfigure server: %v", err))
				}
				if needsRestart {
					ui.Subtle("Stop and restart the server to serve this project: pv stop && pv start")
				}
			}
		}

		server.WatchProject(name, absPath)

		return nil
	},
}

func init() {
	linkCmd.Flags().StringVar(&linkName, "name", "", "Custom name for the project")
	rootCmd.AddCommand(linkCmd)
}
