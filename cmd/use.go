package cmd

import (
	"fmt"
	"strings"

	"github.com/prvious/pv/internal/daemon"
	"github.com/prvious/pv/internal/phpenv"
	"github.com/prvious/pv/internal/server"
	"github.com/spf13/cobra"
)

var useCmd = &cobra.Command{
	Use:   "use <php:version>",
	Short: "Switch the global PHP version (e.g., pv use php:8.4)",
	Args:  cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		arg := args[0]
		if !strings.HasPrefix(arg, "php:") {
			return fmt.Errorf("invalid format %q: use php:<version> (e.g., pv use php:8.4)", arg)
		}

		version := strings.TrimPrefix(arg, "php:")
		if version == "" {
			return fmt.Errorf("version cannot be empty")
		}

		if !phpenv.IsInstalled(version) {
			return fmt.Errorf("PHP %s is not installed (run: pv php install %s)", version, version)
		}

		oldV, _ := phpenv.GlobalVersion()

		if err := phpenv.SetGlobal(version); err != nil {
			return err
		}

		fmt.Printf("Global PHP switched to %s\n", version)

		// If daemon is running, sync the plist and restart.
		if oldV != version && daemon.IsLoaded() {
			cfg := daemon.DefaultPlistConfig()
			if err := daemon.SyncIfNeeded(cfg); err != nil {
				fmt.Printf("Warning: cannot sync daemon plist: %v\n", err)
			} else {
				fmt.Println("Daemon restarted with new PHP version")
			}
		} else if oldV != version && server.IsRunning() {
			fmt.Println("Server is running — restart required for changes to take effect.")
			fmt.Println("Run: pv restart")
		}

		return nil
	},
}

func init() {
	rootCmd.AddCommand(useCmd)
}
