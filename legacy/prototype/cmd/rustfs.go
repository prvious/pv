package cmd

import (
	rustfs "github.com/prvious/pv/internal/commands/rustfs"
	"github.com/spf13/cobra"
)

func init() {
	rootCmd.AddGroup(&cobra.Group{
		ID:    "rustfs",
		Title: "RustFS (S3) Management:",
	})
	rustfs.Register(rootCmd)
}
