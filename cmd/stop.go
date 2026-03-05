package cmd

import (
	"fmt"
	"os"
	"syscall"
	"time"

	"github.com/prvious/pv/internal/daemon"
	"github.com/prvious/pv/internal/server"
	"github.com/spf13/cobra"
)

var stopCmd = &cobra.Command{
	Use:   "stop",
	Short: "Stop the pv server",
	RunE: func(cmd *cobra.Command, args []string) error {
		// Check daemon mode first.
		if daemon.IsLoaded() {
			if err := daemon.Unload(); err != nil {
				return fmt.Errorf("cannot stop daemon: %w", err)
			}

			// Wait for process to exit.
			for i := 0; i < 25; i++ {
				time.Sleep(200 * time.Millisecond)
				if !daemon.IsLoaded() {
					break
				}
			}

			fmt.Println("pv stopped")
			return nil
		}

		// Foreground mode — use PID file.
		pid, err := server.ReadPID()
		if err != nil {
			fmt.Println("pv is not running")
			return nil
		}

		proc, err := os.FindProcess(pid)
		if err != nil {
			return fmt.Errorf("cannot find process %d: %w", pid, err)
		}

		if err := proc.Signal(syscall.SIGTERM); err != nil {
			return fmt.Errorf("cannot send signal to process %d: %w", pid, err)
		}

		// Wait for process to exit.
		for i := 0; i < 25; i++ {
			time.Sleep(200 * time.Millisecond)
			if proc.Signal(syscall.Signal(0)) != nil {
				break
			}
		}

		fmt.Println("pv stopped")
		return nil
	},
}

func init() {
	rootCmd.AddCommand(stopCmd)
}
