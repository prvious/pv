package cmd

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"syscall"
	"time"

	"charm.land/huh/v2"
	"github.com/prvious/pv/internal/certs"
	colimacmd "github.com/prvious/pv/internal/commands/colima"
	"github.com/prvious/pv/internal/commands/composer"
	"github.com/prvious/pv/internal/commands/mago"
	"github.com/prvious/pv/internal/commands/php"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/daemon"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/setup"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var forceUninstall bool

var uninstallCmd = &cobra.Command{
	Use:     "uninstall",
	GroupID: "core",
	Short:   "Completely remove pv and all its data",
	RunE: func(cmd *cobra.Command, args []string) error {
		start := time.Now()

		// Confirmation prompt.
		ui.Subtle("This will remove:")
		ui.Subtle("- The pv binary")
		ui.Subtle("- All PHP versions and FrankenPHP binaries")
		ui.Subtle("- All Composer global packages and cache")
		ui.Subtle("- All project links (your project files are NOT deleted)")
		ui.Subtle("- DNS resolver configuration")
		ui.Subtle("- Trusted CA certificate")
		ui.Subtle("- Launchd service")
		ui.Subtle("")
		ui.Subtle("Your projects themselves will not be touched.")

		if !forceUninstall {
			var confirmation string
			if err := huh.NewInput().
				Title("Type \"uninstall\" to confirm").
				Value(&confirmation).
				Run(); err != nil {
				return err
			}
			if confirmation != "uninstall" {
				ui.Subtle("Aborted.")
				return nil
			}
		}

		// Auth backup offer.
		authPath := filepath.Join(config.ComposerDir(), "auth.json")
		if !forceUninstall && hasAuthTokens(authPath) {
			backupAuth := true
			if err := huh.NewConfirm().
				Title("Back up Composer auth tokens to ~/pv-auth-backup.json?").
				Affirmative("Yes").
				Negative("No").
				Value(&backupAuth).
				Run(); err != nil {
				return err
			}
			if backupAuth {
				home, _ := os.UserHomeDir()
				backupPath := filepath.Join(home, "pv-auth-backup.json")
				if err := copyFile(authPath, backupPath); err != nil {
					ui.Fail(fmt.Sprintf("Could not back up auth tokens: %v", err))
				} else {
					ui.Success(fmt.Sprintf("Backed up to %s", backupPath))
				}
			}
		}

		fmt.Fprintln(os.Stderr)
		ui.Header(version)

		var projectPaths []string
		tld := "test"
		var reg *registry.Registry

		if err := ui.Step("Preparing uninstall...", func() (string, error) {
			loadedReg, err := registry.Load()
			if err == nil {
				reg = loadedReg
				for _, p := range reg.List() {
					projectPaths = append(projectPaths, p.Path)
				}
			}

			settings, _ := config.LoadSettings()
			if settings != nil {
				tld = settings.Defaults.TLD
			}

			return fmt.Sprintf("Using .%s domain", tld), nil
		}); err != nil {
			return err
		}

		// Uninstall tools (each cleans up its own binary + PATH entry).
		if err := colimacmd.RunUninstall(); err != nil {
			if !errors.Is(err, ui.ErrAlreadyPrinted) {
				ui.Fail(fmt.Sprintf("Colima uninstall failed: %v", err))
			}
		}
		if err := php.RunUninstall(); err != nil {
			if !errors.Is(err, ui.ErrAlreadyPrinted) {
				ui.Fail(fmt.Sprintf("PHP uninstall failed: %v", err))
			}
		}
		if err := mago.RunUninstall(); err != nil {
			if !errors.Is(err, ui.ErrAlreadyPrinted) {
				ui.Fail(fmt.Sprintf("Mago uninstall failed: %v", err))
			}
		}
		if err := composer.RunUninstall(); err != nil {
			if !errors.Is(err, ui.ErrAlreadyPrinted) {
				ui.Fail(fmt.Sprintf("Composer uninstall failed: %v", err))
			}
		}

		// Stop services.
		if err := ui.Step("Stopping services...", func() (string, error) {
			if daemon.IsLoaded() {
				if err := daemon.Unload(); err != nil {
					return "", fmt.Errorf("could not unload daemon: %w", err)
				}
				for i := 0; i < 25; i++ {
					time.Sleep(200 * time.Millisecond)
					if !daemon.IsLoaded() {
						break
					}
				}
			}

			if pid, err := server.ReadPID(); err == nil {
				if proc, err := os.FindProcess(pid); err == nil {
					_ = proc.Signal(syscall.SIGTERM)
					for i := 0; i < 25; i++ {
						time.Sleep(200 * time.Millisecond)
						if proc.Signal(syscall.Signal(0)) != nil {
							break
						}
					}
					if proc.Signal(syscall.Signal(0)) == nil {
						_ = proc.Signal(syscall.SIGKILL)
					}
				}
			}

			return "Services stopped", nil
		}); err != nil {
			// Error already displayed by ui.Step
		}

		// Remove launchd plist.
		if err := ui.Step("Removing launchd service...", func() (string, error) {
			if err := daemon.Uninstall(); err != nil {
				return "", err
			}
			return "Launchd service removed", nil
		}); err != nil {
			// Error already displayed by ui.Step
		}

		resolverFile := filepath.Join("/etc/resolver", tld)
		caCertPath := config.CACertPath()
		if err := ui.Step("Checking system cleanup requirements...", func() (string, error) {
			needSudo := false
			if _, err := os.Stat(resolverFile); err == nil {
				needSudo = true
			}
			if _, err := os.Stat(caCertPath); err == nil {
				needSudo = true
			}

			if needSudo {
				if err := acquireSudo(); err != nil {
					return "", err
				}
				return "Sudo ready for system cleanup", nil
			}

			return "No sudo cleanup needed", nil
		}); err != nil {
			return err
		}

		// Remove system configuration (sudo).
		if err := ui.Step("Removing DNS resolver...", func() (string, error) {
			if runSudo("rm", "-f", resolverFile) {
				return "DNS resolver removed", nil
			}
			return "", fmt.Errorf("could not remove %s — run: sudo rm -f %s", resolverFile, resolverFile)
		}); err != nil {
			// Error already displayed by ui.Step
		}

		// Untrust CA certificate.
		if _, err := os.Stat(caCertPath); err == nil {
			if err := ui.Step("Removing CA certificate...", func() (string, error) {
				if err := setup.RunSudoUntrustCACert(); err != nil {
					return "", fmt.Errorf("could not untrust CA — run: sudo security remove-trusted-cert -d %s", caCertPath)
				}
				return "CA certificate removed", nil
			}); err != nil {
				// Error already displayed by ui.Step
			}
		}

		// Remove Vite TLS certs for linked projects only.
		if reg != nil {
			var hostnames []string
			for _, p := range reg.List() {
				hostnames = append(hostnames, p.Name+"."+tld)
			}
			if err := certs.RemoveLinkedCerts(hostnames); err != nil {
				ui.Subtle(fmt.Sprintf("Could not remove some Vite TLS certs: %v", err))
			}
		}
		if err := certs.RemoveConfig(); err != nil {
			ui.Subtle(fmt.Sprintf("Could not remove Valet config: %v", err))
		}

		// Remove ~/.pv directory.
		if err := ui.Step("Removing ~/.pv...", func() (string, error) {
			pvDir := config.PvDir()
			if err := os.RemoveAll(pvDir); err != nil {
				if runSudo("rm", "-rf", pvDir) {
					return "~/.pv removed", nil
				}
				return "", fmt.Errorf("could not fully remove %s", pvDir)
			}
			return "~/.pv removed", nil
		}); err != nil {
			// Error already displayed by ui.Step
		}

		// Remove the pv binary itself.
		if err := ui.Step("Removing pv binary...", func() (string, error) {
			pvBin, err := os.Executable()
			if err != nil {
				return "", err
			}
			if resolved, err := filepath.EvalSymlinks(pvBin); err == nil {
				pvBin = resolved
			}
			if err := os.Remove(pvBin); err != nil {
				if runSudo("rm", "-f", pvBin) {
					return fmt.Sprintf("Removed %s", pvBin), nil
				}
				return "", fmt.Errorf("could not remove %s — delete it manually", pvBin)
			}
			return fmt.Sprintf("Removed %s", pvBin), nil
		}); err != nil {
			// Error already displayed by ui.Step
		}

		// Report scattered pv.yml files.
		var found []string
		for _, p := range projectPaths {
			pvYmlPath := filepath.Join(p, "pv.yml")
			if _, err := os.Stat(pvYmlPath); err == nil {
				found = append(found, pvYmlPath)
			}
		}
		if len(found) > 0 {
			ui.Subtle("Found pv.yml files in your projects:")
			for _, f := range found {
				ui.Subtle(fmt.Sprintf("  %s", f))
			}
			ui.Subtle("You can safely delete these.")
		}

		// Print manual steps.
		shell := setup.DetectShell()
		configFile := setup.ShellConfigFile(shell)
		exportLine := setup.PathExportLine(shell)

		ui.Subtle("Remove the pv lines from your shell config:")
		ui.Subtle(fmt.Sprintf("  # Remove from %s:", configFile))
		ui.Subtle(fmt.Sprintf("  %s", exportLine))
		ui.Subtle("  eval \"$(pv env)\"   # if present")

		ui.Success("pv has been completely uninstalled. Your projects were not modified.")
		ui.Footer(start, "https://pv.prvious.dev/docs")

		return nil
	},
}

// hasAuthTokens checks if the auth.json file exists and contains any tokens.
func hasAuthTokens(path string) bool {
	data, err := os.ReadFile(path)
	if err != nil {
		return false
	}
	var auth map[string]any
	if err := json.Unmarshal(data, &auth); err != nil {
		return false
	}
	return len(auth) > 0
}

// copyFile copies a file from src to dst.
func copyFile(src, dst string) error {
	data, err := os.ReadFile(src)
	if err != nil {
		return err
	}
	return os.WriteFile(dst, data, 0600)
}

// runSudo runs a command via sudo -n (non-interactive). Returns true on success.
func runSudo(args ...string) bool {
	cmdArgs := append([]string{"-n"}, args...)
	cmd := exec.Command("sudo", cmdArgs...)
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run() == nil
}

func init() {
	uninstallCmd.Flags().BoolVarP(&forceUninstall, "force", "f", false, "Skip confirmation prompt")
	rootCmd.AddCommand(uninstallCmd)
}
