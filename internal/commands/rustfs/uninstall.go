package rustfs

import (
	"fmt"

	"charm.land/huh/v2"
	"github.com/prvious/pv/internal/registry"
	pkg "github.com/prvious/pv/internal/rustfs"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var uninstallForce bool

var uninstallCmd = &cobra.Command{
	Use:     "rustfs:uninstall",
	GroupID: "rustfs",
	Short:   "Stop, remove the binary, and DELETE the data directory",
	Long:    "Stops the supervised process, removes the rustfs binary, deletes the data directory, and unbinds linked projects. Data deletion is irreversible.",
	Example: `pv rustfs:uninstall --force`,
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}
		if _, ok := reg.Services["s3"]; !ok {
			ui.Subtle("RustFS is not installed.")
			return nil
		}
		if !uninstallForce {
			confirmed := false
			if err := huh.NewConfirm().
				Title("Remove RustFS and DELETE its data directory? This cannot be undone.").
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
		if err := pkg.Uninstall(true); err != nil {
			return err
		}
		ui.Success("RustFS uninstalled.")
		return nil
	},
}

func init() {
	uninstallCmd.Flags().BoolVar(&uninstallForce, "force", false, "Skip the confirmation prompt")
}
