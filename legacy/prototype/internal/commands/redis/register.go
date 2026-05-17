package redis

import (
	r "github.com/prvious/pv/internal/redis"
	"github.com/spf13/cobra"
)

func resolveVersion(args []string) (string, error) {
	version := ""
	if len(args) > 0 {
		version = args[0]
	}
	return r.ResolveVersion(version)
}

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
	}
}

func RunInstall(args []string) error {
	return installCmd.RunE(installCmd, args)
}

func RunUpdate(args []string) error {
	return updateCmd.RunE(updateCmd, args)
}

func RunUninstall(args []string) error {
	return uninstallCmd.RunE(uninstallCmd, args)
}

func UninstallForce(version string) error {
	prev := uninstallForce
	uninstallForce = true
	defer func() { uninstallForce = prev }()
	return uninstallCmd.RunE(uninstallCmd, []string{version})
}
