package mysql

import (
	"fmt"

	my "github.com/prvious/pv/internal/mysql"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var stopCmd = &cobra.Command{
	Use:     "mysql:stop [version]",
	GroupID: "mysql",
	Short:   "Mark a MySQL version as wanted-stopped",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version, err := ResolveVersion(args)
		if err != nil {
			return err
		}
		if err := my.SetWanted(version, my.WantedStopped); err != nil {
			return err
		}
		ui.Success(fmt.Sprintf("MySQL %s marked stopped.", version))
		if server.IsRunning() {
			return server.SignalDaemon()
		}
		return nil
	},
}
