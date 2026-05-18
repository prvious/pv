package cmd

import (
	mailpit "github.com/prvious/pv/internal/commands/mailpit"
	"github.com/spf13/cobra"
)

func init() {
	rootCmd.AddGroup(&cobra.Group{
		ID:    "mailpit",
		Title: "Mailpit (Mail) Management:",
	})
	mailpit.Register(rootCmd)
}
