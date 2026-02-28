package setup

import (
	"fmt"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/prvious/pv/internal/config"
)

// TestResult holds the outcome of a single self-test check.
type TestResult struct {
	Name string
	Err  error
}

// RunSelfTest runs verification checks and returns results.
func RunSelfTest(tld string) []TestResult {
	var results []TestResult
	results = append(results, checkBinary("FrankenPHP", "frankenphp", "version"))
	results = append(results, checkBinary("Mago", "mago", "--version"))
	results = append(results, checkBinary("PHP CLI", "php", "--version"))
	results = append(results, checkResolverConfigured(tld))
	results = append(results, checkFrankenPHPBoots())
	return results
}

func checkBinary(displayName, binaryName string, args ...string) TestResult {
	binPath := filepath.Join(config.BinDir(), binaryName)
	cmd := exec.Command(binPath, args...)
	output, err := cmd.CombinedOutput()
	if err != nil {
		return TestResult{displayName, fmt.Errorf("%v: %s", err, strings.TrimSpace(string(output)))}
	}
	return TestResult{displayName, nil}
}


func checkResolverConfigured(tld string) TestResult {
	if err := CheckResolverFile(tld); err != nil {
		return TestResult{"DNS resolver", err}
	}
	return TestResult{"DNS resolver", nil}
}

func checkFrankenPHPBoots() TestResult {
	frankenphp := filepath.Join(config.BinDir(), "frankenphp")
	caddyfile := config.CaddyfilePath()

	cmd := exec.Command(frankenphp, "run", "--config", caddyfile, "--adapter", "caddyfile")
	cmd.Stdout = nil
	cmd.Stderr = nil

	if err := cmd.Start(); err != nil {
		return TestResult{"FrankenPHP boots", fmt.Errorf("failed to start: %w", err)}
	}

	// Wait briefly to see if process stays up.
	done := make(chan error, 1)
	go func() { done <- cmd.Wait() }()

	select {
	case err := <-done:
		// Process exited on its own — likely a crash or config error.
		return TestResult{"FrankenPHP boots", fmt.Errorf("exited unexpectedly: %v", err)}
	case <-time.After(3 * time.Second):
		// Still running after 3s — it booted successfully.
		cmd.Process.Kill()
		<-done
		return TestResult{"FrankenPHP boots", nil}
	}
}

// PrintResults prints self-test results with checkmarks.
func PrintResults(results []TestResult) bool {
	allPassed := true
	for _, r := range results {
		if r.Err != nil {
			fmt.Printf("  x %s: %v\n", r.Name, r.Err)
			allPassed = false
		} else {
			fmt.Printf("  ✓ %s\n", r.Name)
		}
	}
	return allPassed
}
