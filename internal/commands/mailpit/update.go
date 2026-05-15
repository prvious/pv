package mailpit

import (
	"fmt"
	"net/http"
	"time"

	pkg "github.com/prvious/pv/internal/mailpit"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var updateCmd = &cobra.Command{
	Use:     "mailpit:update [version]",
	GroupID: "mailpit",
	Short:   "Re-download the Mailpit binary for a version",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		resolved, err := pkg.ResolveVersion(argVersion(args))
		if err != nil {
			return err
		}
		if !pkg.IsInstalled(resolved) {
			return fmt.Errorf("%s %s is not installed", pkg.Binary().Name, resolved)
		}
		wasRunning := false
		st, err := pkg.LoadState()
		if err != nil {
			return fmt.Errorf("load mailpit state: %w", err)
		}
		if entry, ok := st.Versions[resolved]; ok && entry.Wanted == pkg.WantedRunning {
			wasRunning = true
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
		client := &http.Client{Timeout: 5 * time.Minute}
		if err := ui.StepProgress(fmt.Sprintf("Updating %s %s...", pkg.DisplayName(), resolved), func(progress func(written, total int64)) (string, error) {
			if err := pkg.UpdateProgress(client, resolved, progress); err != nil {
				return "", err
			}
			return fmt.Sprintf("Updated %s %s", pkg.DisplayName(), resolved), nil
		}); err != nil {
			return err
		}
		if wasRunning {
			if err := pkg.SetWanted(resolved, pkg.WantedRunning); err != nil {
				return err
			}
		}
		ui.Success(fmt.Sprintf("%s %s updated.", pkg.DisplayName(), resolved))
		return signalDaemon(pkg.DisplayName())
	},
}
