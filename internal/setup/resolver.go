package setup

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"

	"github.com/prvious/pv/internal/config"
)

const (
	resolverDir     = "/etc/resolver"
	resolverContent = "nameserver 127.0.0.1\n"
)

// SudoSetupScript returns the shell script for the combined sudo operations:
// creating the DNS resolver file and trusting the Caddy CA certificate.
func SudoSetupScript(tld string) string {
	resolverFile := filepath.Join(resolverDir, tld)
	frankenphp := filepath.Join(config.BinDir(), "frankenphp")
	return fmt.Sprintf(
		`mkdir -p %s && printf 'nameserver 127.0.0.1\n' > %s && "%s" trust`,
		resolverDir, resolverFile, frankenphp,
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
