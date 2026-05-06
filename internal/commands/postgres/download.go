package postgres

import (
	"fmt"
	"net/http"
	"time"

	pg "github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var downloadCmd = &cobra.Command{
	Use:     "postgres:download <major>",
	GroupID: "postgres",
	Short:   "Download a PostgreSQL tarball into private storage",
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
