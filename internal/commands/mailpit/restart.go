package mailpit

import (
	"time"

	pkg "github.com/prvious/pv/internal/mailpit"
	"github.com/spf13/cobra"
)

var restartCmd = &cobra.Command{
	Use:     "mailpit:restart",
	GroupID: "mailpit",
	Short:   "Stop then start Mailpit (toggles wanted state, daemon reconciles)",
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		if err := pkg.SetWanted(pkg.DefaultVersion(), pkg.WantedStopped); err != nil {
			return err
		}
		if err := pkg.WaitStopped(pkg.DefaultVersion(), 30*time.Second); err != nil {
			return err
		}
		return pkg.SetWanted(pkg.DefaultVersion(), pkg.WantedRunning)
	},
}
