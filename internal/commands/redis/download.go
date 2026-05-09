package redis

import (
	"net/http"
	"time"

	r "github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

// Hidden debug rung. Mirrors postgres' / mysql's downloadCmd: collapses
// to the same call as :install. Useful when poking at a half-installed
// state without going through the wizard / orchestrator.
var downloadCmd = &cobra.Command{
	Use:     "redis:download",
	GroupID: "redis",
	Short:   "Run the full install pipeline (debug; same as redis:install)",
	Hidden:  true,
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		client := &http.Client{Timeout: 5 * time.Minute}
		return ui.StepProgress("Downloading Redis...",
			func(progress func(written, total int64)) (string, error) {
				if err := r.InstallProgress(client, progress); err != nil {
					return "", err
				}
				return "Installed Redis", nil
			})
	},
}
