package rustfs

import (
	"fmt"

	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/svchooks"
	"github.com/spf13/cobra"
)

var installCmd = &cobra.Command{
	Use:     "rustfs:install",
	GroupID: "rustfs",
	Short:   "Install RustFS (S3-compatible storage) and start it",
	Long:    "Downloads the RustFS binary, registers it as a supervised service, and signals the daemon to start it.",
	Example: `pv rustfs:install
pv s3:install`,
	Args: cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}
		svc, ok := services.LookupBinary("s3")
		if !ok {
			return fmt.Errorf("rustfs binary service not registered (build issue)")
		}
		return svchooks.Install(reg, svc)
	},
}
