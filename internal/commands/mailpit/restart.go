package mailpit

import (
	"fmt"

	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/svchooks"
	"github.com/spf13/cobra"
)

var restartCmd = &cobra.Command{
	Use:     "mailpit:restart",
	GroupID: "mailpit",
	Short:   "Stop then start Mailpit (toggles wanted state, daemon reconciles)",
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
		if err := svchooks.SetEnabled(reg, svc, false); err != nil {
			return err
		}
		reg, err = registry.Load()
		if err != nil {
			return fmt.Errorf("cannot reload registry: %w", err)
		}
		return svchooks.SetEnabled(reg, svc, true)
	},
}
