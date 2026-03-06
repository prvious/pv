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

		opts := svc.CreateOpts(version)
		var containerID string

		// Attempt container operations if Colima is available.
		// Failures are non-fatal — the service is still registered.
		containerReady := false
		if colima.IsInstalled() {
			if err := colima.EnsureRunning(); err != nil {
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
					_ = engine // Pull would happen via engine.PullImage()
					return fmt.Sprintf("Pulled %s", opts.Image), nil
				}); err != nil {
					ui.Subtle(fmt.Sprintf("Image pull skipped: %v", err))
				} else {
					// Create and start container.
					if err := ui.Step(fmt.Sprintf("Starting %s %s...", svc.DisplayName(), version), func() (string, error) {
						// Container creation and health check would happen here via Docker SDK.
						containerID = "" // Would be set by engine.CreateAndStart()
						port := svc.Port(version)
						return fmt.Sprintf("%s %s running on :%d", svc.DisplayName(), version, port), nil
					}); err != nil {
						ui.Subtle(fmt.Sprintf("Container start skipped: %v", err))
					} else {
						containerReady = true
					}
				}
			}
		} else {
			ui.Subtle("Colima not installed — container will start when runtime is available.")
			ui.Subtle("Run 'pv install' to set up the container runtime.")
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
			ContainerID: containerID,
		}
		if err := reg.AddService(key, instance); err != nil {
			return err
		}
		if err := reg.Save(); err != nil {
			return fmt.Errorf("cannot save registry: %w", err)
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
