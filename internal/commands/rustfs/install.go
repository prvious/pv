package rustfs

import (
	"net/http"
	"time"

	pkg "github.com/prvious/pv/internal/rustfs"
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
		client := &http.Client{Timeout: 60 * time.Second}
		return pkg.Install(client, pkg.DefaultVersion())
	},
}
