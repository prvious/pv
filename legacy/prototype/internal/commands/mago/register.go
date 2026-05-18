package mago

import "github.com/spf13/cobra"

func Register(parent *cobra.Command) {
	parent.AddCommand(installCmd)
	parent.AddCommand(downloadCmd)
	parent.AddCommand(pathCmd)
	parent.AddCommand(updateCmd)
	parent.AddCommand(uninstallCmd)
}

func RunInstall() error {
	return installCmd.RunE(installCmd, nil)
}

func RunDownload() error {
	return downloadCmd.RunE(downloadCmd, nil)
}

func RunUpdate() error {
	return updateCmd.RunE(updateCmd, nil)
}

func RunUninstall() error {
	return uninstallCmd.RunE(uninstallCmd, nil)
}
