// Package rustfs holds cobra commands for the rustfs:* / s3:* group.
// rustfs is the canonical command name (matching the binary); s3:* is a
// hidden alias preserving the user-facing service name from when this
// service lived under service:add s3.
package rustfs

import (
	"strings"

	"github.com/spf13/cobra"
)

// Register wires every rustfs:* command + the hidden s3:* alias variant onto parent.
func Register(parent *cobra.Command) {
	cmds := []*cobra.Command{
		installCmd,
		uninstallCmd,
		updateCmd,
		startCmd,
		stopCmd,
		restartCmd,
		statusCmd,
		logsCmd,
	}
	for _, c := range cmds {
		parent.AddCommand(c)
		parent.AddCommand(aliasCommand(c, "rustfs:", "s3:"))
	}
}

// aliasCommand returns a shallow clone of c whose Use, name, and visibility
// reflect a fromPrefix→toPrefix rewrite. The clone's RunE points at the
// original — single source of truth for the implementation. Hidden in
// --help to avoid duplicating every entry while remaining a real,
// callable command (matches the postgres / pg: pattern).
func aliasCommand(c *cobra.Command, fromPrefix, toPrefix string) *cobra.Command {
	clone := *c
	clone.Use = strings.Replace(c.Use, fromPrefix, toPrefix, 1)
	clone.Hidden = true
	clone.RunE = c.RunE
	return &clone
}

// Convenience wrappers for orchestrators (mirrors postgres/php/composer).
func RunInstall() error {
	return installCmd.RunE(installCmd, nil)
}
func RunUpdate() error {
	return updateCmd.RunE(updateCmd, nil)
}
func RunUninstall() error {
	return uninstallCmd.RunE(uninstallCmd, nil)
}
