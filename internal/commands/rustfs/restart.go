package rustfs

import (
	"fmt"

	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/svchooks"
	"github.com/spf13/cobra"
)

var restartCmd = &cobra.Command{
	Use:     "rustfs:restart",
	GroupID: "rustfs",
	Short:   "Stop then start RustFS (toggles wanted state, daemon reconciles)",
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
		return svchooks.Restart(reg, svc)
	},
}
