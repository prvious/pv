package php

import (
	"fmt"
	"os"

	"github.com/prvious/pv/internal/daemon"
	"github.com/prvious/pv/internal/phpenv"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var useCmd = &cobra.Command{
	Use:     "php:use [version]",
	GroupID: "php",
	Short:   "Switch the global PHP version (e.g., pv php:use 8.4)",
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
				ui.Accent.Render("→"),
				ui.Positive.Bold(true).Render(version),
			))
		} else {
			ui.Success(fmt.Sprintf("Global PHP set to %s", ui.Positive.Bold(true).Render(version)))
		}

		// The global PHP binary changed — daemon needs full restart.
		if oldV != version && server.IsRunning() {
			if daemon.IsLoaded() {
				if err := daemon.Restart(); err != nil {
					ui.Fail(fmt.Sprintf("Could not restart daemon: %v — run 'pv restart' manually", err))
				} else {
					ui.Success("Daemon restarted with new PHP version")
				}
			} else {
				ui.Subtle("Server is running in foreground — restart required.")
				ui.Subtle("Run: pv stop && pv start")
			}
		}

		return nil
	},
}
