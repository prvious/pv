package cmd

import (
	"errors"
	"fmt"
	"os"

	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

// version is set at build time via ldflags:
//
//	go build -ldflags "-X github.com/prvious/pv/cmd.version=1.0.0"
var version = "dev"

var rootCmd = &cobra.Command{
	Use:          "pv",
	Short:        "Local dev server manager powered by FrankenPHP",
	Version:      version,
	SilenceErrors: true,
}

func Execute() {
	if err := rootCmd.Execute(); err != nil {
		// If the error was already printed with styled output, just exit.
		if errors.Is(err, ui.ErrAlreadyPrinted) {
			os.Exit(1)
		}
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
