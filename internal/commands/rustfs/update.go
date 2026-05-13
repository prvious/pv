package rustfs

import (
	"net/http"
	"time"

	pkg "github.com/prvious/pv/internal/rustfs"
	"github.com/spf13/cobra"
)

var updateCmd = &cobra.Command{
	Use:     "rustfs:update",
	GroupID: "rustfs",
	Short:   "Re-download the RustFS binary at the latest version",
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		client := &http.Client{Timeout: 60 * time.Second}
		return pkg.Update(client, pkg.DefaultVersion())
	},
}
