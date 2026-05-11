package mysql

import (
	"fmt"

	my "github.com/prvious/pv/internal/mysql"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var dbDropCmd = &cobra.Command{
	Use:     "mysql:db:drop <name>",
	GroupID: "mysql",
	Short:   "Drop a database from the highest-installed MySQL version",
	Args:    cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		dbName := args[0]
		versions, err := my.InstalledVersions()
		if err != nil {
			return fmt.Errorf("list installed versions: %w", err)
		}
		if len(versions) == 0 {
			return fmt.Errorf("no MySQL installed — nothing to drop")
		}
		// InstalledVersions sorts lexicographically — fine for current
		// values, revisit if versions ever reach 3 digits (e.g., 10.0).
		version := versions[len(versions)-1]
		if len(versions) > 1 {
			ui.Subtle(fmt.Sprintf("Using mysql %s (highest of %d installed)", version, len(versions)))
		}
		if err := my.DropDatabase(version, dbName); err != nil {
			return fmt.Errorf("drop %s: %w", dbName, err)
		}
		ui.Success(fmt.Sprintf("Database %q dropped from mysql %s.", dbName, version))
		return nil
	},
}
