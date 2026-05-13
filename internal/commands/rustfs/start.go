package rustfs

import (
	pkg "github.com/prvious/pv/internal/rustfs"
	"github.com/spf13/cobra"
)

var startCmd = &cobra.Command{
	Use:     "rustfs:start",
	GroupID: "rustfs",
	Short:   "Mark RustFS as wanted-running",
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		return pkg.SetWanted(pkg.DefaultVersion(), pkg.WantedRunning)
	},
}
