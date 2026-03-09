package certs

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/config"
)

// valetConfig mirrors the config.json structure the laravel-vite-plugin reads.
type valetConfig struct {
	TLD string `json:"tld"`
}

// ValetConfigDir returns ~/.config/valet (where laravel-vite-plugin looks for
// Valet on macOS). pv populates this so Vite auto-detects TLS certificates.
func ValetConfigDir() string {
	home, _ := os.UserHomeDir()
	return filepath.Join(home, ".config", "valet")
}

// ValetCertsDir returns ~/.config/valet/Certificates.
func ValetCertsDir() string {
	return filepath.Join(ValetConfigDir(), "Certificates")
}

// EnsureValetConfig writes ~/.config/valet/config.json with the current TLD.
// Creates the directory tree if needed.
func EnsureValetConfig(tld string) error {
	dir := ValetConfigDir()
	if err := os.MkdirAll(filepath.Join(dir, "Certificates"), 0755); err != nil {
		return fmt.Errorf("cannot create valet config dir: %w", err)
	}

	cfg := valetConfig{TLD: tld}
	data, err := json.MarshalIndent(cfg, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(dir, "config.json"), data, 0644)
}

// GenerateSiteTLS generates a TLS cert/key pair for hostname and places them
// in the Valet Certificates directory where laravel-vite-plugin expects them.
// Uses Caddy's local CA to sign the certificate.
func GenerateSiteTLS(hostname string) error {
	caCertPath := config.CACertPath()
	caKeyPath := caKeyPath()

	if _, err := os.Stat(caCertPath); err != nil {
		return fmt.Errorf("Caddy CA not found at %s (run pv start first to generate it)", caCertPath)
	}
	if _, err := os.Stat(caKeyPath); err != nil {
		return fmt.Errorf("Caddy CA key not found at %s", caKeyPath)
	}

	certsDir := ValetCertsDir()
	if err := os.MkdirAll(certsDir, 0755); err != nil {
		return fmt.Errorf("cannot create certificates dir: %w", err)
	}

	certPath := filepath.Join(certsDir, hostname+".crt")
	keyPath := filepath.Join(certsDir, hostname+".key")

	return GenerateSiteCert(hostname, caCertPath, caKeyPath, certPath, keyPath)
}

// RemoveSiteTLS removes the TLS cert/key pair for hostname from the Valet
// Certificates directory.
func RemoveSiteTLS(hostname string) {
	certsDir := ValetCertsDir()
	os.Remove(filepath.Join(certsDir, hostname+".crt"))
	os.Remove(filepath.Join(certsDir, hostname+".key"))
}

// RemoveAll removes the entire ~/.config/valet directory created by pv.
func RemoveAll() error {
	return os.RemoveAll(ValetConfigDir())
}

func caKeyPath() string {
	return filepath.Join(config.PvDir(), "caddy", "pki", "authorities", "local", "root.key")
}
