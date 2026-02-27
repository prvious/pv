package cmd

import (
	"fmt"
	"text/tabwriter"

	"github.com/prvious/pv/internal/registry"
	"github.com/spf13/cobra"
)

var listCmd = &cobra.Command{
	Use:     "list",
	Aliases: []string{"ls"},
	Short:   "List linked projects",
	RunE: func(cmd *cobra.Command, args []string) error {
		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}

		projects := reg.List()
		if len(projects) == 0 {
			fmt.Println("No projects linked yet. Run `pv link` in a project directory to get started.")
			return nil
		}

		w := tabwriter.NewWriter(cmd.OutOrStdout(), 0, 0, 2, ' ', 0)
		fmt.Fprintln(w, "NAME\tPATH\tTYPE")
		for _, p := range projects {
			typ := p.Type
			if typ == "" {
				typ = "-"
			}
			fmt.Fprintf(w, "%s\t%s\t%s\n", p.Name, p.Path, typ)
		}
		return w.Flush()
	},
}

func init() {
	rootCmd.AddCommand(listCmd)
}
