package cmd

import (
	"github.com/spf13/cobra"
)

var phpCmd = &cobra.Command{
	Use:   "php",
	Short: "Manage PHP versions",
}

func init() {
	rootCmd.AddCommand(phpCmd)
}
