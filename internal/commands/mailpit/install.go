package mailpit

import (
	"fmt"

	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/svchooks"
	"github.com/spf13/cobra"
)

var installCmd = &cobra.Command{
	Use:     "mailpit:install",
	GroupID: "mailpit",
	Short:   "Install Mailpit (SMTP catcher + web UI) and start it",
	Long:    "Downloads the Mailpit binary, registers it as a supervised service, and signals the daemon to start it.",
	Example: `pv mailpit:install
pv mail:install`,
	Args: cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}
		svc, ok := services.LookupBinary("mail")
		if !ok {
			return fmt.Errorf("mailpit binary service not registered (build issue)")
		}
		if existing, ok := reg.Services["mail"]; ok && existing.Kind != "binary" {
			return fmt.Errorf(
				"mail is already registered (as docker) from a previous pv version. " +
					"Run `pv uninstall && pv setup` to reset")
		}
		return svchooks.Install(reg, svc)
	},
}
