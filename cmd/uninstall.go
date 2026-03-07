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
	Short: "Completely remove pv and all its data",
	RunE: func(cmd *cobra.Command, args []string) error {
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

		// Read registry before deletion (for .pv-php file scan later).
		var projectPaths []string
		reg, err := registry.Load()
		if err == nil {
			for _, p := range reg.List() {
				projectPaths = append(projectPaths, p.Path)
			}
		}

		settings, _ := config.LoadSettings()
		tld := settings.TLD

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

		// Remove system configuration (sudo).
		if err := ui.Step("Removing DNS resolver...", func() (string, error) {
			if runSudo(fmt.Sprintf("rm -f /etc/resolver/%s", tld)) {
				return "DNS resolver removed", nil
			}
			return "", fmt.Errorf("could not remove /etc/resolver/%s — run: sudo rm -f /etc/resolver/%s", tld, tld)
		}); err != nil {
			// Error already displayed by ui.Step
		}

		// Untrust CA certificate.
		caCertPath := config.CACertPath()
		if _, err := os.Stat(caCertPath); err == nil {
			if err := ui.Step("Removing CA certificate...", func() (string, error) {
				certCmd := exec.Command("sudo", "-n", "security", "remove-trusted-cert", "-d", caCertPath)
				certCmd.Stdout = os.Stdout
				certCmd.Stderr = os.Stderr

				if err := certCmd.Start(); err != nil {
					return "", fmt.Errorf("could not untrust CA — run: sudo security remove-trusted-cert -d %s", caCertPath)
				}

				done := make(chan error, 1)
				go func() { done <- certCmd.Wait() }()
				select {
				case err := <-done:
					if err != nil {
						return "", fmt.Errorf("could not untrust CA — run: sudo security remove-trusted-cert -d %s", caCertPath)
					}
					return "CA certificate removed", nil
				case <-time.After(10 * time.Second):
					certCmd.Process.Kill()
					<-done
					return "", fmt.Errorf("CA removal timed out — run: sudo security remove-trusted-cert -d %s", caCertPath)
				}
			}); err != nil {
				// Error already displayed by ui.Step
			}
		}

		// Remove ~/.pv directory.
		if err := ui.Step("Removing ~/.pv...", func() (string, error) {
			pvDir := config.PvDir()
			if err := os.RemoveAll(pvDir); err != nil {
				if runSudo(fmt.Sprintf("rm -rf '%s'", pvDir)) {
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
				if runSudo(fmt.Sprintf("rm -f '%s'", pvBin)) {
					return fmt.Sprintf("Removed %s", pvBin), nil
				}
				return "", fmt.Errorf("could not remove %s — delete it manually", pvBin)
			}
			return fmt.Sprintf("Removed %s", pvBin), nil
		}); err != nil {
			// Error already displayed by ui.Step
		}

		// Report scattered .pv-php files.
		var found []string
		for _, p := range projectPaths {
			pvPhpPath := filepath.Join(p, ".pv-php")
			if _, err := os.Stat(pvPhpPath); err == nil {
				found = append(found, pvPhpPath)
			}
		}
		if len(found) > 0 {
			ui.Subtle("Found .pv-php files in your projects:")
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
	return os.WriteFile(dst, data, 0644)
}

// runSudo runs a command via sudo -n (non-interactive). Returns true on success.
func runSudo(script string) bool {
	cmd := exec.Command("sudo", "-n", "sh", "-c", script)
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run() == nil
}

func init() {
	uninstallCmd.Flags().BoolVarP(&forceUninstall, "force", "f", false, "Skip confirmation prompt")
	rootCmd.AddCommand(uninstallCmd)
}
