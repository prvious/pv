package service

import (
	"context"
	"fmt"
	"net/http"
	"os"
	"time"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/caddy"
	"github.com/prvious/pv/internal/colima"
	colimacmds "github.com/prvious/pv/internal/commands/colima"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/container"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
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

# Add S3 (RustFS binary)
pv service:add s3

# Add a specific Redis version
pv service:add redis 7`,
	Args: cobra.RangeArgs(1, 2),
	RunE: func(cmd *cobra.Command, args []string) error {
		name := args[0]

		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}

		kind, binSvc, dockerSvc, err := resolveKind(reg, name)
		if err != nil {
			return err
		}

		switch kind {
		case kindBinary:
			return addBinary(cmd.Context(), reg, binSvc)
		case kindDocker:
			version := dockerSvc.DefaultVersion()
			if len(args) > 1 {
				version = args[1]
			}
			return addDocker(cmd, reg, dockerSvc, name, version)
		}
		return fmt.Errorf("unknown service %q", name)
	},
}

func addDocker(cmd *cobra.Command, reg *registry.Registry, svc services.Service, svcName, version string) error {
	key := services.ServiceKey(svcName, version)

	existing, findErr := reg.FindService(key)
	if findErr != nil {
		return findErr
	}
	if existing != nil {
		fmt.Fprintln(os.Stderr)
		ui.Success(fmt.Sprintf("%s is already added", ui.Accent.Bold(true).Render(svc.DisplayName()+" "+version)))
		fmt.Fprintln(os.Stderr)
		return nil
	}

	fmt.Fprintln(os.Stderr)

	opts := svc.CreateOpts(version)

	// Create data directory before container creation (bind mounts require it to exist).
	dataDir := config.ServiceDataDir(svcName, version)
	if err := os.MkdirAll(dataDir, 0755); err != nil {
		return fmt.Errorf("cannot create data directory: %w", err)
	}

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
		engine, engineErr := container.NewEngine(config.ColimaSocketPath())
		if engineErr != nil {
			return fmt.Errorf("cannot connect to Docker: %w", engineErr)
		}
		defer engine.Close()

		// Pull image.
		if err := ui.Step(fmt.Sprintf("Pulling %s...", opts.Image), func() (string, error) {
			if err := engine.Pull(cmd.Context(), opts.Image); err != nil {
				return "", fmt.Errorf("cannot pull %s: %w", opts.Image, err)
			}
			return fmt.Sprintf("Pulled %s", opts.Image), nil
		}); err != nil {
			return err
		}

		// Create and start container.
		if err := ui.Step(fmt.Sprintf("Starting %s %s...", svc.DisplayName(), version), func() (string, error) {
			if _, err := engine.CreateAndStart(cmd.Context(), opts); err != nil {
				return "", err
			}
			port := svc.Port(version)
			return fmt.Sprintf("%s %s running on :%d", svc.DisplayName(), version, port), nil
		}); err != nil {
			return err
		}
		containerReady = true
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
	if server.IsRunning() {
		if err := server.SignalDaemon(); err != nil {
			ui.Subtle(fmt.Sprintf("Could not signal daemon: %v", err))
		}
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
}

// addBinary downloads the service's binary (if not yet present), persists its
// version, registers the service in the registry, then signals the daemon.
func addBinary(ctx context.Context, reg *registry.Registry, svc services.BinaryService) error {
	name := svc.Name()
	if _, exists := reg.Services[name]; exists {
		ui.Success(fmt.Sprintf("%s is already added", svc.DisplayName()))
		return nil
	}

	client := &http.Client{Timeout: 60 * time.Second}

	// Resolve latest upstream version.
	latest, err := binaries.FetchLatestVersion(client, svc.Binary())
	if err != nil {
		return fmt.Errorf("cannot resolve latest %s version: %w", svc.Binary().DisplayName, err)
	}

	// Download + extract into ~/.pv/internal/bin/<name>.
	if err := ui.Step(fmt.Sprintf("Downloading %s %s...", svc.Binary().DisplayName, latest), func() (string, error) {
		if err := binaries.InstallBinary(client, svc.Binary(), latest); err != nil {
			return "", err
		}
		return fmt.Sprintf("Installed %s %s", svc.Binary().DisplayName, latest), nil
	}); err != nil {
		return err
	}

	// Record version for later pv-update comparisons.
	vs, err := binaries.LoadVersions()
	if err != nil {
		return fmt.Errorf("cannot load versions state: %w", err)
	}
	vs.Set(svc.Binary().Name, latest)
	if err := vs.Save(); err != nil {
		return fmt.Errorf("cannot save versions state: %w", err)
	}

	// Register service.
	enabled := true
	inst := &registry.ServiceInstance{
		Port:        svc.Port(),
		ConsolePort: svc.ConsolePort(),
		Kind:        "binary",
		Enabled:     &enabled,
	}
	if err := reg.AddService(name, inst); err != nil {
		return err
	}
	if err := reg.Save(); err != nil {
		return fmt.Errorf("cannot save registry: %w", err)
	}

	// Projects linked before this service was added won't be in ProjectsUsingService;
	// bind them now so updateLinkedProjectsEnvBinary can find them.
	if err := bindBinaryServiceToAllProjects(reg, name); err != nil {
		return fmt.Errorf("cannot bind service to projects: %w", err)
	}
	if err := reg.Save(); err != nil {
		return fmt.Errorf("cannot save registry after binding service: %w", err)
	}

	// Update .env for linked Laravel projects — parity with the docker path
	// (updateLinkedProjectsEnv at the end of addDocker). Without this the
	// user adds s3 but linked projects never get AWS_* keys written.
	updateLinkedProjectsEnvBinary(reg, name, svc)

	// Regenerate Caddy configs for service consoles (*.pv.{tld}).
	if err := caddy.GenerateServiceSiteConfigs(reg); err != nil {
		ui.Subtle(fmt.Sprintf("Could not generate service site config: %v", err))
	}

	// Signal daemon to reconcile.
	if server.IsRunning() {
		if err := server.SignalDaemon(); err != nil {
			ui.Subtle(fmt.Sprintf("Could not signal daemon: %v", err))
		}
		ui.Success(fmt.Sprintf("%s registered and running on :%d", svc.DisplayName(), svc.Port()))
	} else {
		ui.Success(fmt.Sprintf("%s registered on :%d", svc.DisplayName(), svc.Port()))
		ui.Subtle("daemon not running — service will start on next `pv start`")
	}

	printBinaryConnectionDetails(svc)
	return nil
}

// printBinaryConnectionDetails mirrors the verbose "Host / Port / web routes"
// footer that the docker path prints, scoped to the binary-service shape.
func printBinaryConnectionDetails(svc services.BinaryService) {
	fmt.Fprintln(os.Stderr)
	fmt.Fprintf(os.Stderr, "    %s  127.0.0.1\n", ui.Muted.Render("Host"))
	fmt.Fprintf(os.Stderr, "    %s  %d\n", ui.Muted.Render("Port"), svc.Port())
	settings, _ := config.LoadSettings()
	if settings != nil {
		for _, route := range svc.WebRoutes() {
			fmt.Fprintf(os.Stderr, "    %s  https://%s.pv.%s\n",
				ui.Muted.Render(route.Subdomain), route.Subdomain, settings.Defaults.TLD)
		}
	}
	fmt.Fprintln(os.Stderr)
}
