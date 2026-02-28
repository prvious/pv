package cmd

import (
	"fmt"
	"net/http"
	"regexp"

	"github.com/prvious/pv/internal/phpenv"
	"github.com/spf13/cobra"
)

var validPHPVersion = regexp.MustCompile(`^\d+\.\d+$`)

var phpInstallCmd = &cobra.Command{
	Use:   "install <version>",
	Short: "Install a PHP version (e.g., pv php install 8.4)",
	Args:  cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version := args[0]
		if !validPHPVersion.MatchString(version) {
			return fmt.Errorf("invalid version format %q: use major.minor (e.g., 8.4)", version)
		}

		if phpenv.IsInstalled(version) {
			return fmt.Errorf("PHP %s is already installed", version)
		}

		fmt.Printf("Installing PHP %s...\n", version)
		client := &http.Client{}
		if err := phpenv.Install(client, version); err != nil {
			return err
		}

		// If no global default, set this as the default.
		if _, err := phpenv.GlobalVersion(); err != nil {
			fmt.Printf("Setting PHP %s as global default...\n", version)
			if err := phpenv.SetGlobal(version); err != nil {
				return err
			}
		}

		return nil
	},
}

func init() {
	phpCmd.AddCommand(phpInstallCmd)
}
