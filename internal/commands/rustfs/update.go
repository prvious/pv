package rustfs

import (
	"fmt"

	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/svchooks"
	"github.com/spf13/cobra"
)

var updateCmd = &cobra.Command{
	Use:     "rustfs:update",
	GroupID: "rustfs",
	Short:   "Re-download the RustFS binary at the latest version",
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}
		svc, ok := services.LookupBinary("s3")
		if !ok {
			return fmt.Errorf("rustfs binary service not registered (build issue)")
		}
		return svchooks.Update(reg, svc)
	},
}
