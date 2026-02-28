package cmd

import (
	"fmt"
	"net/http"

	"github.com/prvious/pv/internal/binaries"
	"github.com/spf13/cobra"
)

var updateCmd = &cobra.Command{
	Use:   "update",
	Short: "Download and update all managed binaries",
	RunE: func(cmd *cobra.Command, args []string) error {
		client := &http.Client{}

		vs, err := binaries.LoadVersions()
		if err != nil {
			return fmt.Errorf("cannot load version state: %w", err)
		}

		for _, b := range binaries.All() {
			fmt.Printf("Checking %s...\n", b.DisplayName)

			latest, err := binaries.FetchLatestVersion(client, b)
			if err != nil {
				return fmt.Errorf("cannot check %s version: %w", b.DisplayName, err)
			}

			if !binaries.NeedsUpdate(vs, b, latest) {
				fmt.Printf("  %s is already up to date (%s)\n", b.DisplayName, vs.Get(b.Name))
				continue
			}

			// Strip "v" prefix for download URL construction (FrankenPHP adds it back).
			version := latest
			if b.Name == "frankenphp" {
				// FrankenPHP tag format: v1.11.3 — DownloadURL prepends "v"
				// so pass the bare version number.
				if len(version) > 0 && version[0] == 'v' {
					version = version[1:]
				}
			}

			if err := binaries.InstallBinary(client, b, version); err != nil {
				return fmt.Errorf("cannot install %s: %w", b.DisplayName, err)
			}

			vs.Set(b.Name, latest)
			if err := vs.Save(); err != nil {
				return fmt.Errorf("cannot save version state: %w", err)
			}

			fmt.Printf("  %s updated to %s\n", b.DisplayName, latest)
		}

		fmt.Println("Generating shims...")
		if err := binaries.WriteAllShims(); err != nil {
			return fmt.Errorf("cannot write shims: %w", err)
		}

		fmt.Println("Done.")
		return nil
	},
}

func init() {
	rootCmd.AddCommand(updateCmd)
}
