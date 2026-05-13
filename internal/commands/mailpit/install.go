package mailpit

import (
	"net/http"
	"time"

	pkg "github.com/prvious/pv/internal/mailpit"
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
		client := &http.Client{Timeout: 60 * time.Second}
		return pkg.Install(client, pkg.DefaultVersion())
	},
}
