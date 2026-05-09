// Package redis holds cobra commands for the redis:* group. There is
// intentionally no alias namespace — `redis:` is already short.
package redis

import (
	"github.com/spf13/cobra"
)

// Register wires every redis:* command onto parent.
func Register(parent *cobra.Command) {
	cmds := []*cobra.Command{
		installCmd,
		uninstallCmd,
		updateCmd,
		startCmd,
		stopCmd,
		restartCmd,
		downloadCmd, // hidden; included so it's discoverable for debugging
	}
	for _, c := range cmds {
		parent.AddCommand(c)
	}
}

// Run* — convenience wrappers for orchestrators (pv install / pv update /
// pv uninstall) and the setup wizard. Each one threads args through to
// the corresponding cobra command's RunE so behavior stays in a single
// place.
func RunInstall(args []string) error {
	return installCmd.RunE(installCmd, args)
}

func RunUpdate(args []string) error {
	return updateCmd.RunE(updateCmd, args)
}

func RunUninstall(args []string) error {
	return uninstallCmd.RunE(uninstallCmd, args)
}

// UninstallForce removes redis without a confirmation prompt. Used by
// the pv uninstall orchestrator after it has already obtained blanket
// consent from the user. Mirrors postgres.UninstallForce / mysql.UninstallForce.
func UninstallForce() error {
	prev := uninstallForce
	uninstallForce = true
	defer func() { uninstallForce = prev }()
	return uninstallCmd.RunE(uninstallCmd, nil)
}
