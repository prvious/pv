package mailpit

import (
	pkg "github.com/prvious/pv/internal/mailpit"
	"github.com/spf13/cobra"
)

var restartCmd = &cobra.Command{
	Use:     "mailpit:restart",
	GroupID: "mailpit",
	Short:   "Stop then start Mailpit (toggles wanted state, daemon reconciles)",
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		return pkg.Restart()
	},
}
