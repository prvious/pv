package cmd

import (
	"errors"
	"fmt"
	"net/http"
	"os"
	"strings"
	"syscall"
	"time"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/commands/composer"
	"github.com/prvious/pv/internal/commands/mago"
	mysqlCmds "github.com/prvious/pv/internal/commands/mysql"
	"github.com/prvious/pv/internal/commands/php"
	postgresCmds "github.com/prvious/pv/internal/commands/postgres"
	rediscmd "github.com/prvious/pv/internal/commands/redis"
	"github.com/prvious/pv/internal/mailpit"
	my "github.com/prvious/pv/internal/mysql"
	"github.com/prvious/pv/internal/packages"
	pg "github.com/prvious/pv/internal/postgres"
	r "github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/rustfs"
	"github.com/prvious/pv/internal/selfupdate"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var noSelfUpdate bool

var updateCmd = &cobra.Command{
	Use:     "update",
	GroupID: "core",
	Short:   "Update pv and all managed tools to their latest versions",
	RunE: func(cmd *cobra.Command, args []string) error {
		start := time.Now()

		ui.Header(version)

		client := &http.Client{}

		// Step 1: Self-update pv binary (unless --no-self-update).
		if !noSelfUpdate {
			reexeced, err := selfUpdate(client)
			if err != nil {
				if !errors.Is(err, ui.ErrAlreadyPrinted) {
					ui.Fail(fmt.Sprintf("pv self-update failed: %v", err))
				}
				return ui.ErrAlreadyPrinted
			}
			if reexeced {
				return nil // unreachable — syscall.Exec replaced the process
			}
		}

		// Step 2: Update tools.
		var failures []string

		if err := php.RunUpdate(); err != nil {
			if !errors.Is(err, ui.ErrAlreadyPrinted) {
				ui.Fail(fmt.Sprintf("PHP update failed: %v", err))
			}
			failures = append(failures, "PHP")
		}

		if err := mago.RunUpdate(); err != nil {
			if !errors.Is(err, ui.ErrAlreadyPrinted) {
				ui.Fail(fmt.Sprintf("Mago update failed: %v", err))
			}
			failures = append(failures, "Mago")
		}

		if err := composer.RunUpdate(); err != nil {
			if !errors.Is(err, ui.ErrAlreadyPrinted) {
				ui.Fail(fmt.Sprintf("Composer update failed: %v", err))
			}
			failures = append(failures, "Composer")
		}

		// Update each installed postgres major. Each fetches the latest rolling
		// artifact and atomically swaps in the new binary tree. Failures are
		// surfaced via ui.Fail and counted in the failures list — same shape
		// as PHP / Mago / Composer above.
		if majors, err := pg.InstalledMajors(); err == nil {
			for _, major := range majors {
				if err := postgresCmds.RunUpdate([]string{major}); err != nil {
					if !errors.Is(err, ui.ErrAlreadyPrinted) {
						ui.Fail(fmt.Sprintf("PostgreSQL %s update failed: %v", major, err))
					}
					failures = append(failures, "PostgreSQL "+major)
				}
			}
		}

		// Update each installed mysql version. Mirrors the postgres pass — fetches
		// the rolling artifact and atomic-replaces the binary tree per version.
		if versions, err := my.InstalledVersions(); err == nil {
			for _, version := range versions {
				if err := mysqlCmds.RunUpdate([]string{version}); err != nil {
					if !errors.Is(err, ui.ErrAlreadyPrinted) {
						ui.Fail(fmt.Sprintf("MySQL %s update failed: %v", version, err))
					}
					failures = append(failures, "MySQL "+version)
				}
			}
		}

		// Update redis (single-version). Skip if not installed — redis is
		// opt-in via `pv redis:install`.
		if r.IsInstalled() {
			if err := rediscmd.RunUpdate(nil); err != nil {
				if !errors.Is(err, ui.ErrAlreadyPrinted) {
					ui.Fail(fmt.Sprintf("Redis update failed: %v", err))
				}
				failures = append(failures, "Redis")
			}
		}

		// Step 3: Update managed packages.
		for _, pkg := range packages.Managed {
			if err := ui.Step(fmt.Sprintf("Updating %s...", pkg.Name), func() (string, error) {
				updated, version, err := packages.Update(cmd.Context(), client, pkg)
				if err != nil {
					return "", err
				}
				if !updated {
					return fmt.Sprintf("%s already up to date", pkg.Name), nil
				}
				return fmt.Sprintf("%s updated to %s", pkg.Name, version), nil
			}); err != nil {
				if !errors.Is(err, ui.ErrAlreadyPrinted) {
					ui.Fail(fmt.Sprintf("%s update failed: %v", pkg.Name, err))
				}
				failures = append(failures, pkg.Name)
			}
		}

		// Step 4: Update binary-service binaries.
		// registry.Load / LoadVersions return nil on a non-IsNotExist error
		// (corrupt file, permissions), so both pointers must be checked
		// before we dereference them.
		reg, regErr := registry.Load()
		if regErr != nil {
			ui.Subtle(fmt.Sprintf("Skipping binary-service updates: cannot load registry: %v", regErr))
		} else {
			vs, vsErr := binaries.LoadVersions()
			if vsErr != nil {
				ui.Subtle(fmt.Sprintf("Skipping binary-service updates: cannot load versions state: %v", vsErr))
			} else {
				type binaryAddonInfo struct {
					regKey string
					bin    binaries.Binary
					label  string
				}
				addons := []binaryAddonInfo{
					{regKey: "mail", bin: mailpit.Binary(), label: mailpit.DisplayName()},
					{regKey: "s3", bin: rustfs.Binary(), label: rustfs.DisplayName()},
				}

				var binaryUpdated []string
				for _, a := range addons {
					if _, registered := reg.Services[a.regKey]; !registered {
						continue
					}
					latest, err := binaries.FetchLatestVersion(client, a.bin)
					if err != nil {
						ui.Subtle(fmt.Sprintf("Skipping %s: %v", a.label, err))
						continue
					}
					if !binaries.NeedsUpdate(vs, a.bin, latest) {
						continue
					}
					current := vs.Get(a.bin.Name)
					if err := ui.Step(fmt.Sprintf("Updating %s %s -> %s", a.bin.DisplayName, current, latest), func() (string, error) {
						if err := binaries.InstallBinary(client, a.bin, latest); err != nil {
							return "", err
						}
						return fmt.Sprintf("Updated %s to %s", a.bin.DisplayName, latest), nil
					}); err != nil {
						ui.Subtle(fmt.Sprintf("Could not update %s: %v", a.label, err))
						continue
					}
					vs.Set(a.bin.Name, latest)
					binaryUpdated = append(binaryUpdated, a.regKey)
				}
				if len(binaryUpdated) > 0 {
					if err := vs.Save(); err != nil {
						ui.Subtle(fmt.Sprintf("Could not save versions state: %v", err))
					}
					if server.IsRunning() {
						ui.Subtle(fmt.Sprintf("Updated binaries: %s. Run `pv restart` to load them.",
							strings.Join(binaryUpdated, ", ")))
					}
				}
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

	return true, syscall.Exec(newBinary, newArgs, os.Environ())
}

func init() {
	updateCmd.Flags().BoolVar(&noSelfUpdate, "no-self-update", false, "Skip updating the pv binary itself")
	rootCmd.AddCommand(updateCmd)
}
