package mysql

import (
	"fmt"

	my "github.com/prvious/pv/internal/mysql"
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
			return fmt.Errorf("no MySQL installed")
		}
		version := versions[len(versions)-1]
		if err := my.DropDatabase(version, dbName); err != nil {
			return fmt.Errorf("drop %s: %w", dbName, err)
		}
		fmt.Printf("dropped database %q from mysql %s\n", dbName, version)
		return nil
	},
}
