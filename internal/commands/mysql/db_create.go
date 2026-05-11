package mysql

import (
	"fmt"

	my "github.com/prvious/pv/internal/mysql"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var dbCreateCmd = &cobra.Command{
	Use:     "mysql:db:create <name>",
	GroupID: "mysql",
	Short:   "Create a database in the highest-installed MySQL version",
	Args:    cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		dbName := args[0]
		versions, err := my.InstalledVersions()
		if err != nil {
			return fmt.Errorf("list installed versions: %w", err)
		}
		if len(versions) == 0 {
			return fmt.Errorf("no MySQL installed — run `pv mysql:install <version>`")
		}
		// Highest installed. InstalledMajors / InstalledVersions sort
		// lexicographically — fine for current values, revisit if
		// versions ever reach 3 digits (e.g., postgres 100, mysql 10.0).
		version := versions[len(versions)-1]
		if err := my.CreateDatabase(version, dbName); err != nil {
			return fmt.Errorf("create %s: %w", dbName, err)
		}
		ui.Success(fmt.Sprintf("Database %q created in mysql %s.", dbName, version))
		return nil
	},
}
