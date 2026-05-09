package mailpit

import (
	"fmt"

	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/svchooks"
	"github.com/spf13/cobra"
)

var statusCmd = &cobra.Command{
	Use:     "mailpit:status",
	GroupID: "mailpit",
	Short:   "Show Mailpit supervised state",
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
		svchooks.PrintStatus(reg, svc)
		return nil
	},
}
