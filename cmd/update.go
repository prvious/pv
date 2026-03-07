package cmd

import (
	"fmt"
	"net/http"
	"os"
	"strings"
	"syscall"
	"time"

	"github.com/prvious/pv/internal/colima"
	"github.com/prvious/pv/internal/selfupdate"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var (
	updateVerbose    bool
	noSelfUpdate     bool
)

var updateCmd = &cobra.Command{
	Use:   "update",
	Short: "Update pv and all managed tools to their latest versions",
	RunE: func(cmd *cobra.Command, args []string) error {
		start := time.Now()

		ui.Header(version)

		client := &http.Client{}

		// Step 1: Self-update pv binary (unless --no-self-update).
		if !noSelfUpdate {
			reexeced, err := selfUpdate(client)
			if err != nil {
				fmt.Fprintf(os.Stderr, "  %s pv self-update failed: %v\n", ui.Red.Render("!"), err)
			}
			if reexeced {
				return nil // reached only if syscall.Exec failed (error already printed)
			}
		}

		// Step 2: Update tools.
		var failures []string

		if err := phpUpdateCmd.RunE(phpUpdateCmd, nil); err != nil {
			fmt.Fprintf(os.Stderr, "  %s PHP update failed: %v\n", ui.Red.Render("!"), err)
			failures = append(failures, "PHP")
		}

		if err := magoUpdateCmd.RunE(magoUpdateCmd, nil); err != nil {
			fmt.Fprintf(os.Stderr, "  %s Mago update failed: %v\n", ui.Red.Render("!"), err)
			failures = append(failures, "Mago")
		}

		if err := composerUpdateCmd.RunE(composerUpdateCmd, nil); err != nil {
			fmt.Fprintf(os.Stderr, "  %s Composer update failed: %v\n", ui.Red.Render("!"), err)
			failures = append(failures, "Composer")
		}

		if colima.IsInstalled() {
			if err := colimaUpdateCmd.RunE(colimaUpdateCmd, nil); err != nil {
				fmt.Fprintf(os.Stderr, "  %s Colima update failed: %v\n", ui.Red.Render("!"), err)
				failures = append(failures, "Colima")
			}
		}

		ui.Footer(start, "")

		if len(failures) > 0 {
			return fmt.Errorf("some updates failed: %s", strings.Join(failures, ", "))
		}

		return nil
	},
}

// selfUpdate checks for a new pv version, downloads it, and re-execs.
// Returns true if the process was re-execed (caller should return immediately).
func selfUpdate(client *http.Client) (bool, error) {
	latest, needed, err := selfupdate.NeedsUpdate(client, version)
	if err != nil {
		return false, err
	}

	if !needed {
		ui.Success("pv already up to date")
		return false, nil
	}

	var newBinary string
	if err := ui.StepProgress("Updating pv...", func(progress func(written, total int64)) (string, error) {
		path, err := selfupdate.Update(client, latest, progress)
		if err != nil {
			return "", err
		}
		newBinary = path
		return fmt.Sprintf("pv %s -> %s", version, latest), nil
	}); err != nil {
		return false, err
	}

	// Re-exec the new binary with --no-self-update to continue with tool updates.
	newArgs := []string{"pv", "update", "--no-self-update"}
	if updateVerbose {
		newArgs = append(newArgs, "--verbose")
	}

	return true, syscall.Exec(newBinary, newArgs, os.Environ())
}

func init() {
	updateCmd.Flags().BoolVarP(&updateVerbose, "verbose", "v", false, "Show detailed output")
	updateCmd.Flags().BoolVar(&noSelfUpdate, "no-self-update", false, "Skip updating the pv binary itself")
	rootCmd.AddCommand(updateCmd)
}
