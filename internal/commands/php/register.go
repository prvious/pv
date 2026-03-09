package php

import "github.com/spf13/cobra"

func Register(parent *cobra.Command) {
	parent.AddCommand(installCmd)
	parent.AddCommand(downloadCmd)
	parent.AddCommand(pathCmd)
	parent.AddCommand(updateCmd)
	parent.AddCommand(uninstallCmd)
	parent.AddCommand(useCmd)
	parent.AddCommand(listCmd)
	parent.AddCommand(removeCmd)
	parent.AddCommand(currentCmd)
}

func RunInstall(args []string) error {
	return installCmd.RunE(installCmd, args)
}

func RunUpdate() error {
	return updateCmd.RunE(updateCmd, nil)
}

func RunUninstall() error {
	return uninstallCmd.RunE(uninstallCmd, nil)
}
