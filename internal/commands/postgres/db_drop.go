package postgres

import (
	"fmt"

	pg "github.com/prvious/pv/internal/postgres"
	"github.com/spf13/cobra"
)

var dbDropCmd = &cobra.Command{
	Use:     "postgres:db:drop <name>",
	GroupID: "postgres",
	Short:   "Drop a database from the highest-installed PostgreSQL major",
	Args:    cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		dbName := args[0]
		majors, err := pg.InstalledMajors()
		if err != nil {
			return fmt.Errorf("list installed majors: %w", err)
		}
		if len(majors) == 0 {
			return fmt.Errorf("no PostgreSQL installed")
		}
		major := majors[len(majors)-1]
		if err := pg.DropDatabase(major, dbName); err != nil {
			return fmt.Errorf("drop %s: %w", dbName, err)
		}
		fmt.Printf("dropped database %q from postgres %s\n", dbName, major)
		return nil
	},
}
