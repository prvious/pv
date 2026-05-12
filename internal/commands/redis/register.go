package redis

import (
	"github.com/prvious/pv/internal/config"
	"github.com/spf13/cobra"
)

func resolveVersion(args []string) string {
	if len(args) > 0 {
		return args[0]
	}
	return config.RedisDefaultVersion()
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
