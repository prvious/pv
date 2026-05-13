package mailpit

import (
	pkg "github.com/prvious/pv/internal/mailpit"
	"github.com/spf13/cobra"
)

var startCmd = &cobra.Command{
	Use:     "mailpit:start",
	GroupID: "mailpit",
	Short:   "Mark Mailpit as wanted-running",
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		return pkg.SetWanted(pkg.DefaultVersion(), pkg.WantedRunning)
	},
}
