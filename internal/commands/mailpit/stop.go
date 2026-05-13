package mailpit

import (
	pkg "github.com/prvious/pv/internal/mailpit"
	"github.com/spf13/cobra"
)

var stopCmd = &cobra.Command{
	Use:     "mailpit:stop",
	GroupID: "mailpit",
	Short:   "Mark Mailpit as wanted-stopped",
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		return pkg.SetWanted(pkg.DefaultVersion(), pkg.WantedStopped)
	},
}
