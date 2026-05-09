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
