package postgres

import (
	"fmt"

	pg "github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/ui"
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
		// Highest installed. InstalledMajors / InstalledVersions sort
		// lexicographically — fine for current values, revisit if
		// versions ever reach 3 digits (e.g., postgres 100, mysql 10.0).
		major := majors[len(majors)-1]
		if err := pg.CreateDatabase(major, dbName); err != nil {
			return fmt.Errorf("create %s: %w", dbName, err)
		}
		ui.Success(fmt.Sprintf("Database %q created in postgres %s.", dbName, major))
		return nil
	},
}
