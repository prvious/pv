package mysql

import (
	"fmt"
	"net/http"
	"time"

	my "github.com/prvious/pv/internal/mysql"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

// Hidden debug rung. Mirrors postgres' downloadCmd: a bare tarball on
// disk without --initialize-insecure is useless, so :download collapses
// to the same call as :install. The convention from CLAUDE.md
// (download → expose) applies cleanly to PATH-exposed singletons; mysql
// is supervised, not exposed.
var downloadCmd = &cobra.Command{
	Use:     "mysql:download <version>",
	GroupID: "mysql",
	Short:   "Run the full install pipeline (debug; same as mysql:install)",
	Hidden:  true,
	Args:    cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version := args[0]
		client := &http.Client{Timeout: 5 * time.Minute}
		return ui.StepProgress(fmt.Sprintf("Downloading MySQL %s...", version),
			func(progress func(written, total int64)) (string, error) {
				if err := my.InstallProgress(client, version, progress); err != nil {
					return "", err
				}
				return fmt.Sprintf("Installed MySQL %s", version), nil
			})
	},
}
