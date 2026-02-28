package cmd

import (
	"fmt"
	"strings"

	"github.com/prvious/pv/internal/phpenv"
	"github.com/prvious/pv/internal/registry"
	"github.com/spf13/cobra"
)

var phpListCmd = &cobra.Command{
	Use:     "list",
	Aliases: []string{"ls"},
	Short:   "List installed PHP versions",
	RunE: func(cmd *cobra.Command, args []string) error {
		versions, err := phpenv.InstalledVersions()
		if err != nil {
			return err
		}
		if len(versions) == 0 {
			fmt.Println("No PHP versions installed. Run: pv php install <version>")
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

		for _, v := range versions {
			marker := "  "
			if v == globalV {
				marker = "* "
			}

			line := marker + v
			if projects, ok := versionProjects[v]; ok && len(projects) > 0 {
				line += "  <- " + strings.Join(projects, ", ")
			}
			if v == globalV {
				line += " (default)"
			}
			fmt.Println(line)
		}

		return nil
	},
}

func init() {
	phpCmd.AddCommand(phpListCmd)
}
