package cmd

import (
	"fmt"
	"strings"
	"time"

	"net/http"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var updateVerbose bool

var updateCmd = &cobra.Command{
	Use:   "update",
	Short: "Download and update all managed binaries",
	RunE: func(cmd *cobra.Command, args []string) error {
		start := time.Now()

		binaries.Verbose = updateVerbose

		ui.Header(version)

		client := &http.Client{}

		// Step 1: Check for updates.
		vs, err := binaries.LoadVersions()
		if err != nil {
			return fmt.Errorf("cannot load version state: %w", err)
		}

		type updateInfo struct {
			binary  binaries.Binary
			latest  string
			current string
			needed  bool
		}

		var updates []updateInfo
		var anyNeeded bool

		if err := ui.Step("Checking for updates...", func() (string, error) {
			for _, b := range binaries.Tools() {
				latest, err := binaries.FetchLatestVersion(client, b)
				if err != nil {
					return "", fmt.Errorf("cannot check %s version: %w", b.DisplayName, err)
				}
				needed := binaries.NeedsUpdate(vs, b, latest)
				if needed {
					anyNeeded = true
				}
				updates = append(updates, updateInfo{
					binary:  b,
					latest:  latest,
					current: vs.Get(b.Name),
					needed:  needed,
				})
			}
			if anyNeeded {
				return "Updates available", nil
			}
			return "Already up to date", nil
		}); err != nil {
			return err
		}

		if !anyNeeded {
			fmt.Fprintln(cmd.OutOrStderr())
			return nil
		}

		// Step 2: Update tools.
		if err := ui.Step("Updating tools...", func() (string, error) {
			var results []string
			for _, u := range updates {
				if !u.needed {
					results = append(results, fmt.Sprintf("%s up to date", u.binary.DisplayName))
					continue
				}

				if err := binaries.InstallBinary(client, u.binary, u.latest); err != nil {
					return "", fmt.Errorf("cannot install %s: %w", u.binary.DisplayName, err)
				}

				vs.Set(u.binary.Name, u.latest)
				if err := vs.Save(); err != nil {
					return "", fmt.Errorf("cannot save version state: %w", err)
				}

				if u.current != "" {
					results = append(results, fmt.Sprintf("%s %s → %s", u.binary.DisplayName, u.current, u.latest))
				} else {
					results = append(results, fmt.Sprintf("%s %s", u.binary.DisplayName, u.latest))
				}
			}
			return strings.Join(results, ", "), nil
		}); err != nil {
			return err
		}

		ui.Footer(start, "")

		return nil
	},
}

func init() {
	updateCmd.Flags().BoolVarP(&updateVerbose, "verbose", "v", false, "Show detailed output")
	rootCmd.AddCommand(updateCmd)
}
