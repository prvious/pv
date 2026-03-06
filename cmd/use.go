package cmd

import (
	"fmt"
	"os"
	"strings"

	"github.com/prvious/pv/internal/daemon"
	"github.com/prvious/pv/internal/phpenv"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var useCmd = &cobra.Command{
	Use:   "use <php:version>",
	Short: "Switch the global PHP version (e.g., pv use php:8.4)",
	Args:  cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		arg := args[0]
		if !strings.HasPrefix(arg, "php:") {
			fmt.Fprintln(os.Stderr)
			ui.Fail(fmt.Sprintf("Invalid format %s", ui.Bold.Render(arg)))
			ui.FailDetail("Use php:<version> (e.g., pv use php:8.4)")
			fmt.Fprintln(os.Stderr)
			cmd.SilenceUsage = true
			return ui.ErrAlreadyPrinted
		}

		version := strings.TrimPrefix(arg, "php:")
		if version == "" {
			return fmt.Errorf("version cannot be empty")
		}

		if !phpenv.IsInstalled(version) {
			fmt.Fprintln(os.Stderr)
			ui.Fail(fmt.Sprintf("PHP %s is not installed", ui.Bold.Render(version)))
			ui.FailDetail(fmt.Sprintf("Run: pv php install %s", version))
			fmt.Fprintln(os.Stderr)
			cmd.SilenceUsage = true
			return ui.ErrAlreadyPrinted
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
				fmt.Fprintf(os.Stderr, "  %s %s\n",
					ui.Red.Render("!"),
					ui.Muted.Render(fmt.Sprintf("Cannot sync daemon plist: %v", err)),
				)
			} else {
				ui.Success("Daemon restarted with new PHP version")
			}
		} else if oldV != version && server.IsRunning() {
			ui.Subtle("Server is running — restart required for changes to take effect.")
			ui.Subtle("Run: pv restart")
		}

		fmt.Fprintln(os.Stderr)
		return nil
	},
}

func init() {
	rootCmd.AddCommand(useCmd)
}
