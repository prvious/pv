package mysql

import (
	"fmt"

	my "github.com/prvious/pv/internal/mysql"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var installCmd = &cobra.Command{
	Use:     "mysql:install [version]",
	GroupID: "mysql",
	Short:   "Install (or re-install) a MySQL version",
	Long:    "Downloads MySQL binaries, runs --initialize-insecure on first install, and registers the version as wanted-running. Default version: 8.4.",
	Example: `# Install MySQL 8.4 (default)
pv mysql:install

# Install MySQL 9.7 alongside 8.4
pv mysql:install 9.7`,
	Args: cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		arg := ""
		if len(args) > 0 {
			arg = args[0]
		}
		version, err := my.ResolveVersion(arg)
		if err != nil {
			return err
		}

		// Already installed → idempotent: re-mark wanted=running and
		// signal the daemon. Same friendly contract postgres uses.
		if my.IsInstalled(version) {
			if err := my.SetWanted(version, my.WantedRunning); err != nil {
				return err
			}
			ui.Success(fmt.Sprintf("MySQL %s already installed — marked as wanted running.", version))
			return signalDaemon()
		}

		// Run the download/extract/initdb pipeline.
		if err := downloadCmd.RunE(downloadCmd, []string{version}); err != nil {
			return err
		}
		ui.Success(fmt.Sprintf("MySQL %s installed.", version))
		return signalDaemon()
	},
}

// signalDaemon nudges the running pv daemon to reconcile, or no-ops with
// a friendly note if the daemon isn't up.
func signalDaemon() error {
	if !server.IsRunning() {
		ui.Subtle("daemon not running — mysql will start on next `pv start`")
		return nil
	}
	return server.SignalDaemon()
}
