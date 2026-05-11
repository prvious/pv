package cmd

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/automation/steps"
	"github.com/prvious/pv/internal/caddy"
	"github.com/prvious/pv/internal/certs"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/detection"
	"github.com/prvious/pv/internal/laravel"
	"github.com/prvious/pv/internal/phpenv"
	"github.com/prvious/pv/internal/projectenv"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
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

		// Load pv.yml before any side effects — a malformed pv.yml must fail
		// fast, not after we've already wiped the site config and TLS certs.
		projectCfg, err := config.FindAndLoadProjectConfig(absPath)
		if err != nil {
			return fmt.Errorf("cannot read pv.yml: %w", err)
		}

		if projectCfg.HasSetup() {
			ui.Subtle("pv.yml setup: declared — legacy setup steps skipped")
		}

		name := linkName
		if name == "" {
			name = filepath.Base(absPath)
		}

		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}

		settings, err := config.LoadSettings()
		if err != nil {
			return fmt.Errorf("cannot load settings: %w", err)
		}
		globalPHP := settings.Defaults.PHP

		// Check if project is already linked — if so, update in place.
		relink := reg.Find(name) != nil
		if relink {
			// Clean up old configs before pipeline regenerates them.
			if err := caddy.RemoveSiteConfig(name); err != nil {
				return fmt.Errorf("cannot remove old site config: %w", err)
			}
			hostname := name + "." + settings.Defaults.TLD
			if err := certs.RemoveSiteTLS(hostname); err != nil {
				ui.Subtle(fmt.Sprintf("Could not remove old TLS certs: %v", err))
			}
		}

		projectType := detection.Detect(absPath)

		phpVersion := globalPHP
		if v, err := phpenv.ResolveVersion(absPath); err == nil && v != "" {
			phpVersion = v
		}

		var aliases []string
		if projectCfg != nil {
			aliases = projectCfg.Aliases
		}

		// Register or update project.
		if relink {
			if err := reg.UpdateWith(name, func(p *registry.Project) {
				p.Path = absPath
				p.Type = projectType
				p.PHP = phpVersion
				p.Aliases = aliases
			}); err != nil {
				return err
			}
		} else {
			if err := reg.Add(registry.Project{Name: name, Path: absPath, Type: projectType, PHP: phpVersion, Aliases: aliases}); err != nil {
				return err
			}
		}

		// Build automation context.
		ctx := &automation.Context{
			ProjectPath:   absPath,
			ProjectName:   name,
			ProjectType:   projectType,
			PHPVersion:    phpVersion,
			GlobalPHP:     globalPHP,
			TLD:           settings.Defaults.TLD,
			Registry:      reg,
			Settings:      settings,
			Env:           make(map[string]string),
			ProjectConfig: projectCfg,
		}

		// Load existing .env if present.
		if envVars, err := projectenv.ReadDotEnv(filepath.Join(absPath, ".env")); err == nil {
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
			&steps.ApplyPvYmlServicesStep{},
			&laravel.DetectServicesStep{},
			&steps.ApplyPvYmlEnvStep{},
			&laravel.SetAppURLStep{},
			&laravel.SetViteTLSStep{},
			&laravel.CreateDatabaseStep{},
			&laravel.RunMigrationsStep{},
			&steps.ApplySetupStep{},
		}
		if err := automation.RunPipeline(allSteps, ctx); err != nil {
			return err
		}

		// Print success output.
		typeLabel := ctx.ProjectType
		if typeLabel == "" {
			typeLabel = "unknown"
		}

		action := "Linked"
		if relink {
			action = "Relinked"
		}

		domain := "https://" + name + "." + settings.Defaults.TLD

		fmt.Fprintln(os.Stderr)
		ui.Success(fmt.Sprintf("%s %s", action, ui.Accent.Bold(true).Render(domain)))
		fmt.Fprintln(os.Stderr)
		fmt.Fprintf(os.Stderr, "  %s  %s\n", ui.Muted.Render("Path"), absPath)
		fmt.Fprintf(os.Stderr, "  %s  %s\n", ui.Muted.Render("Type"), typeLabel)
		fmt.Fprintf(os.Stderr, "  %s   %s\n", ui.Muted.Render("PHP"), ui.Green.Render(ctx.PHPVersion))
		fmt.Fprintln(os.Stderr)

		// Signal the daemon to reconcile FrankenPHP instances.
		if server.IsRunning() {
			if err := server.SignalDaemon(); err != nil {
				ui.Subtle(fmt.Sprintf("Could not signal daemon: %v", err))
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
