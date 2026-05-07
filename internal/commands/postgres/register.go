package postgres

import (
	"strings"

	"github.com/spf13/cobra"
)

// Register wires every postgres:* command + a pg:* alias variant onto parent.
func Register(parent *cobra.Command) {
	cmds := []*cobra.Command{
		installCmd,
		uninstallCmd,
		updateCmd,
		startCmd,
		stopCmd,
		restartCmd,
		listCmd,
		logsCmd,
		statusCmd,
		downloadCmd,
	}
	for _, c := range cmds {
		parent.AddCommand(c)
		parent.AddCommand(aliasCommand(c, "postgres:", "pg:"))
	}
}

// aliasCommand returns a shallow clone of c whose Use, name, and visibility
// reflect a fromPrefix→toPrefix rewrite. The clone's RunE points at the
// original — single source of truth for the implementation.
func aliasCommand(c *cobra.Command, fromPrefix, toPrefix string) *cobra.Command {
	clone := *c
	clone.Use = strings.Replace(c.Use, fromPrefix, toPrefix, 1)
	// Mark the alias as hidden in --help to avoid duplicating every entry,
	// while still being a real, callable command.
	clone.Hidden = true
	clone.RunE = c.RunE
	return &clone
}

// Convenience wrappers for orchestrators (mirrors mago/php/composer).
func RunInstall(args []string) error {
	return installCmd.RunE(installCmd, args)
}
func RunUpdate(args []string) error {
	return updateCmd.RunE(updateCmd, args)
}
func RunUninstall(args []string) error {
	return uninstallCmd.RunE(uninstallCmd, args)
}

// UninstallForce removes a postgres major without a confirmation prompt.
// Used by the pv uninstall orchestrator which has already obtained consent.
func UninstallForce(major string) error {
	prev := uninstallForce
	uninstallForce = true
	defer func() { uninstallForce = prev }()
	return uninstallCmd.RunE(uninstallCmd, []string{major})
}
