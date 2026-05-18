package mailpit

import (
	"fmt"
	"time"

	pkg "github.com/prvious/pv/internal/mailpit"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var restartCmd = &cobra.Command{
	Use:     "mailpit:restart [version]",
	GroupID: "mailpit",
	Short:   "Stop then start Mailpit (toggles wanted state, daemon reconciles)",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		resolved, err := pkg.ResolveVersion(argVersion(args))
		if err != nil {
			return err
		}
		if !pkg.IsInstalled(resolved) {
			return fmt.Errorf("%s %s is not installed", pkg.Binary().Name, resolved)
		}
		if err := pkg.SetWanted(resolved, pkg.WantedStopped); err != nil {
			return err
		}
		if server.IsRunning() {
			if err := server.SignalDaemon(); err != nil {
				return fmt.Errorf("signal daemon: %w", err)
			}
			if err := pkg.WaitStopped(resolved, 30*time.Second); err != nil {
				return err
			}
		}
		if err := pkg.SetWanted(resolved, pkg.WantedRunning); err != nil {
			return err
		}
		ui.Success(fmt.Sprintf("%s %s restarted.", pkg.DisplayName(), resolved))
		return signalDaemon(pkg.DisplayName())
	},
}
