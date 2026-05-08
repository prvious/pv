package mysql

import (
	"fmt"
	"time"

	"charm.land/huh/v2"
	my "github.com/prvious/pv/internal/mysql"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var uninstallForce bool

var uninstallCmd = &cobra.Command{
	Use:     "mysql:uninstall <version>",
	GroupID: "mysql",
	Short:   "Stop, remove binaries, and (with --force) DELETE the data directory",
	Long: "Stops the supervised process and removes the binary tree at " +
		"~/.pv/mysql/<version>/. With --force, also removes the data " +
		"directory at ~/.pv/data/mysql/<version>/. Unbinds linked projects " +
		"that were pointed at this version.",
	Example: `pv mysql:uninstall 8.0 --force`,
	Args:    cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version := args[0]
		if !my.IsInstalled(version) {
			ui.Subtle(fmt.Sprintf("MySQL %s is not installed.", version))
			return nil
		}
		if !uninstallForce {
			confirmed := false
			if err := huh.NewConfirm().
				Title(fmt.Sprintf("Remove MySQL %s? With --force this also DELETES the data directory. This cannot be undone.", version)).
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

		// Mark stopped + signal daemon. Verify shutdown completes (mysqld
		// can take a moment to flush InnoDB) before we remove files.
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

		if err := ui.Step(fmt.Sprintf("Uninstalling MySQL %s...", version), func() (string, error) {
			if err := my.Uninstall(version, uninstallForce); err != nil {
				return "", err
			}
			return fmt.Sprintf("Uninstalled MySQL %s", version), nil
		}); err != nil {
			return err
		}

		// Unbind from projects — keeps "9.7" bindings alive when "8.4" goes away.
		reg, err := registry.Load()
		if err != nil {
			return err
		}
		reg.UnbindMysqlVersion(version)
		if err := reg.Save(); err != nil {
			return err
		}

		ui.Success(fmt.Sprintf("MySQL %s uninstalled.", version))
		return nil
	},
}

func init() {
	uninstallCmd.Flags().BoolVar(&uninstallForce, "force", false, "Skip the confirmation prompt and delete the data directory")
}
