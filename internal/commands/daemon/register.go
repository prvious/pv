package daemon

import "github.com/spf13/cobra"

func Register(parent *cobra.Command) {
	parent.AddCommand(enableCmd)
	parent.AddCommand(disableCmd)
	parent.AddCommand(restartCmd)
}

func RunEnable() error {
	return enableCmd.RunE(enableCmd, nil)
}

func RunRestart() error {
	return restartCmd.RunE(restartCmd, nil)
}
