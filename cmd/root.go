package cmd

import (
	"fmt"
	"os"

	"github.com/spf13/cobra"
)

// version is set at build time via ldflags:
//
//	go build -ldflags "-X github.com/prvious/pv/cmd.version=1.0.0"
var version = "dev"

var rootCmd = &cobra.Command{
	Use:     "pv",
	Short:   "Local dev server manager powered by FrankenPHP",
	Version: version,
}

func Execute() {
	if err := rootCmd.Execute(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
