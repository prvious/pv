package mysql

import (
	"fmt"
	"time"

	my "github.com/prvious/pv/internal/mysql"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var restartCmd = &cobra.Command{
	Use:     "mysql:restart [version]",
	GroupID: "mysql",
	Short:   "Stop and start a MySQL version",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version, err := ResolveVersion(args)
		if err != nil {
			return err
		}
		// Phase 1: ask for stop, signal, wait for actual shutdown. Skipping
		// WaitStopped here would race with the supervisor's restart of the
		// next phase — reconciler could observe wanted=stopped, kill the
		// process, then observe wanted=running and start a fresh one before
		// the old one has fully released the port.
		if err := my.SetWanted(version, my.WantedStopped); err != nil {
			return err
		}
		if server.IsRunning() {
			if err := server.SignalDaemon(); err != nil {
				return fmt.Errorf("signal daemon: %w", err)
			}
			if err := my.WaitStopped(version, 30*time.Second); err != nil {
				return fmt.Errorf("waiting for mysql %s to stop: %w", version, err)
			}
		}
		// Phase 2: ask for running, signal once. The supervisor pass the
		// daemon does after this signal sees the wanted-set jump back to
		// running and brings the process up.
		if err := my.SetWanted(version, my.WantedRunning); err != nil {
			return err
		}
		if server.IsRunning() {
			if err := server.SignalDaemon(); err != nil {
				return err
			}
		}
		ui.Success(fmt.Sprintf("MySQL %s restarted.", version))
		return nil
	},
}
