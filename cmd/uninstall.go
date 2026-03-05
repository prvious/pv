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
	"github.com/spf13/cobra"
)

var uninstallCmd = &cobra.Command{
	Use:   "uninstall",
	Short: "Completely remove pv and all its data",
	RunE: func(cmd *cobra.Command, args []string) error {
		// Task 1: Confirmation prompt.
		fmt.Println()
		fmt.Println("This will remove:")
		fmt.Println("  • The pv binary")
		fmt.Println("  • All PHP versions and FrankenPHP binaries")
		fmt.Println("  • All Composer global packages and cache")
		fmt.Println("  • All project links (your project files are NOT deleted)")
		fmt.Println("  • DNS resolver configuration")
		fmt.Println("  • Trusted CA certificate")
		fmt.Println("  • Launchd service")
		fmt.Println()
		fmt.Println("Your projects themselves will not be touched.")
		fmt.Println()
		fmt.Print("Type \"uninstall\" to confirm: ")

		scanner := bufio.NewScanner(os.Stdin)
		scanner.Scan()
		if strings.TrimSpace(scanner.Text()) != "uninstall" {
			fmt.Println("Aborted.")
			return nil
		}
		fmt.Println()

		// Task 2: Auth backup offer.
		authPath := filepath.Join(config.ComposerDir(), "auth.json")
		if hasAuthTokens(authPath) {
			fmt.Print("Back up Composer auth tokens to ~/pv-auth-backup.json? [Y/n] ")
			scanner.Scan()
			answer := strings.TrimSpace(strings.ToLower(scanner.Text()))
			if answer == "" || answer == "y" || answer == "yes" {
				home, _ := os.UserHomeDir()
				backupPath := filepath.Join(home, "pv-auth-backup.json")
				if err := copyFile(authPath, backupPath); err != nil {
					fmt.Printf("  Warning: could not back up auth tokens: %v\n", err)
				} else {
					fmt.Printf("  Backed up to %s\n", backupPath)
				}
			}
			fmt.Println()
		}

		// Task 3: Read registry before deletion.
		var projectPaths []string
		reg, err := registry.Load()
		if err == nil {
			for _, p := range reg.List() {
				projectPaths = append(projectPaths, p.Path)
			}
		}

		// Load settings to know the TLD for resolver cleanup.
		settings, _ := config.LoadSettings()
		tld := settings.TLD

		// Task 4: Stop all services.
		fmt.Println("Stopping services...")
		if daemon.IsLoaded() {
			if err := daemon.Unload(); err != nil {
				fmt.Printf("  Warning: could not unload daemon: %v\n", err)
			}
			// Wait for clean shutdown.
			for i := 0; i < 25; i++ {
				time.Sleep(200 * time.Millisecond)
				if !daemon.IsLoaded() {
					break
				}
			}
		}

		// Also check foreground mode PID.
		if pid, err := server.ReadPID(); err == nil {
			if proc, err := os.FindProcess(pid); err == nil {
				_ = proc.Signal(syscall.SIGTERM)
				// Wait for exit.
				for i := 0; i < 25; i++ {
					time.Sleep(200 * time.Millisecond)
					if proc.Signal(syscall.Signal(0)) != nil {
						break
					}
				}
				// Force kill if still alive.
				if proc.Signal(syscall.Signal(0)) == nil {
					_ = proc.Signal(syscall.SIGKILL)
				}
			}
		}
		fmt.Println("  Done")

		// Task 5: Remove launchd plist.
		fmt.Println("Removing launchd service...")
		if err := daemon.Uninstall(); err != nil {
			fmt.Printf("  Warning: %v\n", err)
		} else {
			fmt.Println("  Done")
		}

		// Task 6: Remove system configuration (sudo).
		fmt.Println("Removing system configuration...")
		fmt.Println("  This requires administrator privileges.")

		// Remove DNS resolver file.
		resolverRemoved := runSudo(fmt.Sprintf("rm -f /etc/resolver/%s", tld))
		if resolverRemoved {
			fmt.Println("  Removed DNS resolver")
		} else {
			fmt.Printf("  Warning: could not remove /etc/resolver/%s. Clean up manually:\n", tld)
			fmt.Printf("    sudo rm -f /etc/resolver/%s\n", tld)
		}

		// Untrust CA certificate (may trigger keychain dialog, so use a timeout).
		caCertPath := config.CACertPath()
		if _, err := os.Stat(caCertPath); err == nil {
			certCmd := exec.Command("sudo", "-n", "security", "remove-trusted-cert", "-d", caCertPath)
			certCmd.Stdout = os.Stdout
			certCmd.Stderr = os.Stderr

			if err := certCmd.Start(); err == nil {
				done := make(chan error, 1)
				go func() { done <- certCmd.Wait() }()
				select {
				case err := <-done:
					if err == nil {
						fmt.Println("  Removed CA certificate")
					} else {
						fmt.Println("  Warning: could not untrust CA certificate. Clean up manually:")
						fmt.Printf("    sudo security remove-trusted-cert -d %s\n", caCertPath)
					}
				case <-time.After(10 * time.Second):
					certCmd.Process.Kill()
					<-done
					fmt.Println("  Warning: CA certificate removal timed out. Clean up manually:")
					fmt.Printf("    sudo security remove-trusted-cert -d %s\n", caCertPath)
				}
			} else {
				fmt.Println("  Warning: could not untrust CA certificate. Clean up manually:")
				fmt.Printf("    sudo security remove-trusted-cert -d %s\n", caCertPath)
			}
		}

		// Task 7: Remove ~/.pv directory.
		fmt.Println("Removing ~/.pv...")
		pvDir := config.PvDir()
		if err := os.RemoveAll(pvDir); err != nil {
			// Some files may be owned by root (e.g. from sudo -E pv start).
			// Fall back to sudo rm -rf.
			if runSudo(fmt.Sprintf("rm -rf '%s'", pvDir)) {
				fmt.Println("  Done")
			} else {
				fmt.Printf("  Warning: could not fully remove %s: %v\n", pvDir, err)
			}
		} else {
			fmt.Println("  Done")
		}

		// Remove the pv binary itself.
		fmt.Println("Removing pv binary...")
		if pvBin, err := os.Executable(); err == nil {
			// Resolve symlinks to get the real path.
			if resolved, err := filepath.EvalSymlinks(pvBin); err == nil {
				pvBin = resolved
			}
			if err := os.Remove(pvBin); err != nil {
				// May need sudo if installed in /usr/local/bin.
				if runSudo(fmt.Sprintf("rm -f '%s'", pvBin)) {
					fmt.Printf("  Removed %s\n", pvBin)
				} else {
					fmt.Printf("  Warning: could not remove %s. Delete it manually.\n", pvBin)
				}
			} else {
				fmt.Printf("  Removed %s\n", pvBin)
			}
		}

		// Task 8: Report scattered .pv-php files.
		fmt.Println()
		var found []string
		for _, p := range projectPaths {
			pvPhpPath := filepath.Join(p, ".pv-php")
			if _, err := os.Stat(pvPhpPath); err == nil {
				found = append(found, pvPhpPath)
			}
		}
		if len(found) > 0 {
			fmt.Println("Found .pv-php files in your projects:")
			for _, f := range found {
				fmt.Printf("  %s\n", f)
			}
			fmt.Println("You can safely delete these.")
			fmt.Println()
		}

		// Task 9: Print manual steps.
		shell := setup.DetectShell()
		configFile := setup.ShellConfigFile(shell)
		exportLine := setup.PathExportLine(shell)

		fmt.Println("Done! Just remove the pv lines from your shell config:")
		fmt.Println()
		fmt.Printf("  # Remove from %s:\n", configFile)
		fmt.Printf("  %s\n", exportLine)
		fmt.Println("  eval \"$(pv env)\"   # if present")
		fmt.Println()
		fmt.Println("pv has been completely uninstalled. Your projects were not modified.")

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
