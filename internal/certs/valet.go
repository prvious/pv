package certs

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/config"
)

// ValetConfigDir returns ~/.config/valet (where laravel-vite-plugin looks for
// Valet on macOS). pv populates this so Vite auto-detects TLS certificates.
func ValetConfigDir() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("cannot determine home directory: %w", err)
	}
	return filepath.Join(home, ".config", "valet"), nil
}

// ValetCertsDir returns ~/.config/valet/Certificates.
func ValetCertsDir() (string, error) {
	dir, err := ValetConfigDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(dir, "Certificates"), nil
}

// EnsureValetConfig writes the TLD to ~/.config/valet/config.json for
// laravel-vite-plugin's TLS certificate discovery. Merges with any existing
// config.json to avoid destroying real Valet settings.
func EnsureValetConfig(tld string) error {
	dir, err := ValetConfigDir()
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Join(dir, "Certificates"), 0755); err != nil {
		return fmt.Errorf("cannot create valet config dir: %w", err)
	}

	configPath := filepath.Join(dir, "config.json")

	// Read existing config to preserve other fields (e.g., real Valet settings).
	cfg := make(map[string]any)
	if data, err := os.ReadFile(configPath); err == nil {
		_ = json.Unmarshal(data, &cfg) // ignore parse errors, overwrite corrupt file
	}

	cfg["tld"] = tld

	data, err := json.MarshalIndent(cfg, "", "  ")
	if err != nil {
		return fmt.Errorf("cannot marshal valet config: %w", err)
	}
	if err := os.WriteFile(configPath, data, 0644); err != nil {
		return fmt.Errorf("cannot write valet config: %w", err)
	}
	return nil
}

// GenerateSiteTLS generates a TLS cert/key pair for hostname and places them
// in the Valet Certificates directory where laravel-vite-plugin expects them.
// Uses Caddy's local CA to sign the certificate.
func GenerateSiteTLS(hostname string) error {
	caCertPath := config.CACertPath()
	caKeyPath := config.CAKeyPath()

	if _, err := os.Stat(caCertPath); err != nil {
		return fmt.Errorf("Caddy CA not found at %s (run pv start first to generate it)", caCertPath)
	}
	if _, err := os.Stat(caKeyPath); err != nil {
		return fmt.Errorf("Caddy CA key not found at %s", caKeyPath)
	}

	certsDir, err := ValetCertsDir()
	if err != nil {
		return err
	}
	if err := os.MkdirAll(certsDir, 0755); err != nil {
		return fmt.Errorf("cannot create certificates dir: %w", err)
	}

	certPath := filepath.Join(certsDir, hostname+".crt")
	keyPath := filepath.Join(certsDir, hostname+".key")

	return GenerateSiteCert(hostname, caCertPath, caKeyPath, certPath, keyPath)
}

// RemoveSiteTLS removes the TLS cert/key pair for hostname from the Valet
// Certificates directory. Returns nil if the files do not exist.
func RemoveSiteTLS(hostname string) error {
	certsDir, err := ValetCertsDir()
	if err != nil {
		return err
	}

	var errs []error
	for _, ext := range []string{".crt", ".key"} {
		if err := os.Remove(filepath.Join(certsDir, hostname+ext)); err != nil && !os.IsNotExist(err) {
			errs = append(errs, err)
		}
	}
	return errors.Join(errs...)
}

// RemoveLinkedCerts removes TLS cert/key pairs for the given hostnames only.
// Does not touch any other files in ~/.config/valet.
func RemoveLinkedCerts(hostnames []string) error {
	certsDir, err := ValetCertsDir()
	if err != nil {
		return err
	}

	var errs []error
	for _, h := range hostnames {
		for _, ext := range []string{".crt", ".key"} {
			if err := os.Remove(filepath.Join(certsDir, h+ext)); err != nil && !os.IsNotExist(err) {
				errs = append(errs, err)
			}
		}
	}
	return errors.Join(errs...)
}

// RemoveConfig removes the config.json file from ~/.config/valet.
// Does not remove the directory itself or any other files.
func RemoveConfig() error {
	dir, err := ValetConfigDir()
	if err != nil {
		return err
	}
	if err := os.Remove(filepath.Join(dir, "config.json")); err != nil && !os.IsNotExist(err) {
		return err
	}
	return nil
}
