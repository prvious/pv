package cmd

import (
	"fmt"
	"os"
	"syscall"

	"github.com/prvious/pv/internal/server"
	"github.com/spf13/cobra"
)

var stopCmd = &cobra.Command{
	Use:   "stop",
	Short: "Stop the pv server",
	RunE: func(cmd *cobra.Command, args []string) error {
		pid, err := server.ReadPID()
		if err != nil {
			return fmt.Errorf("pv does not appear to be running (no PID file)")
		}

		proc, err := os.FindProcess(pid)
		if err != nil {
			return fmt.Errorf("cannot find process %d: %w", pid, err)
		}

		if err := proc.Signal(syscall.SIGTERM); err != nil {
			return fmt.Errorf("cannot send signal to process %d: %w", pid, err)
		}

		fmt.Printf("Sent stop signal to pv (PID %d)\n", pid)
		return nil
	},
}

func init() {
	rootCmd.AddCommand(stopCmd)
}
