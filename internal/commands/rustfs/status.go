package rustfs

import (
	"fmt"

	pkg "github.com/prvious/pv/internal/rustfs"
	"github.com/spf13/cobra"
)

var statusCmd = &cobra.Command{
	Use:     "rustfs:status",
	GroupID: "rustfs",
	Short:   "Show RustFS supervised state",
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		if !pkg.IsInstalled(pkg.DefaultVersion()) {
			return fmt.Errorf("RustFS is not installed")
		}
		return nil
	},
}
