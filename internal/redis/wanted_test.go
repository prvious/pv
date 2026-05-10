package redis

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func install(t *testing.T) {
	t.Helper()
	if err := os.MkdirAll(config.RedisDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(config.RedisDir(), "redis-server"), []byte{}, 0o755); err != nil {
		t.Fatal(err)
	}
}

func TestIsWanted_NotInstalled(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if IsWanted() {
		t.Error("IsWanted should be false when binary missing")
	}
}

func TestIsWanted_InstalledButStopped(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	install(t)
	if err := SetWanted(WantedStopped); err != nil {
		t.Fatal(err)
	}
	if IsWanted() {
		t.Error("IsWanted should be false when wanted=stopped")
	}
}

func TestIsWanted_InstalledAndRunning(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	install(t)
	if err := SetWanted(WantedRunning); err != nil {
		t.Fatal(err)
	}
	if !IsWanted() {
		t.Error("IsWanted should be true when binary present and wanted=running")
	}
}

func TestIsWanted_StaleStateNoBinary(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	// SetWanted to running without installing the binary — drift case.
	if err := SetWanted(WantedRunning); err != nil {
		t.Fatal(err)
	}
	if IsWanted() {
		t.Error("IsWanted should be false when binary missing despite state=running")
	}
}
