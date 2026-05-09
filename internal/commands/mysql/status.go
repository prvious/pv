package mysql

import (
	"fmt"

	my "github.com/prvious/pv/internal/mysql"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var statusCmd = &cobra.Command{
	Use:     "mysql:status [version]",
	GroupID: "mysql",
	Short:   "Show MySQL version status",
	Long:    "Without [version], reports the status of every installed MySQL version. With [version], reports just that one (must be installed).",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		var versions []string
		if len(args) > 0 {
			v, err := ResolveVersion(args)
			if err != nil {
				return err
			}
			versions = []string{v}
		} else {
			vs, err := my.InstalledVersions()
			if err != nil {
				return err
			}
			versions = vs
		}
		if len(versions) == 0 {
			ui.Subtle("No MySQL versions installed.")
			return nil
		}

		status, _ := server.ReadDaemonStatus()
		for _, version := range versions {
			port, _ := my.PortFor(version)
			supKey := "mysql-" + version
			if status != nil {
				if s, ok := status.Supervised[supKey]; ok && s.Running {
					ui.Success(fmt.Sprintf("mysql %s: running on :%d (pid %d)", version, port, s.PID))
					continue
				}
			}
			ui.Subtle(fmt.Sprintf("mysql %s: stopped", version))
		}
		return nil
	},
}
