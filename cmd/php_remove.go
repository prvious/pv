package cmd

import (
	"fmt"
	"regexp"

	"github.com/prvious/pv/internal/phpenv"
	"github.com/prvious/pv/internal/registry"
	"github.com/spf13/cobra"
)

var phpRemoveCmd = &cobra.Command{
	Use:   "remove <version>",
	Short: "Remove an installed PHP version",
	Args:  cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version := args[0]
		if !regexp.MustCompile(`^\d+\.\d+$`).MatchString(version) {
			return fmt.Errorf("invalid version format %q: use major.minor (e.g., 8.4)", version)
		}

		// Check if any linked projects depend on this version.
		reg, err := registry.Load()
		if err == nil {
			globalV, _ := phpenv.GlobalVersion()
			for _, p := range reg.List() {
				v := p.PHP
				if v == "" {
					v = globalV
				}
				if v == version {
					return fmt.Errorf("cannot remove PHP %s: project %q depends on it", version, p.Name)
				}
			}
		}

		if err := phpenv.Remove(version); err != nil {
			return err
		}

		fmt.Printf("PHP %s removed\n", version)
		return nil
	},
}

func init() {
	phpCmd.AddCommand(phpRemoveCmd)
}
