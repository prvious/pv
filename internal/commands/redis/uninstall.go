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
	Use:     "redis:uninstall [version]",
	GroupID: "redis",
	Short:   "Stop, remove the binary, and (with --force) DELETE the data directory",
	Long: "Stops the supervised process and removes the binary tree at " +
		"~/.pv/redis/{version}/. With --force, also removes the data directory at " +
		"~/.pv/data/redis/{version}/ (deletes dump.rdb). Unbinds linked projects using that version.",
	Example: `pv redis:uninstall --force`,
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version, err := resolveVersion(args)
		if err != nil {
			return err
		}

		if !r.IsInstalled(version) {
			ui.Subtle(fmt.Sprintf("Redis %s is not installed.", version))
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

		if err := r.SetWanted(version, r.WantedStopped); err != nil {
			return err
		}
		if server.IsRunning() {
			if err := server.SignalDaemon(); err != nil {
				return fmt.Errorf("signal daemon: %w", err)
			}
			if err := r.WaitStopped(version, 10*time.Second); err != nil {
				return fmt.Errorf("waiting for redis to stop: %w", err)
			}
		}

		if err := ui.Step("Uninstalling Redis...", func() (string, error) {
			if err := r.Uninstall(version, uninstallForce); err != nil {
				return "", err
			}
			return fmt.Sprintf("Uninstalled Redis %s", version), nil
		}); err != nil {
			return err
		}

		ui.Success(fmt.Sprintf("Redis %s uninstalled.", version))
		return nil
	},
}

func init() {
	uninstallCmd.Flags().BoolVar(&uninstallForce, "force", false, "Skip the confirmation prompt and delete the data directory")
}
