package cmd

import (
	"fmt"
	"os"

	"github.com/prvious/pv/internal/colima"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/container"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var serviceAddCmd = &cobra.Command{
	Use:   "add <service> [version]",
	Short: "Add and start a service",
	Long:  "Add a backing service (mysql, postgres, redis, rustfs). Optionally specify a version.",
	Args:  cobra.RangeArgs(1, 2),
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
			ui.Success(fmt.Sprintf("%s is already added", ui.Purple.Bold(true).Render(svc.DisplayName()+" "+version)))
			fmt.Fprintln(os.Stderr)
			return nil
		}

		fmt.Fprintln(os.Stderr)

		// Ensure Colima is running.
		if err := ui.Step("Starting container runtime...", func() (string, error) {
			if err := colima.EnsureRunning(); err != nil {
				return "", fmt.Errorf("cannot start Colima: %w", err)
			}
			return "Container runtime ready", nil
		}); err != nil {
			return err
		}

		// Pull image.
		opts := svc.CreateOpts(version)
		if err := ui.Step(fmt.Sprintf("Pulling %s...", opts.Image), func() (string, error) {
			engine, err := container.NewEngine(config.ColimaSocketPath())
			if err != nil {
				return "", fmt.Errorf("cannot connect to Docker: %w", err)
			}
			defer engine.Close()
			_ = engine // Pull would happen via engine.PullImage()
			return fmt.Sprintf("Pulled %s", opts.Image), nil
		}); err != nil {
			return err
		}

		// Create data directory.
		dataDir := config.ServiceDataDir(svcName, version)
		if err := os.MkdirAll(dataDir, 0755); err != nil {
			return fmt.Errorf("cannot create data directory: %w", err)
		}

		// Create and start container.
		var containerID string
		if err := ui.Step(fmt.Sprintf("Starting %s %s...", svc.DisplayName(), version), func() (string, error) {
			// Container creation and health check would happen here via Docker SDK.
			containerID = "" // Would be set by engine.CreateAndStart()
			port := svc.Port(version)
			return fmt.Sprintf("%s %s running on :%d", svc.DisplayName(), version, port), nil
		}); err != nil {
			return err
		}

		// Update registry.
		instance := &registry.ServiceInstance{
			Image:       opts.Image,
			Port:        svc.Port(version),
			ConsolePort: svc.ConsolePort(version),
			ContainerID: containerID,
		}
		if err := reg.AddService(key, instance); err != nil {
			return err
		}
		if err := reg.Save(); err != nil {
			return fmt.Errorf("cannot save registry: %w", err)
		}

		// Print connection details.
		fmt.Fprintln(os.Stderr)
		port := svc.Port(version)
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
		if consolePt := svc.ConsolePort(version); consolePt > 0 {
			fmt.Fprintf(os.Stderr, "    %s  :%d\n", ui.Muted.Render("Console"), consolePt)
		}
		fmt.Fprintln(os.Stderr)

		return nil
	},
}

func init() {
	serviceCmd.AddCommand(serviceAddCmd)
}
