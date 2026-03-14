package php

import (
	"fmt"
	"os"
	"strings"

	"github.com/prvious/pv/internal/phpenv"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var listCmd = &cobra.Command{
	Use:     "php:list",
	GroupID: "php",
	Short:   "List installed PHP versions",
	RunE: func(cmd *cobra.Command, args []string) error {
		versions, err := phpenv.InstalledVersions()
		if err != nil {
			return err
		}
		if len(versions) == 0 {
			fmt.Fprintln(os.Stderr)
			ui.Subtle("No PHP versions installed. Run: pv php:install [version]")
			fmt.Fprintln(os.Stderr)
			return nil
		}

		globalV, _ := phpenv.GlobalVersion()

		// Load registry to show which projects use which version.
		reg, _ := registry.Load()
		versionProjects := make(map[string][]string)
		if reg != nil {
			for _, p := range reg.List() {
				v := p.PHP
				if v == "" {
					v = globalV
				}
				if v != "" {
					versionProjects[v] = append(versionProjects[v], p.Name)
				}
			}
		}

		fmt.Fprintln(os.Stderr)
		for _, v := range versions {
			var parts []string

			// Version number
			if v == globalV {
				parts = append(parts, ui.Green.Bold(true).Render(v))
				parts = append(parts, ui.Muted.Render("(default)"))
			} else {
				parts = append(parts, ui.Accent.Render(v))
			}

			// Projects using this version
			if projects, ok := versionProjects[v]; ok && len(projects) > 0 {
				parts = append(parts, ui.Muted.Render("← "+strings.Join(projects, ", ")))
			}

			fmt.Fprintf(os.Stderr, "  %s %s\n",
				ui.Green.Render("●"),
				strings.Join(parts, " "),
			)
		}
		fmt.Fprintln(os.Stderr)

		return nil
	},
}
