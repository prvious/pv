package setup

import (
	"bytes"
	"crypto/x509"
	"encoding/pem"
	"fmt"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"time"

	"github.com/prvious/pv/internal/config"
)

const (
	resolverDir     = "/etc/resolver"
	resolverContent = "nameserver 127.0.0.1\nport 10053\n"
	systemKeychain  = "/Library/Keychains/System.keychain"
	caCommonName    = "Caddy Local Authority"
)

// ResolverSetupScript returns the shell script for creating the DNS resolver file only (no trust).
func ResolverSetupScript(tld string) string {
	resolverFile := filepath.Join(resolverDir, tld)
	return fmt.Sprintf(
		`mkdir -p %s && printf 'nameserver 127.0.0.1\nport 10053\n' > %s && (dscacheutil -flushcache; killall -HUP mDNSResponder 2>/dev/null || true)`,
		resolverDir, resolverFile,
	)
}

// Verbose controls whether sudo commands show output.
var Verbose bool

// RunSudoResolver executes the sudo command for DNS resolver setup only.
func RunSudoResolver(tld string) error {
	script := ResolverSetupScript(tld)
	cmd := exec.Command("sudo", "sh", "-c", script)
	cmd.Stdin = os.Stdin
	if Verbose {
		cmd.Stdout = os.Stdout
		cmd.Stderr = os.Stderr
	}
	return cmd.Run()
}

func caFingerprint(certPath string) ([]byte, error) {
	certPEM, err := os.ReadFile(certPath)
	if err != nil {
		return nil, fmt.Errorf("read CA certificate: %w", err)
	}

	block, _ := pem.Decode(certPEM)
	if block == nil {
		return nil, fmt.Errorf("decode CA certificate: no PEM block found")
	}

	cert, err := x509.ParseCertificate(block.Bytes)
	if err != nil {
		return nil, fmt.Errorf("parse CA certificate: %w", err)
	}

	return cert.Raw, nil
}

func keychainHasCert(certPath string, args ...string) (bool, error) {
	out, err := exec.Command("security", args...).Output()
	if err != nil {
		return false, err
	}

	target, err := caFingerprint(certPath)
	if err != nil {
		return false, err
	}

	rest := out
	for {
		block, remaining := pem.Decode(rest)
		if block == nil {
			break
		}
		if block.Type == "CERTIFICATE" {
			cert, err := x509.ParseCertificate(block.Bytes)
			if err == nil && bytes.Equal(cert.Raw, target) {
				return true, nil
			}
		}
		rest = remaining
	}

	return false, nil
}

// IsCATrusted reports whether the current Caddy local CA certificate is present
// in the System keychain.
func IsCATrusted() (bool, error) {
	certPath := config.CACertPath()
	if _, err := os.Stat(certPath); err != nil {
		if os.IsNotExist(err) {
			return false, nil
		}
		return false, fmt.Errorf("stat CA certificate: %w", err)
	}

	trusted, err := keychainHasCert(certPath, "find-certificate", "-c", caCommonName, "-a", "-p", systemKeychain)
	if err != nil {
		if _, ok := err.(*exec.ExitError); ok {
			return false, nil
		}
		return false, fmt.Errorf("check System keychain: %w", err)
	}

	return trusted, nil
}

// RunSudoTrustCACert trusts the current Caddy local CA certificate using
// FrankenPHP's trust command.
func RunSudoTrustCACert() error {
	frankenphp := filepath.Join(config.BinDir(), "frankenphp")
	pvDir := config.PvDir()
	trust := exec.Command(frankenphp, "trust")
	trust.Env = append(os.Environ(), "XDG_DATA_HOME="+pvDir, "XDG_CONFIG_HOME="+pvDir)
	trust.Stdin = os.Stdin
	if Verbose {
		trust.Stdout = os.Stdout
		trust.Stderr = os.Stderr
	}
	if err := trust.Run(); err != nil {
		return fmt.Errorf("frankenphp trust: %w", err)
	}

	return nil
}

// RunSudoUntrustCACert removes the current Caddy local CA certificate using
// FrankenPHP's untrust command.
func RunSudoUntrustCACert() error {
	certPath := config.CACertPath()
	if _, err := os.Stat(certPath); err != nil {
		return fmt.Errorf("Caddy CA not found at %s: %w", certPath, err)
	}

	frankenphp := filepath.Join(config.BinDir(), "frankenphp")
	pvDir := config.PvDir()
	cmd := exec.Command(frankenphp, "untrust", "--cert", certPath)
	cmd.Env = append(os.Environ(), "XDG_DATA_HOME="+pvDir, "XDG_CONFIG_HOME="+pvDir)
	cmd.Stdin = os.Stdin
	if Verbose {
		cmd.Stdout = os.Stdout
		cmd.Stderr = os.Stderr
	}
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("frankenphp untrust: %w", err)
	}

	return nil
}

// RunSudoTrustWithServer starts FrankenPHP temporarily so the local CA exists,
// then trusts that CA in the System keychain.
func RunSudoTrustWithServer() error {
	frankenphp := filepath.Join(config.BinDir(), "frankenphp")
	caddyfile := config.CaddyfilePath()

	// Start FrankenPHP in the background.
	srv := exec.Command(frankenphp, "run", "--config", caddyfile, "--adapter", "caddyfile")
	srv.Env = append(os.Environ(), config.CaddyEnv()...)
	srv.Stdout = nil
	srv.Stderr = nil
	if err := srv.Start(); err != nil {
		return fmt.Errorf("starting FrankenPHP: %w", err)
	}

	// Ensure we always kill the server when done.
	done := make(chan error, 1)
	go func() { done <- srv.Wait() }()
	defer func() {
		srv.Process.Kill()
		<-done
	}()

	// Poll admin API until ready (up to 5s).
	deadline := time.Now().Add(5 * time.Second)
	ready := false
	for time.Now().Before(deadline) {
		resp, err := http.Get("http://localhost:2019/config/")
		if err == nil {
			resp.Body.Close()
			ready = true
			break
		}
		time.Sleep(200 * time.Millisecond)
	}
	if !ready {
		return fmt.Errorf("FrankenPHP admin API did not become ready within 5s")
	}

	if err := RunSudoTrustCACert(); err != nil {
		return err
	}

	return nil
}

// CheckResolverFile verifies that /etc/resolver/{tld} exists with the correct content.
func CheckResolverFile(tld string) error {
	resolverFile := filepath.Join(resolverDir, tld)
	data, err := os.ReadFile(resolverFile)
	if err != nil {
		return fmt.Errorf("cannot read %s: %w", resolverFile, err)
	}
	if string(data) != resolverContent {
		return fmt.Errorf("unexpected content in %s", resolverFile)
	}
	return nil
}
