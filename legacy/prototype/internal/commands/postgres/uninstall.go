package postgres

import (
	"fmt"
	"time"

	"charm.land/huh/v2"
	pg "github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var uninstallForce bool

var uninstallCmd = &cobra.Command{
	Use:     "postgres:uninstall <major>",
	GroupID: "postgres",
	Short:   "Stop, remove data, and remove a PostgreSQL major",
	Long:    "Stops the supervised process, deletes the data directory, removes binaries and logs, and unbinds linked projects.",
	Example: `pv postgres:uninstall 17 --force`,
	Args:    cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		major := args[0]
		if !pg.IsInstalled(major) {
			ui.Subtle(fmt.Sprintf("PostgreSQL %s is not installed.", major))
			return nil
		}
		if !uninstallForce {
			confirmed := false
			if err := huh.NewConfirm().
				Title(fmt.Sprintf("Remove PostgreSQL %s and DELETE its data directory? This cannot be undone.", major)).
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

		// Mark stopped + signal daemon to bring the process down. We must
		// verify the process actually stopped before doing destructive
		// on-disk operations — postgres doing a WAL flush would still be
		// writing if we proceeded after a fixed sleep.
		if err := pg.SetWanted(major, pg.WantedStopped); err != nil {
			return err
		}
		if server.IsRunning() {
			if err := server.SignalDaemon(); err != nil {
				return fmt.Errorf("signal daemon: %w", err)
			}
			if err := pg.WaitStopped(major, 30*time.Second); err != nil {
				return fmt.Errorf("waiting for postgres %s to stop: %w", major, err)
			}
		}

		if err := pg.Uninstall(major); err != nil {
			return err
		}

		// Unbind from projects.
		reg, err := registry.Load()
		if err != nil {
			return err
		}
		reg.UnbindPostgresMajor(major)
		if err := reg.Save(); err != nil {
			return err
		}

		ui.Success(fmt.Sprintf("PostgreSQL %s uninstalled.", major))
		return nil
	},
}

func init() {
	uninstallCmd.Flags().BoolVar(&uninstallForce, "force", false, "Skip the confirmation prompt")
}
