package mailpit

import (
	"fmt"

	"charm.land/huh/v2"
	pkg "github.com/prvious/pv/internal/mailpit"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var uninstallForce bool

var uninstallCmd = &cobra.Command{
	Use:     "mailpit:uninstall",
	GroupID: "mailpit",
	Short:   "Stop, remove the binary, and DELETE the data directory",
	Long:    "Stops the supervised process, removes the mailpit binary, deletes the data directory, and unbinds linked projects. Data deletion is irreversible.",
	Example: `pv mailpit:uninstall --force`,
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}
		if _, ok := reg.Services["mail"]; !ok {
			ui.Subtle("Mailpit is not installed.")
			return nil
		}
		if !uninstallForce {
			confirmed := false
			if err := huh.NewConfirm().
				Title("Remove Mailpit and DELETE its data directory? This cannot be undone.").
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
		if err := pkg.Uninstall(pkg.DefaultVersion(), true); err != nil {
			return err
		}
		ui.Success("Mailpit uninstalled.")
		return nil
	},
}

func init() {
	uninstallCmd.Flags().BoolVar(&uninstallForce, "force", false, "Skip the confirmation prompt")
}
