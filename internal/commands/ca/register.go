package ca

import "github.com/spf13/cobra"

func Register(parent *cobra.Command) {
	parent.AddCommand(trustCmd)
	parent.AddCommand(untrustCmd)
	parent.AddCommand(statusCmd)
}

func RunTrust() error {
	return trustCmd.RunE(trustCmd, nil)
}

func RunUntrust() error {
	return untrustCmd.RunE(untrustCmd, nil)
}

func RunStatus() error {
	return statusCmd.RunE(statusCmd, nil)
}
