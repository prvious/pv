package ca

import (
	"errors"
	"fmt"
	"os"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/setup"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var untrustCmd = &cobra.Command{
	Use:     "ca:untrust",
	GroupID: "ca",
	Short:   "Remove pv's local CA from the macOS System keychain",
	RunE: func(cmd *cobra.Command, args []string) error {
		setup.Verbose = true
		defer func() { setup.Verbose = false }()

		if _, err := os.Stat(config.CACertPath()); err != nil {
			return fmt.Errorf("CA certificate not found at %s", config.CACertPath())
		}

		if err := acquireSudo(); err != nil {
			return err
		}

		if err := ui.Step("Removing CA certificate...", func() (string, error) {
			if err := setup.RunSudoUntrustCACert(); err != nil {
				return "", err
			}

			trusted, err := setup.IsCATrusted()
			if err != nil {
				return "", err
			}
			if trusted {
				return "", fmt.Errorf("CA certificate is still trusted")
			}

			return "CA certificate removed", nil
		}); err != nil {
			if !errors.Is(err, ui.ErrAlreadyPrinted) {
				return fmt.Errorf("cannot remove CA certificate: %w", err)
			}
			return err
		}

		return nil
	},
}
