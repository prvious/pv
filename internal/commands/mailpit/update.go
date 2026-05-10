package mailpit

import (
	pkg "github.com/prvious/pv/internal/mailpit"
	"github.com/spf13/cobra"
)

var updateCmd = &cobra.Command{
	Use:     "mailpit:update",
	GroupID: "mailpit",
	Short:   "Re-download the Mailpit binary at the latest version",
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		return pkg.Update()
	},
}
