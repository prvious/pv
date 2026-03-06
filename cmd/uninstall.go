package cmd

import (
	"bufio"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
	"time"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/daemon"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/setup"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var uninstallCmd = &cobra.Command{
	Use:   "uninstall",
	Short: "Completely remove pv and all its data",
	RunE: func(cmd *cobra.Command, args []string) error {
		// Confirmation prompt.
		fmt.Fprintln(os.Stderr)
		fmt.Fprintln(os.Stderr, "This will remove:")
		fmt.Fprintln(os.Stderr, "  - The pv binary")
		fmt.Fprintln(os.Stderr, "  - All PHP versions and FrankenPHP binaries")
		fmt.Fprintln(os.Stderr, "  - All Composer global packages and cache")
		fmt.Fprintln(os.Stderr, "  - All project links (your project files are NOT deleted)")
		fmt.Fprintln(os.Stderr, "  - DNS resolver configuration")
		fmt.Fprintln(os.Stderr, "  - Trusted CA certificate")
		fmt.Fprintln(os.Stderr, "  - Launchd service")
		fmt.Fprintln(os.Stderr)
		fmt.Fprintln(os.Stderr, "Your projects themselves will not be touched.")
		fmt.Fprintln(os.Stderr)
		fmt.Fprint(os.Stderr, "Type \"uninstall\" to confirm: ")

		scanner := bufio.NewScanner(os.Stdin)
		scanner.Scan()
		if strings.TrimSpace(scanner.Text()) != "uninstall" {
			fmt.Fprintln(os.Stderr, "Aborted.")
			return nil
		}
		fmt.Fprintln(os.Stderr)

		// Auth backup offer.
		authPath := filepath.Join(config.ComposerDir(), "auth.json")
		if hasAuthTokens(authPath) {
			fmt.Fprint(os.Stderr, "Back up Composer auth tokens to ~/pv-auth-backup.json? [Y/n] ")
			scanner.Scan()
			answer := strings.TrimSpace(strings.ToLower(scanner.Text()))
			if answer == "" || answer == "y" || answer == "yes" {
				home, _ := os.UserHomeDir()
				backupPath := filepath.Join(home, "pv-auth-backup.json")
				if err := copyFile(authPath, backupPath); err != nil {
					fmt.Fprintf(os.Stderr, "  Warning: could not back up auth tokens: %v\n", err)
				} else {
					ui.Success(fmt.Sprintf("Backed up to %s", backupPath))
				}
			}
			fmt.Fprintln(os.Stderr)
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
		_ = colimaUninstallCmd.RunE(colimaUninstallCmd, nil)
		_ = phpUninstallCmd.RunE(phpUninstallCmd, nil)
		_ = magoUninstallCmd.RunE(magoUninstallCmd, nil)
		_ = composerUninstallCmd.RunE(composerUninstallCmd, nil)

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
			fmt.Fprintf(os.Stderr, "  %s %v\n", ui.Red.Render("!"), err)
		}

		// Remove launchd plist.
		if err := ui.Step("Removing launchd service...", func() (string, error) {
			if err := daemon.Uninstall(); err != nil {
				return "", err
			}
			return "Launchd service removed", nil
		}); err != nil {
			fmt.Fprintf(os.Stderr, "  %s %v\n", ui.Red.Render("!"), err)
		}

		// Remove system configuration (sudo).
		if err := ui.Step("Removing DNS resolver...", func() (string, error) {
			if runSudo(fmt.Sprintf("rm -f /etc/resolver/%s", tld)) {
				return "DNS resolver removed", nil
			}
			return "", fmt.Errorf("could not remove /etc/resolver/%s — run: sudo rm -f /etc/resolver/%s", tld, tld)
		}); err != nil {
			fmt.Fprintf(os.Stderr, "  %s %v\n", ui.Red.Render("!"), err)
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
				fmt.Fprintf(os.Stderr, "  %s %v\n", ui.Red.Render("!"), err)
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
			fmt.Fprintf(os.Stderr, "  %s %v\n", ui.Red.Render("!"), err)
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
			fmt.Fprintf(os.Stderr, "  %s %v\n", ui.Red.Render("!"), err)
		}

		// Report scattered .pv-php files.
		fmt.Fprintln(os.Stderr)
		var found []string
		for _, p := range projectPaths {
			pvPhpPath := filepath.Join(p, ".pv-php")
			if _, err := os.Stat(pvPhpPath); err == nil {
				found = append(found, pvPhpPath)
			}
		}
		if len(found) > 0 {
			fmt.Fprintln(os.Stderr, "Found .pv-php files in your projects:")
			for _, f := range found {
				fmt.Fprintf(os.Stderr, "  %s\n", f)
			}
			fmt.Fprintln(os.Stderr, "You can safely delete these.")
			fmt.Fprintln(os.Stderr)
		}

		// Print manual steps.
		shell := setup.DetectShell()
		configFile := setup.ShellConfigFile(shell)
		exportLine := setup.PathExportLine(shell)

		fmt.Fprintln(os.Stderr, "Done! Just remove the pv lines from your shell config:")
		fmt.Fprintln(os.Stderr)
		fmt.Fprintf(os.Stderr, "  # Remove from %s:\n", configFile)
		fmt.Fprintf(os.Stderr, "  %s\n", exportLine)
		fmt.Fprintln(os.Stderr, "  eval \"$(pv env)\"   # if present")
		fmt.Fprintln(os.Stderr)
		fmt.Fprintln(os.Stderr, "pv has been completely uninstalled. Your projects were not modified.")

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
	rootCmd.AddCommand(uninstallCmd)
}
