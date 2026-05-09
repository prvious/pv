package mysql

import (
	"fmt"

	my "github.com/prvious/pv/internal/mysql"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var startCmd = &cobra.Command{
	Use:     "mysql:start [version]",
	GroupID: "mysql",
	Short:   "Mark a MySQL version as wanted-running",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version, err := ResolveVersion(args)
		if err != nil {
			return err
		}
		if err := my.SetWanted(version, my.WantedRunning); err != nil {
			return err
		}
		ui.Success(fmt.Sprintf("MySQL %s marked running.", version))
		if server.IsRunning() {
			return server.SignalDaemon()
		}
		ui.Subtle("daemon not running — will start on next `pv start`")
		return nil
	},
}
