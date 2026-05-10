package rustfs

import (
	pkg "github.com/prvious/pv/internal/rustfs"
	"github.com/spf13/cobra"
)

var statusCmd = &cobra.Command{
	Use:     "rustfs:status",
	GroupID: "rustfs",
	Short:   "Show RustFS supervised state",
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		return pkg.PrintStatus()
	},
}
