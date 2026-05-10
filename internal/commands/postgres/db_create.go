package postgres

import (
	"fmt"

	pg "github.com/prvious/pv/internal/postgres"
	"github.com/spf13/cobra"
)

var dbCreateCmd = &cobra.Command{
	Use:     "postgres:db:create <name>",
	GroupID: "postgres",
	Short:   "Create a database in the highest-installed PostgreSQL major",
	Args:    cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		dbName := args[0]
		majors, err := pg.InstalledMajors()
		if err != nil {
			return fmt.Errorf("list installed majors: %w", err)
		}
		if len(majors) == 0 {
			return fmt.Errorf("no PostgreSQL installed — run `pv postgres:install <major>`")
		}
		major := majors[len(majors)-1] // highest installed (InstalledMajors returns sorted asc)
		if err := pg.CreateDatabase(major, dbName); err != nil {
			return fmt.Errorf("create %s: %w", dbName, err)
		}
		fmt.Printf("created database %q in postgres %s\n", dbName, major)
		return nil
	},
}
