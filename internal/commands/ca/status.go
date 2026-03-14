package ca

import (
	"fmt"
	"os"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/setup"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var statusCmd = &cobra.Command{
	Use:     "ca:status",
	Aliases: []string{"ca:check"},
	GroupID: "ca",
	Short:   "Show pv local CA trust status",
	RunE: func(cmd *cobra.Command, args []string) error {
		caPath := config.CACertPath()
		if _, err := os.Stat(caPath); err != nil {
			ui.Subtle(fmt.Sprintf("CA certificate not found at %s", caPath))
			return nil
		}

		trusted, err := setup.IsCATrusted()
		if err != nil {
			return fmt.Errorf("cannot check CA trust status: %w", err)
		}

		if trusted {
			ui.Success("CA certificate is trusted")
		} else {
			ui.Subtle("CA certificate is not trusted")
		}
		ui.Subtle(fmt.Sprintf("CA path: %s", caPath))
		return nil
	},
}
