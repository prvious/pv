package cmd

import (
	"fmt"
	"os"

	"github.com/prvious/pv/internal/daemon"
	"github.com/prvious/pv/internal/phpenv"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var phpUseCmd = &cobra.Command{
	Use:     "php:use <version>",
	GroupID: "php",
	Short: "Switch the global PHP version (e.g., pv php:use 8.4)",
	Example: `pv php:use 8.4
pv php:use 8.3`,
	Args: cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version := args[0]
		if version == "" {
			return fmt.Errorf("version cannot be empty")
		}

		if !phpenv.IsInstalled(version) {
			return fmt.Errorf("PHP %s is not installed, run: pv php:install %s", version, version)
		}

		oldV, _ := phpenv.GlobalVersion()

		if err := phpenv.SetGlobal(version); err != nil {
			return err
		}

		fmt.Fprintln(os.Stderr)

		if oldV != "" && oldV != version {
			ui.Success(fmt.Sprintf("Global PHP switched %s %s %s",
				ui.Muted.Render(oldV),
				ui.Purple.Render("→"),
				ui.Green.Bold(true).Render(version),
			))
		} else {
			ui.Success(fmt.Sprintf("Global PHP set to %s", ui.Green.Bold(true).Render(version)))
		}

		// If daemon is running, sync the plist and restart.
		if oldV != version && daemon.IsLoaded() {
			cfg := daemon.DefaultPlistConfig()
			if err := daemon.SyncIfNeeded(cfg); err != nil {
				ui.Fail(fmt.Sprintf("Cannot sync daemon plist: %v", err))
			} else {
				ui.Success("Daemon restarted with new PHP version")
			}
		} else if oldV != version && server.IsRunning() {
			ui.Subtle("Server is running — restart required for changes to take effect.")
			ui.Subtle("Run: pv restart")
		}

		return nil
	},
}

func init() {
	rootCmd.AddCommand(phpUseCmd)
}
