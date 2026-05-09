package redis

import (
	"fmt"
	"time"

	"charm.land/huh/v2"
	r "github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var uninstallForce bool

var uninstallCmd = &cobra.Command{
	Use:     "redis:uninstall",
	GroupID: "redis",
	Short:   "Stop, remove the binary, and (with --force) DELETE the data directory",
	Long: "Stops the supervised process and removes the binary tree at " +
		"~/.pv/redis/. With --force, also removes the data directory at " +
		"~/.pv/data/redis/ (deletes dump.rdb). Unbinds every linked project.",
	Example: `pv redis:uninstall --force`,
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		if !r.IsInstalled() {
			ui.Subtle("Redis is not installed.")
			return nil
		}
		if !uninstallForce {
			confirmed := false
			if err := huh.NewConfirm().
				Title("Remove Redis? With --force this also DELETES the data directory. This cannot be undone.").
				Affirmative("Yes").
				Negative("No").
				Value(&confirmed).
				Run(); err != nil {
				return err
			}
			if !confirmed {
				return fmt.Errorf("aborted")
			}
		}

		if err := r.SetWanted(r.WantedStopped); err != nil {
			return err
		}
		if server.IsRunning() {
			if err := server.SignalDaemon(); err != nil {
				return fmt.Errorf("signal daemon: %w", err)
			}
			if err := r.WaitStopped(10 * time.Second); err != nil {
				return fmt.Errorf("waiting for redis to stop: %w", err)
			}
		}

		if err := ui.Step("Uninstalling Redis...", func() (string, error) {
			if err := r.Uninstall(uninstallForce); err != nil {
				return "", err
			}
			return "Uninstalled Redis", nil
		}); err != nil {
			return err
		}

		// NOTE: Uninstall handles the registry unbind internally. We do NOT
		// re-save the registry here — registry.Save calls config.EnsureDirs
		// and any subsequent EnsureDirs call after Uninstall ran would have
		// historically recreated RedisDir/RedisDataDir. Even now that those
		// are no longer in EnsureDirs, a redundant Save is just churn.

		ui.Success("Redis uninstalled.")
		return nil
	},
}

func init() {
	uninstallCmd.Flags().BoolVar(&uninstallForce, "force", false, "Skip the confirmation prompt and delete the data directory")
}
