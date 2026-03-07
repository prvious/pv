package cmd

import (
	"context"
	"os"

	"charm.land/fang/v2"
	"charm.land/lipgloss/v2"
	"github.com/charmbracelet/x/exp/charmtone"
	"github.com/spf13/cobra"
)

// version is set at build time via ldflags:
//
//	go build -ldflags "-X github.com/prvious/pv/cmd.version=1.0.0"
var version = "dev"

var rootCmd = &cobra.Command{
	Use:   "pv",
	Short: "Local dev server manager powered by FrankenPHP",
}

func Execute() {
	if err := fang.Execute(context.Background(), rootCmd,
		fang.WithVersion(version),
		fang.WithColorSchemeFunc(pvColorScheme),
	); err != nil {
		os.Exit(1)
	}
}

func pvColorScheme(c lipgloss.LightDarkFunc) fang.ColorScheme {
	cs := fang.DefaultColorScheme(c)
	cs.Title = charmtone.Charple
	return cs
}
