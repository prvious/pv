package cmd

import (
	rediscmd "github.com/prvious/pv/internal/commands/redis"
	"github.com/spf13/cobra"
)

func init() {
	rootCmd.AddGroup(&cobra.Group{
		ID:    "redis",
		Title: "Redis Management:",
	})
	rediscmd.Register(rootCmd)
}
