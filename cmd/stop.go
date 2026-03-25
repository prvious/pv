package cmd

import (
	"fmt"
	"os"
	"syscall"
	"time"

	"github.com/prvious/pv/internal/daemon"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var stopCmd = &cobra.Command{
	Use:     "stop",
	GroupID: "server",
	Short:   "Stop the pv server",
	RunE: func(cmd *cobra.Command, args []string) error {
		// Check daemon mode first.
		if daemon.IsLoaded() {
			if err := ui.Step("Stopping pv daemon...", func() (string, error) {
				if err := daemon.Unload(); err != nil {
					return "", fmt.Errorf("cannot stop daemon: %w", err)
				}

				// Wait for process to exit.
				for i := 0; i < 25; i++ {
					time.Sleep(200 * time.Millisecond)
					if !daemon.IsLoaded() {
						break
					}
				}

				return "pv stopped", nil
			}); err != nil {
				return err
			}
			return nil
		}

		// Foreground mode — use PID file.
		pid, err := server.ReadPID()
		if err != nil {
			ui.Subtle("pv is not running")
			return nil
		}

		if err := ui.Step("Stopping pv server...", func() (string, error) {
			proc, err := os.FindProcess(pid)
			if err != nil {
				return "", fmt.Errorf("cannot find process %d: %w", pid, err)
			}

			if err := proc.Signal(syscall.SIGTERM); err != nil {
				return "", fmt.Errorf("cannot send signal to process %d: %w", pid, err)
			}

			// Wait for process to exit (5s).
			exited := false
			for i := 0; i < 25; i++ {
				time.Sleep(200 * time.Millisecond)
				if proc.Signal(syscall.Signal(0)) != nil {
					exited = true
					break
				}
			}

			if !exited {
				_ = proc.Signal(syscall.SIGKILL)
				// Brief wait for SIGKILL to take effect.
				time.Sleep(500 * time.Millisecond)
			}

			return "pv stopped", nil
		}); err != nil {
			return err
		}

		return nil
	},
}

func init() {
	rootCmd.AddCommand(stopCmd)
}
