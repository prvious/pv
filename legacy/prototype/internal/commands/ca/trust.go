package ca

import (
	"errors"
	"fmt"

	"github.com/prvious/pv/internal/setup"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var trustCmd = &cobra.Command{
	Use:     "ca:trust",
	GroupID: "ca",
	Short:   "Trust pv's local CA in the macOS System keychain",
	RunE: func(cmd *cobra.Command, args []string) error {
		setup.Verbose = true
		defer func() { setup.Verbose = false }()

		if err := acquireSudo(); err != nil {
			return err
		}

		if err := ui.Step("Trusting CA certificate...", func() (string, error) {
			if err := setup.RunSudoTrustWithServer(); err != nil {
				return "", err
			}

			trusted, err := setup.IsCATrusted()
			if err != nil {
				return "", err
			}
			if !trusted {
				return "", fmt.Errorf("CA certificate is still not trusted")
			}

			return "CA certificate trusted", nil
		}); err != nil {
			if !errors.Is(err, ui.ErrAlreadyPrinted) {
				return fmt.Errorf("cannot trust CA certificate: %w", err)
			}
			return err
		}

		return nil
	},
}
