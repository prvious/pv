package certs

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func TestEnsureValetConfig(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := EnsureValetConfig("test"); err != nil {
		t.Fatalf("EnsureValetConfig() error = %v", err)
	}

	// Verify config.json.
	configPath := filepath.Join(home, ".config", "valet", "config.json")
	data, err := os.ReadFile(configPath)
	if err != nil {
		t.Fatalf("read config.json: %v", err)
	}

	var cfg valetConfig
	if err := json.Unmarshal(data, &cfg); err != nil {
		t.Fatalf("parse config.json: %v", err)
	}
	if cfg.TLD != "test" {
		t.Errorf("TLD = %q, want %q", cfg.TLD, "test")
	}

	// Verify Certificates directory exists.
	certsDir := filepath.Join(home, ".config", "valet", "Certificates")
	info, err := os.Stat(certsDir)
	if err != nil {
		t.Fatalf("Certificates dir: %v", err)
	}
	if !info.IsDir() {
		t.Error("Certificates is not a directory")
	}
}

func TestEnsureValetConfig_UpdatesTLD(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := EnsureValetConfig("test"); err != nil {
		t.Fatal(err)
	}
	if err := EnsureValetConfig("dev"); err != nil {
		t.Fatal(err)
	}

	configPath := filepath.Join(home, ".config", "valet", "config.json")
	data, _ := os.ReadFile(configPath)
	var cfg valetConfig
	json.Unmarshal(data, &cfg)

	if cfg.TLD != "dev" {
		t.Errorf("TLD = %q, want %q after update", cfg.TLD, "dev")
	}
}

func TestRemoveSiteTLS(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	certsDir := filepath.Join(home, ".config", "valet", "Certificates")
	os.MkdirAll(certsDir, 0755)

	// Create dummy cert files.
	certPath := filepath.Join(certsDir, "myapp.test.crt")
	keyPath := filepath.Join(certsDir, "myapp.test.key")
	os.WriteFile(certPath, []byte("cert"), 0644)
	os.WriteFile(keyPath, []byte("key"), 0600)

	RemoveSiteTLS("myapp.test")

	if _, err := os.Stat(certPath); !os.IsNotExist(err) {
		t.Error("cert file should be removed")
	}
	if _, err := os.Stat(keyPath); !os.IsNotExist(err) {
		t.Error("key file should be removed")
	}
}

func TestRemoveSiteTLS_NonExistent(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	// Should not panic on non-existent files.
	RemoveSiteTLS("nonexistent.test")
}

func TestRemoveAll(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := EnsureValetConfig("test"); err != nil {
		t.Fatal(err)
	}

	if err := RemoveAll(); err != nil {
		t.Fatalf("RemoveAll() error = %v", err)
	}

	valetDir := filepath.Join(home, ".config", "valet")
	if _, err := os.Stat(valetDir); !os.IsNotExist(err) {
		t.Error("valet dir should be removed")
	}
}
