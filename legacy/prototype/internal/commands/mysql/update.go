package mysql

import (
	"fmt"
	"net/http"
	"time"

	my "github.com/prvious/pv/internal/mysql"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var updateCmd = &cobra.Command{
	Use:     "mysql:update <version>",
	GroupID: "mysql",
	Short:   "Re-download a MySQL version (data dir untouched)",
	Example: `pv mysql:update 8.4`,
	Args:    cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version := args[0]
		if !my.IsInstalled(version) {
			return fmt.Errorf("mysql %s is not installed", version)
		}

		// Capture whether the version was running before we update so we can
		// restore that state at the end. We always stop for the swap to avoid
		// replacing a binary mid-execution.
		wasRunning := false
		if st, err := my.LoadState(); err == nil {
			if entry, ok := st.Versions[version]; ok && entry.Wanted == my.WantedRunning {
				wasRunning = true
			}
		}

		// Stop running process before swap; verify shutdown before the
		// atomic-rename phase touches the binary tree.
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

		client := &http.Client{Timeout: 5 * time.Minute}
		if err := ui.StepProgress(fmt.Sprintf("Updating MySQL %s...", version),
			func(progress func(written, total int64)) (string, error) {
				if err := my.UpdateProgress(client, version, progress); err != nil {
					return "", err
				}
				return fmt.Sprintf("Updated MySQL %s", version), nil
			}); err != nil {
			return err
		}

		// Restore wanted=running iff it was running before the update.
		if wasRunning {
			if err := my.SetWanted(version, my.WantedRunning); err != nil {
				return err
			}
		}

		ui.Success(fmt.Sprintf("MySQL %s updated.", version))
		if server.IsRunning() {
			return server.SignalDaemon()
		}
		return nil
	},
}
