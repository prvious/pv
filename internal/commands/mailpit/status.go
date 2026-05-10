package mailpit

import (
	pkg "github.com/prvious/pv/internal/mailpit"
	"github.com/spf13/cobra"
)

var statusCmd = &cobra.Command{
	Use:     "mailpit:status",
	GroupID: "mailpit",
	Short:   "Show Mailpit supervised state",
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		return pkg.PrintStatus()
	},
}
