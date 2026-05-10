package rustfs

import (
	pkg "github.com/prvious/pv/internal/rustfs"
	"github.com/spf13/cobra"
)

var restartCmd = &cobra.Command{
	Use:     "rustfs:restart",
	GroupID: "rustfs",
	Short:   "Stop then start RustFS (toggles wanted state, daemon reconciles)",
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		return pkg.Restart()
	},
}
