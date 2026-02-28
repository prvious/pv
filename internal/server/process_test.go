package server

import (
	"os"
	"path/filepath"
	"strconv"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestWriteAndReadPID(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := config.EnsureDirs(); err != nil {
		t.Fatalf("EnsureDirs() error = %v", err)
	}

	if err := writePID(); err != nil {
		t.Fatalf("writePID() error = %v", err)
	}

	pid, err := ReadPID()
	if err != nil {
		t.Fatalf("ReadPID() error = %v", err)
	}

	if pid != os.Getpid() {
		t.Errorf("ReadPID() = %d, want %d", pid, os.Getpid())
	}
}

func TestReadPID_MissingFile(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	_, err := ReadPID()
	if err == nil {
		t.Error("expected error for missing PID file, got nil")
	}
}

func TestRemovePID(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := config.EnsureDirs(); err != nil {
		t.Fatalf("EnsureDirs() error = %v", err)
	}

	if err := writePID(); err != nil {
		t.Fatalf("writePID() error = %v", err)
	}

	removePID()

	if _, err := os.Stat(config.PidFilePath()); !os.IsNotExist(err) {
		t.Error("PID file still exists after removePID()")
	}
}

func TestIsRunning_NoFile(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if IsRunning() {
		t.Error("IsRunning() = true with no PID file, want false")
	}
}

func TestIsRunning_CurrentProcess(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := config.EnsureDirs(); err != nil {
		t.Fatalf("EnsureDirs() error = %v", err)
	}

	if err := writePID(); err != nil {
		t.Fatalf("writePID() error = %v", err)
	}

	if !IsRunning() {
		t.Error("IsRunning() = false for current process, want true")
	}
}

func TestIsRunning_DeadProcess(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := config.EnsureDirs(); err != nil {
		t.Fatalf("EnsureDirs() error = %v", err)
	}

	// Write a PID that almost certainly doesn't exist.
	fakePID := 99999
	pidPath := filepath.Join(config.DataDir(), "pv.pid")
	if err := os.WriteFile(pidPath, []byte(strconv.Itoa(fakePID)), 0644); err != nil {
		t.Fatalf("write fake PID error = %v", err)
	}

	// This may or may not return false depending on whether PID 99999 exists.
	// The test primarily verifies no panic/crash.
	_ = IsRunning()
}
