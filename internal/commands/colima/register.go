package colima

import "github.com/spf13/cobra"

func Register(parent *cobra.Command) {
	parent.AddCommand(installCmd)
	parent.AddCommand(downloadCmd)
	parent.AddCommand(pathCmd)
	parent.AddCommand(updateCmd)
	parent.AddCommand(uninstallCmd)
	parent.AddCommand(stopCmd)
}

func RunInstall() error {
	return installCmd.RunE(installCmd, nil)
}

func RunUpdate() error {
	return updateCmd.RunE(updateCmd, nil)
}

func RunUninstall() error {
	return uninstallCmd.RunE(uninstallCmd, nil)
}

func RunStop() error {
	return stopCmd.RunE(stopCmd, nil)
}
