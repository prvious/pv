package cmd

import (
	"fmt"
	"os"
	"os/exec"
	"strings"

	"github.com/prvious/pv/internal/caddy"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/setup"
	"github.com/prvious/pv/internal/ui"
)

// acquireSudo prompts for sudo credentials upfront so password prompts
// don't interfere with spinner output during DNS/CA steps.
func acquireSudo() error {
	ui.Subtle("pv needs sudo for DNS and certificate setup.")
	sudoCmd := exec.Command("sudo", "-v")
	sudoCmd.Stdin = os.Stdin
	sudoCmd.Stdout = os.Stderr
	sudoCmd.Stderr = os.Stderr
	if err := sudoCmd.Run(); err != nil {
		return fmt.Errorf("sudo authentication failed: %w", err)
	}
	fmt.Fprintln(os.Stderr)
	return nil
}

// bootstrapFinalize runs the post-install finalization steps shared by
// both `pv install` and `pv setup`: Caddyfile, registry, DNS, CA trust, shell PATH.
func bootstrapFinalize(tld string) error {
	// Generate Caddyfile.
	if err := ui.Step("Configuring environment...", func() (string, error) {
		if err := caddy.GenerateCaddyfile(); err != nil {
			return "", fmt.Errorf("cannot generate Caddyfile: %w", err)
		}

		// Create empty registry if it doesn't exist.
		if _, err := os.Stat(config.RegistryPath()); os.IsNotExist(err) {
			reg := &registry.Registry{}
			if err := reg.Save(); err != nil {
				return "", fmt.Errorf("cannot save registry: %w", err)
			}
		}

		return "Environment configured", nil
	}); err != nil {
		return err
	}

	// DNS resolver (sudo).
	if err := ui.Step("Setting up DNS resolver...", func() (string, error) {
		if err := setup.RunSudoResolver(tld); err != nil {
			return "", fmt.Errorf("DNS resolver setup failed: %w", err)
		}
		return "DNS resolver configured", nil
	}); err != nil {
		return err
	}

	// Trust CA certificate (sudo).
	if err := ui.Step("Trusting HTTPS certificate...", func() (string, error) {
		if err := setup.RunSudoTrustWithServer(); err != nil {
			return "", fmt.Errorf("CA trust failed: %w", err)
		}
		return "HTTPS certificate trusted", nil
	}); err != nil {
		return err
	}

	// Shell PATH.
	if err := ui.Step("Configuring shell...", func() (string, error) {
		shell := setup.DetectShell()
		configFile := setup.ShellConfigFile(shell)
		line := setup.PathExportLine(shell)

		data, err := os.ReadFile(configFile)
		if err == nil && strings.Contains(string(data), line) {
			return "PATH already configured", nil
		}

		f, err := os.OpenFile(configFile, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0644)
		if err != nil {
			return "", fmt.Errorf("cannot open %s: %w", configFile, err)
		}
		defer f.Close()
		if _, err := fmt.Fprintf(f, "\n# pv\n%s\n", line); err != nil {
			return "", fmt.Errorf("cannot write to %s: %w", configFile, err)
		}

		return fmt.Sprintf("Added to ~/%s", shortPath(configFile)), nil
	}); err != nil {
		return err
	}

	return nil
}
