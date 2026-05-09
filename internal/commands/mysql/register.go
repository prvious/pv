package mysql

import (
	"github.com/spf13/cobra"
)

// Register wires every mysql:* command onto parent. There is intentionally
// no alias namespace (no my:*) — `mysql:` is already 5 characters and an
// alias would risk colliding with a future user-facing `my:profile` or
// similar. See the spec ("Non-goals" → alias namespace).
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
		downloadCmd, // hidden; included so it's discoverable for debugging
	}
	for _, c := range cmds {
		parent.AddCommand(c)
	}
}

// Run* — convenience wrappers for orchestrators (pv install / pv update /
// pv uninstall). Mirrors the postgres pattern. Each one threads args
// through to the corresponding cobra command's RunE so behavior stays in
// a single place.
func RunInstall(args []string) error {
	return installCmd.RunE(installCmd, args)
}

func RunUpdate(args []string) error {
	return updateCmd.RunE(updateCmd, args)
}

func RunUninstall(args []string) error {
	return uninstallCmd.RunE(uninstallCmd, args)
}

// UninstallForce removes a mysql version without a confirmation prompt.
// Used by the pv uninstall orchestrator after it has already obtained
// blanket consent from the user. Mirrors postgres.UninstallForce.
func UninstallForce(version string) error {
	prev := uninstallForce
	uninstallForce = true
	defer func() { uninstallForce = prev }()
	return uninstallCmd.RunE(uninstallCmd, []string{version})
}
