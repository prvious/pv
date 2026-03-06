package cmd

import (
	"fmt"
	"os"

	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var serviceLogsCmd = &cobra.Command{
	Use:   "service:logs <service>",
	Short: "Tail container logs for a service",
	Args:  cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		key := args[0]

		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}

		instance := reg.FindService(key)
		if instance == nil {
			fmt.Fprintln(os.Stderr)
			ui.Fail(fmt.Sprintf("Service %s not found", ui.Bold.Render(key)))
			fmt.Fprintln(os.Stderr)
			cmd.SilenceUsage = true
			return ui.ErrAlreadyPrinted
		}

		if instance.ContainerID == "" {
			fmt.Fprintln(os.Stderr)
			ui.Fail(fmt.Sprintf("Service %s is not running", ui.Bold.Render(key)))
			ui.FailDetail(fmt.Sprintf("Start it first: pv service:start %s", key))
			fmt.Fprintln(os.Stderr)
			cmd.SilenceUsage = true
			return ui.ErrAlreadyPrinted
		}

		// Docker SDK: ContainerLogs with Follow=true
		// This would stream logs to stdout.
		fmt.Fprintf(os.Stderr, "Tailing logs for %s (container: %s)...\n", key, instance.ContainerID)

		return nil
	},
}

func init() {
	rootCmd.AddCommand(serviceLogsCmd)
}
