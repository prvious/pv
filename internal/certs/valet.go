package certs

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/config"
)

// CertsDir returns ~/.pv/data/certs/.
func CertsDir() string {
	return filepath.Join(config.DataDir(), "certs")
}

// CertPath returns the full path to a site certificate.
func CertPath(hostname string) string {
	return filepath.Join(CertsDir(), hostname+".crt")
}

// KeyPath returns the full path to a site private key.
func KeyPath(hostname string) string {
	return filepath.Join(CertsDir(), hostname+".key")
}

// GenerateSiteTLS generates a TLS cert/key pair for hostname and places them
// in the pv certs directory. Uses Caddy's local CA to sign the certificate.
func GenerateSiteTLS(hostname string) error {
	caCertPath := config.CACertPath()
	caKeyPath := config.CAKeyPath()

	if _, err := os.Stat(caCertPath); err != nil {
		return fmt.Errorf("Caddy CA not found at %s (run pv start first to generate it)", caCertPath)
	}
	if _, err := os.Stat(caKeyPath); err != nil {
		return fmt.Errorf("Caddy CA key not found at %s", caKeyPath)
	}

	certsDir := CertsDir()
	if err := os.MkdirAll(certsDir, 0755); err != nil {
		return fmt.Errorf("cannot create certs dir: %w", err)
	}

	certPath := CertPath(hostname)
	keyPath := KeyPath(hostname)

	return GenerateSiteCert(hostname, caCertPath, caKeyPath, certPath, keyPath)
}

// RemoveSiteTLS removes the TLS cert/key pair for hostname.
// Returns nil if the files do not exist.
func RemoveSiteTLS(hostname string) error {
	var errs []error
	for _, ext := range []string{".crt", ".key"} {
		if err := os.Remove(filepath.Join(CertsDir(), hostname+ext)); err != nil && !os.IsNotExist(err) {
			errs = append(errs, err)
		}
	}
	return errors.Join(errs...)
}

// RemoveLinkedCerts removes TLS cert/key pairs for the given hostnames.
func RemoveLinkedCerts(hostnames []string) error {
	var errs []error
	for _, h := range hostnames {
		for _, ext := range []string{".crt", ".key"} {
			if err := os.Remove(filepath.Join(CertsDir(), h+ext)); err != nil && !os.IsNotExist(err) {
				errs = append(errs, err)
			}
		}
	}
	return errors.Join(errs...)
}
