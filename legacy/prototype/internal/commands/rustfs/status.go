package rustfs

import (
	"fmt"

	pkg "github.com/prvious/pv/internal/rustfs"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var statusCmd = &cobra.Command{
	Use:     "rustfs:status [version]",
	GroupID: "rustfs",
	Short:   "Show RustFS supervised state",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		resolved, err := pkg.ResolveVersion(argVersion(args))
		if err != nil {
			return err
		}
		if !pkg.IsInstalled(resolved) {
			ui.Subtle(fmt.Sprintf("%s %s is not installed.", pkg.DisplayName(), resolved))
			return nil
		}
		status, _ := server.ReadDaemonStatus()
		key := pkg.Binary().Name + "-" + resolved
		if status != nil {
			if s, ok := status.Supervised[key]; ok && s.Running {
				ui.Success(fmt.Sprintf("%s: running (pid %d)", key, s.PID))
				return nil
			}
		}
		ui.Subtle(fmt.Sprintf("%s: stopped", key))
		return nil
	},
}
