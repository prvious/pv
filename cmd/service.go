package cmd

import (
	"github.com/spf13/cobra"
)

var serviceCmd = &cobra.Command{
	Use:     "service",
	Aliases: []string{"svc"},
	Short:   "Manage backing services (MySQL, PostgreSQL, Redis, RustFS)",
}

func init() {
	rootCmd.AddCommand(serviceCmd)
}
