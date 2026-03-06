package cmd

import (
	"fmt"
	"os"
	"regexp"

	"github.com/prvious/pv/internal/phpenv"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var phpRemoveCmd = &cobra.Command{
	Use:   "php:remove <version>",
	Short: "Remove an installed PHP version",
	Args:  cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version := args[0]
		if !regexp.MustCompile(`^\d+\.\d+$`).MatchString(version) {
			fmt.Fprintln(os.Stderr)
			ui.Fail(fmt.Sprintf("Invalid version format %s", ui.Bold.Render(version)))
			ui.FailDetail("Use major.minor (e.g., 8.4)")
			fmt.Fprintln(os.Stderr)
			cmd.SilenceUsage = true
			return ui.ErrAlreadyPrinted
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
					fmt.Fprintln(os.Stderr)
					ui.Fail(fmt.Sprintf("Cannot remove PHP %s", ui.Bold.Render(version)))
					ui.FailDetail(fmt.Sprintf("Project %s depends on it", ui.Bold.Render(p.Name)))
					fmt.Fprintln(os.Stderr)
					cmd.SilenceUsage = true
					return ui.ErrAlreadyPrinted
				}
			}
		}

		fmt.Fprintln(os.Stderr)

		if err := ui.Step("Removing PHP "+version+"...", func() (string, error) {
			if err := phpenv.Remove(version); err != nil {
				return "", err
			}
			return fmt.Sprintf("PHP %s removed", version), nil
		}); err != nil {
			return err
		}

		fmt.Fprintln(os.Stderr)
		return nil
	},
}

func init() {
	rootCmd.AddCommand(phpRemoveCmd)
}
