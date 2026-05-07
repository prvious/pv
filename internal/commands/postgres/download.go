package postgres

import (
	"fmt"
	"net/http"
	"time"

	pg "github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

// Hidden debug rung. Despite the name, this calls pg.InstallProgress —
// the postgres lifecycle (extract + initdb + conf + state) isn't
// meaningfully separable into "just download": a bare tarball on disk
// is useless without the other steps. The :download convention from
// CLAUDE.md applies cleanly only to PATH-exposed singletons (mago,
// composer); postgres has its own shape.
var downloadCmd = &cobra.Command{
	Use:     "postgres:download <major>",
	GroupID: "postgres",
	Short:   "Run the full install pipeline (debug; same as postgres:install)",
	Hidden:  true,
	Args:    cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		major := args[0]
		client := &http.Client{Timeout: 5 * time.Minute}
		return ui.StepProgress(fmt.Sprintf("Downloading PostgreSQL %s...", major),
			func(progress func(written, total int64)) (string, error) {
				if err := pg.InstallProgress(client, major, progress); err != nil {
					return "", err
				}
				return fmt.Sprintf("Installed PostgreSQL %s", major), nil
			})
	},
}
