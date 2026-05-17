package cmd

import (
	mysql "github.com/prvious/pv/internal/commands/mysql"
	"github.com/spf13/cobra"
)

func init() {
	rootCmd.AddGroup(&cobra.Group{
		ID:    "mysql",
		Title: "MySQL Management:",
	})
	mysql.Register(rootCmd)
}
