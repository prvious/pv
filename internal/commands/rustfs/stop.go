package rustfs

import (
	pkg "github.com/prvious/pv/internal/rustfs"
	"github.com/spf13/cobra"
)

var stopCmd = &cobra.Command{
	Use:     "rustfs:stop",
	GroupID: "rustfs",
	Short:   "Mark RustFS as wanted-stopped",
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		return pkg.SetWanted(pkg.DefaultVersion(), pkg.WantedStopped)
	},
}
