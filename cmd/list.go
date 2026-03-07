package cmd

import (
	"fmt"
	"os"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var listCmd = &cobra.Command{
	Use:     "list",
	GroupID: "core",
	Aliases: []string{"ls"},
	Short:   "List linked projects",
	RunE: func(cmd *cobra.Command, args []string) error {
		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}

		settings, _ := config.LoadSettings()
		tld := "test"
		if settings != nil {
			tld = settings.TLD
		}

		projects := reg.List()
		if len(projects) == 0 {
			fmt.Fprintln(os.Stderr)
			ui.Subtle("No projects linked yet. Run pv link in a project directory to get started.")
			fmt.Fprintln(os.Stderr)
			return nil
		}

		fmt.Fprintln(os.Stderr)
		rows := projectTableRows(projects, tld)
		ui.Table([]string{"Site", "Type", "PHP", "Path"}, rows)
		fmt.Fprintln(os.Stderr)

		return nil
	},
}

func projectTableRows(projects []registry.Project, tld string) [][]string {
	rows := make([][]string, len(projects))
	for i, p := range projects {
		typ := p.Type
		if typ == "" {
			typ = "unknown"
		}
		php := p.PHP
		if php == "" {
			php = "-"
		}
		domain := "https://" + p.Name + "." + tld

		rows[i] = []string{domain, typ, php, p.Path}
	}
	return rows
}

func init() {
	rootCmd.AddCommand(listCmd)
}
