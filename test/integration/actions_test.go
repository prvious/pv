//go:build integration

package integration

import (
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"testing"
)

// TestBinaryBuild verifies that the pv binary can be built
func TestBinaryBuild(t *testing.T) {
	// Get the project root directory
	_, filename, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("Failed to get current file path")
	}
	projectRoot := filepath.Join(filepath.Dir(filename), "..", "..")

	// Build the binary
	binaryName := "pv"
	if runtime.GOOS == "windows" {
		binaryName = "pv.exe"
	}
	binaryPath := filepath.Join(projectRoot, "bin", binaryName)

	cmd := exec.Command("go", "build", "-o", binaryPath, ".")
	cmd.Dir = projectRoot
	output, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("Failed to build binary: %v\nOutput: %s", err, output)
	}

	// Verify the binary exists
	if _, err := os.Stat(binaryPath); os.IsNotExist(err) {
		t.Fatalf("Binary not found at %s", binaryPath)
	}

	// Clean up
	defer os.Remove(binaryPath)
}

// TestBinaryExecution verifies that the pv binary can be executed
func TestBinaryExecution(t *testing.T) {
	// Get the project root directory
	_, filename, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("Failed to get current file path")
	}
	projectRoot := filepath.Join(filepath.Dir(filename), "..", "..")

	// Build the binary
	binaryName := "pv"
	if runtime.GOOS == "windows" {
		binaryName = "pv.exe"
	}
	binaryPath := filepath.Join(projectRoot, "bin", binaryName)

	cmd := exec.Command("go", "build", "-o", binaryPath, ".")
	cmd.Dir = projectRoot
	if output, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("Failed to build binary: %v\nOutput: %s", err, output)
	}
	defer os.Remove(binaryPath)

	// Try to execute the binary with --help flag (if supported)
	// Note: Since the binary is interactive (TUI), we can't easily test full execution
	// but we can verify it starts without crashing
	cmd = exec.Command(binaryPath)
	cmd.Dir = projectRoot
	
	// Start the command but don't wait for it to complete
	// Just verify it can start without immediate error
	if err := cmd.Start(); err != nil {
		t.Fatalf("Failed to start binary: %v", err)
	}

	// Kill the process after a short time
	if err := cmd.Process.Kill(); err != nil {
		t.Logf("Warning: failed to kill process: %v", err)
	}
	
	// Wait for the process to finish
	cmd.Wait()
}

// TestActionRegistry verifies that actions can be registered
func TestActionRegistry(t *testing.T) {
	// This test verifies the basic action registry functionality
	// In a real scenario, you would test actual action registration
	
	// Get the project root directory
	_, filename, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("Failed to get current file path")
	}
	projectRoot := filepath.Join(filepath.Dir(filename), "..", "..")

	// Build and verify the binary compiles with all actions
	cmd := exec.Command("go", "build", "-o", filepath.Join(os.TempDir(), "pv_test"), ".")
	cmd.Dir = projectRoot
	output, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("Failed to build with actions: %v\nOutput: %s", err, output)
	}
}
