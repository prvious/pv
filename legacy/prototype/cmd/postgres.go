package cmd

import (
	postgres "github.com/prvious/pv/internal/commands/postgres"
	"github.com/spf13/cobra"
)

func init() {
	rootCmd.AddGroup(&cobra.Group{
		ID:    "postgres",
		Title: "PostgreSQL Management:",
	})
	postgres.Register(rootCmd)
}
