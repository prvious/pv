package mailpit

import (
	"fmt"

	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/svchooks"
	"github.com/spf13/cobra"
)

var updateCmd = &cobra.Command{
	Use:     "mailpit:update",
	GroupID: "mailpit",
	Short:   "Re-download the Mailpit binary at the latest version",
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}
		svc, ok := services.LookupBinary("mail")
		if !ok {
			return fmt.Errorf("mailpit binary service not registered (build issue)")
		}
		return svchooks.Update(reg, svc)
	},
}
