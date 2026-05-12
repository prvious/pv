package redis

import (
	"net/http"
	"time"

	r "github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var downloadCmd = &cobra.Command{
	Use:     "redis:download [version]",
	GroupID: "redis",
	Short:   "Run the full install pipeline (debug; same as redis:install)",
	Hidden:  true,
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version, err := resolveVersion(args)
		if err != nil {
			return err
		}

		client := &http.Client{Timeout: 5 * time.Minute}
		return ui.StepProgress("Downloading Redis...",
			func(progress func(written, total int64)) (string, error) {
				if err := r.InstallProgress(client, version, progress); err != nil {
					return "", err
				}
				return "Installed Redis", nil
			})
	},
}
