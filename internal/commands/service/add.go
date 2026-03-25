package service

import (
	"errors"
	"fmt"
	"os"

	"github.com/prvious/pv/internal/caddy"
	"github.com/prvious/pv/internal/colima"
	colimacmds "github.com/prvious/pv/internal/commands/colima"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/container"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var addCmd = &cobra.Command{
	Use:     "service:add <service> [version]",
	GroupID: "service",
	Short:   "Add and start a service",
	Long:    "Add a backing service (mail, mysql, postgres, redis, s3). Optionally specify a version.",
	Example: `# Add MySQL with default version
pv service:add mysql

# Add a specific Redis version
pv service:add redis 7

# Add PostgreSQL
pv service:add postgres 16`,
	Args: cobra.RangeArgs(1, 2),
	RunE: func(cmd *cobra.Command, args []string) error {
		svcName := args[0]
		svc, err := services.Lookup(svcName)
		if err != nil {
			return err
		}

		version := svc.DefaultVersion()
		if len(args) > 1 {
			version = args[1]
		}

		key := services.ServiceKey(svcName, version)

		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}

		if reg.FindService(key) != nil {
			fmt.Fprintln(os.Stderr)
			ui.Success(fmt.Sprintf("%s is already added", ui.Accent.Bold(true).Render(svc.DisplayName()+" "+version)))
			fmt.Fprintln(os.Stderr)
			return nil
		}

		fmt.Fprintln(os.Stderr)

		opts := svc.CreateOpts(version)

		// Ensure Colima is installed (lazy install on first service:add).
		containerReady := false
		if !colima.IsInstalled() {
			if err := colimacmds.RunInstall(); err != nil {
				return fmt.Errorf("cannot install Colima (required for services): %w", err)
			}
		}

		settings, err := config.LoadSettings()
		if err != nil {
			return fmt.Errorf("cannot load settings: %w", err)
		}

		if err := colima.EnsureRunning(settings.Defaults.VM); err != nil {
			ui.Subtle(fmt.Sprintf("Container runtime unavailable: %v", err))
			ui.Subtle("Service registered — container will start when runtime is available.")
		} else {
			// Pull image.
			if err := ui.Step(fmt.Sprintf("Pulling %s...", opts.Image), func() (string, error) {
				engine, err := container.NewEngine(config.ColimaSocketPath())
				if err != nil {
					return "", fmt.Errorf("cannot connect to Docker: %w", err)
				}
				defer engine.Close()
				if err := engine.Pull(cmd.Context(), opts.Image); err != nil {
					return "", fmt.Errorf("cannot pull %s: %w", opts.Image, err)
				}
				return fmt.Sprintf("Pulled %s", opts.Image), nil
			}); err != nil {
				if !errors.Is(err, ui.ErrAlreadyPrinted) {
					ui.Subtle(fmt.Sprintf("Image pull skipped: %v", err))
				}
			} else {
				// Create and start container.
				if err := ui.Step(fmt.Sprintf("Starting %s %s...", svc.DisplayName(), version), func() (string, error) {
					engine, err := container.NewEngine(config.ColimaSocketPath())
					if err != nil {
						return "", fmt.Errorf("cannot connect to Docker: %w", err)
					}
					defer engine.Close()
					if _, err := engine.CreateAndStart(cmd.Context(), opts); err != nil {
						return "", err
					}
					port := svc.Port(version)
					return fmt.Sprintf("%s %s running on :%d", svc.DisplayName(), version, port), nil
				}); err != nil {
					if !errors.Is(err, ui.ErrAlreadyPrinted) {
						ui.Subtle(fmt.Sprintf("Container start skipped: %v", err))
					}
				} else {
					containerReady = true
				}
			}
		}

		// Create data directory.
		dataDir := config.ServiceDataDir(svcName, version)
		if err := os.MkdirAll(dataDir, 0755); err != nil {
			return fmt.Errorf("cannot create data directory: %w", err)
		}

		// Update registry.
		instance := &registry.ServiceInstance{
			Image:       opts.Image,
			Port:        svc.Port(version),
			ConsolePort: svc.ConsolePort(version),
		}
		if err := reg.AddService(key, instance); err != nil {
			return err
		}
		if err := reg.Save(); err != nil {
			return fmt.Errorf("cannot save registry: %w", err)
		}

		// Update .env for linked Laravel projects.
		updateLinkedProjectsEnv(reg, svcName, svc, version)

		// Regenerate Caddy configs for service consoles (*.pv.{tld}).
		if err := caddy.GenerateServiceSiteConfigs(reg); err != nil {
			ui.Subtle(fmt.Sprintf("Could not generate service site config: %v", err))
		}

		// Print connection details.
		port := svc.Port(version)
		if containerReady {
			ui.Success(fmt.Sprintf("%s %s running on :%d", svc.DisplayName(), version, port))
		} else {
			ui.Success(fmt.Sprintf("%s %s registered on :%d", svc.DisplayName(), version, port))
		}
		fmt.Fprintln(os.Stderr)
		fmt.Fprintf(os.Stderr, "    %s  %s\n", ui.Muted.Render("Host"), "127.0.0.1")
		fmt.Fprintf(os.Stderr, "    %s  %d\n", ui.Muted.Render("Port"), port)

		envVars := svc.EnvVars("", port)
		if user, ok := envVars["DB_USERNAME"]; ok {
			fmt.Fprintf(os.Stderr, "    %s  %s\n", ui.Muted.Render("User"), user)
			pw := envVars["DB_PASSWORD"]
			if pw == "" {
				pw = "(none)"
			}
			fmt.Fprintf(os.Stderr, "    %s  %s\n", ui.Muted.Render("Pass"), pw)
		}
		if routes := svc.WebRoutes(); len(routes) > 0 {
			settings, _ := config.LoadSettings()
			if settings != nil {
				for _, route := range routes {
					fmt.Fprintf(os.Stderr, "    %s  https://%s.pv.%s\n", ui.Muted.Render(route.Subdomain), route.Subdomain, settings.Defaults.TLD)
				}
			}
		} else if consolePt := svc.ConsolePort(version); consolePt > 0 {
			fmt.Fprintf(os.Stderr, "    %s  :%d\n", ui.Muted.Render("Console"), consolePt)
		}
		fmt.Fprintln(os.Stderr)

		return nil
	},
}
