package cmd

import (
	"context"
	"errors"
	"io"
	"os"

	"charm.land/fang/v2"
	"charm.land/lipgloss/v2"
	"github.com/charmbracelet/x/exp/charmtone"
	"github.com/prvious/pv/internal/ui"
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

func init() {
	rootCmd.AddGroup(
		&cobra.Group{ID: "core", Title: "Core"},
		&cobra.Group{ID: "server", Title: "Server"},
		&cobra.Group{ID: "php", Title: "PHP"},
		&cobra.Group{ID: "composer", Title: "Composer"},
		&cobra.Group{ID: "mago", Title: "Mago"},
		&cobra.Group{ID: "colima", Title: "Colima"},
		&cobra.Group{ID: "service", Title: "Services"},
		&cobra.Group{ID: "ca", Title: "CA"},
		&cobra.Group{ID: "daemon", Title: "Daemon"},
	)
}

func Execute() {
	if err := fang.Execute(context.Background(), rootCmd,
		fang.WithVersion(version),
		fang.WithColorSchemeFunc(pvColorScheme),
		fang.WithErrorHandler(pvErrorHandler),
	); err != nil {
		os.Exit(1)
	}
}

func pvColorScheme(c lipgloss.LightDarkFunc) fang.ColorScheme {
	cs := fang.DefaultColorScheme(c)
	cs.Title = charmtone.Charple
	return cs
}

func pvErrorHandler(w io.Writer, styles fang.Styles, err error) {
	if errors.Is(err, ui.ErrAlreadyPrinted) {
		return
	}
	fang.DefaultErrorHandler(w, styles, err)
}
