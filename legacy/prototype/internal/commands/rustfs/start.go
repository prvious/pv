package rustfs

import (
	"fmt"

	pkg "github.com/prvious/pv/internal/rustfs"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var startCmd = &cobra.Command{
	Use:     "rustfs:start [version]",
	GroupID: "rustfs",
	Short:   "Mark RustFS as wanted-running",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		resolved, err := pkg.ResolveVersion(argVersion(args))
		if err != nil {
			return err
		}
		if !pkg.IsInstalled(resolved) {
			ui.Subtle(fmt.Sprintf("%s %s is not installed (run `pv %s:install %s`).", pkg.DisplayName(), resolved, pkg.Binary().Name, resolved))
			return nil
		}
		if err := pkg.SetWanted(resolved, pkg.WantedRunning); err != nil {
			return err
		}
		ui.Success(fmt.Sprintf("%s %s marked running.", pkg.DisplayName(), resolved))
		return signalDaemon(pkg.DisplayName())
	},
}
