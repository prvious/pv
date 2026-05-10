package mysql

import (
	"fmt"

	my "github.com/prvious/pv/internal/mysql"
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
		version := versions[len(versions)-1] // highest installed (InstalledVersions returns sorted asc)
		if err := my.CreateDatabase(version, dbName); err != nil {
			return fmt.Errorf("create %s: %w", dbName, err)
		}
		fmt.Printf("created database %q in mysql %s\n", dbName, version)
		return nil
	},
}
