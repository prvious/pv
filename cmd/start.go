package cmd

import (
	"fmt"
	"time"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/daemon"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var (
	startBackground bool
	startForeground bool
)

var startCmd = &cobra.Command{
	Use:     "start",
	GroupID: "server",
	Short: "Start the pv server (DNS + FrankenPHP)",
	RunE: func(cmd *cobra.Command, args []string) error {
		if startBackground {
			return startDaemon()
		}
		return startFG()
	},
}

func startFG() error {
	if server.IsRunning() {
		return fmt.Errorf("pv is already running (PID file exists and process is alive)")
	}

	settings, err := config.LoadSettings()
	if err != nil {
		return fmt.Errorf("cannot load settings: %w", err)
	}

	return server.Start(settings.TLD)
}

func startDaemon() error {
	// Check if already running via launchd.
	if daemon.IsLoaded() {
		pid, err := daemon.GetPID()
		if err == nil && pid > 0 {
			ui.Success(fmt.Sprintf("pv is already running %s", ui.Muted.Render(fmt.Sprintf("(PID %d)", pid))))
			return nil
		}
	}

	// Also check foreground PID file.
	if server.IsRunning() {
		pid, _ := server.ReadPID()
		ui.Success(fmt.Sprintf("pv is already running in foreground %s", ui.Muted.Render(fmt.Sprintf("(PID %d)", pid))))
		return nil
	}

	if err := ui.Step("Starting pv daemon...", func() (string, error) {
		// Generate and write plist.
		cfg := daemon.DefaultPlistConfig()
		if err := daemon.Install(cfg); err != nil {
			return "", fmt.Errorf("cannot install plist: %w", err)
		}

		// Load the service.
		if err := daemon.Load(); err != nil {
			return "", fmt.Errorf("cannot start daemon: %w", err)
		}

		// Wait for the process to appear.
		var pid int
		for i := 0; i < 15; i++ {
			time.Sleep(200 * time.Millisecond)
			p, err := daemon.GetPID()
			if err == nil && p > 0 {
				pid = p
				break
			}
		}

		if pid > 0 {
			return fmt.Sprintf("Running in background (PID %d)", pid), nil
		}
		return "Daemon started (waiting for process...)", nil
	}); err != nil {
		return err
	}

	ui.Subtle("Run pv log to view logs")

	return nil
}

func init() {
	startCmd.Flags().BoolVar(&startBackground, "background", false, "Run as a background daemon via launchd")
	startCmd.Flags().BoolVar(&startForeground, "foreground", false, "Run in the foreground (default)")
	rootCmd.AddCommand(startCmd)
}
