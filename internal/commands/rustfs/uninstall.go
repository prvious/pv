package rustfs

import (
	"fmt"
	"time"

	"charm.land/huh/v2"
	"github.com/prvious/pv/internal/caddy"
	pkg "github.com/prvious/pv/internal/rustfs"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var uninstallForce bool

var uninstallCmd = &cobra.Command{
	Use:     "rustfs:uninstall [version]",
	GroupID: "rustfs",
	Short:   "Stop, remove the binary, and DELETE the data directory",
	Long:    "Stops the supervised process, removes the rustfs binary, deletes the data directory, and unbinds linked projects. Data deletion is irreversible.",
	Example: `pv rustfs:uninstall --force`,
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		resolved, err := pkg.ResolveVersion(argVersion(args))
		if err != nil {
			return err
		}
		if !pkg.IsInstalled(resolved) {
			ui.Subtle(fmt.Sprintf("%s %s is not installed.", pkg.DisplayName(), resolved))
			return nil
		}
		if !uninstallForce {
			confirmed := false
			if err := huh.NewConfirm().
				Title(fmt.Sprintf("Remove %s %s and DELETE its data directory? This cannot be undone.", pkg.DisplayName(), resolved)).
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
		reg, err := registry.Load()
		if err != nil {
			return err
		}
		pkg.ApplyFallbacksToLinkedProjects(reg)
		if err := pkg.Uninstall(resolved, true); err != nil {
			return err
		}
		if err := caddy.GenerateServiceSiteConfigs(nil); err != nil {
			ui.Subtle(fmt.Sprintf("Could not regenerate service site config: %v", err))
		}
		ui.Success(fmt.Sprintf("%s %s uninstalled.", pkg.DisplayName(), resolved))
		return nil
	},
}

func init() {
	uninstallCmd.Flags().BoolVar(&uninstallForce, "force", false, "Skip the confirmation prompt")
}
