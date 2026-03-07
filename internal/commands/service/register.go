package service

import "github.com/spf13/cobra"

func Register(parent *cobra.Command) {
	parent.AddCommand(addCmd)
	parent.AddCommand(startCmd)
	parent.AddCommand(stopCmd)
	parent.AddCommand(statusCmd)
	parent.AddCommand(listCmd)
	parent.AddCommand(envCmd)
	parent.AddCommand(removeCmd)
	parent.AddCommand(destroyCmd)
	parent.AddCommand(logsCmd)
}

func RunAdd(args []string) error {
	return addCmd.RunE(addCmd, args)
}
