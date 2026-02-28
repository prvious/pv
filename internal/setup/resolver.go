package setup

import (
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
)

// SudoSetupScript returns the shell script for the combined sudo operations:
// creating the DNS resolver file and trusting the Caddy CA certificate.
func SudoSetupScript(tld string) string {
	resolverFile := filepath.Join(resolverDir, tld)
	frankenphp := filepath.Join(config.BinDir(), "frankenphp")
	pvDir := config.PvDir()
	return fmt.Sprintf(
		`mkdir -p %s && printf 'nameserver 127.0.0.1\nport 10053\n' > %s && XDG_DATA_HOME="%s" XDG_CONFIG_HOME="%s" "%s" trust`,
		resolverDir, resolverFile, pvDir, pvDir, frankenphp,
	)
}

// RunSudoSetup executes the combined sudo command for DNS resolver and CA trust.
// It connects stdin/stdout/stderr so the user can enter their password.
func RunSudoSetup(tld string) error {
	script := SudoSetupScript(tld)
	cmd := exec.Command("sudo", "sh", "-c", script)
	cmd.Stdin = os.Stdin
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run()
}

// ResolverSetupScript returns the shell script for creating the DNS resolver file only (no trust).
func ResolverSetupScript(tld string) string {
	resolverFile := filepath.Join(resolverDir, tld)
	return fmt.Sprintf(
		`mkdir -p %s && printf 'nameserver 127.0.0.1\nport 10053\n' > %s`,
		resolverDir, resolverFile,
	)
}

// RunSudoResolver executes the sudo command for DNS resolver setup only.
func RunSudoResolver(tld string) error {
	script := ResolverSetupScript(tld)
	cmd := exec.Command("sudo", "sh", "-c", script)
	cmd.Stdin = os.Stdin
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run()
}

// RunSudoTrustWithServer starts FrankenPHP temporarily so the admin API is
// available, runs `sudo frankenphp trust`, then stops the server.
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

	// Run trust with sudo — use sh -c with explicit env vars since sudo clears env.
	pvDir := config.PvDir()
	script := fmt.Sprintf(`XDG_DATA_HOME="%s" XDG_CONFIG_HOME="%s" "%s" trust`, pvDir, pvDir, frankenphp)
	trust := exec.Command("sudo", "sh", "-c", script)
	trust.Stdin = os.Stdin
	trust.Stdout = os.Stdout
	trust.Stderr = os.Stderr
	if err := trust.Run(); err != nil {
		return fmt.Errorf("frankenphp trust: %w", err)
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
