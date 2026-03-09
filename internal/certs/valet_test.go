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

	var cfg map[string]any
	if err := json.Unmarshal(data, &cfg); err != nil {
		t.Fatalf("parse config.json: %v", err)
	}
	if cfg["tld"] != "test" {
		t.Errorf("TLD = %q, want %q", cfg["tld"], "test")
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
	var cfg map[string]any
	json.Unmarshal(data, &cfg)

	if cfg["tld"] != "dev" {
		t.Errorf("TLD = %q, want %q after update", cfg["tld"], "dev")
	}
}

func TestEnsureValetConfig_PreservesExistingFields(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	// Write a pre-existing config with extra fields (e.g., real Valet config).
	configDir := filepath.Join(home, ".config", "valet")
	os.MkdirAll(configDir, 0755)
	existing := map[string]any{
		"tld":      "local",
		"loopback": "127.0.0.1",
		"paths":    []string{"/home/user/Sites"},
	}
	data, _ := json.Marshal(existing)
	os.WriteFile(filepath.Join(configDir, "config.json"), data, 0644)

	if err := EnsureValetConfig("test"); err != nil {
		t.Fatal(err)
	}

	result, _ := os.ReadFile(filepath.Join(configDir, "config.json"))
	var cfg map[string]any
	json.Unmarshal(result, &cfg)

	if cfg["tld"] != "test" {
		t.Errorf("TLD = %q, want %q", cfg["tld"], "test")
	}
	if cfg["loopback"] != "127.0.0.1" {
		t.Error("existing loopback field was lost")
	}
	if cfg["paths"] == nil {
		t.Error("existing paths field was lost")
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

	if err := RemoveSiteTLS("myapp.test"); err != nil {
		t.Fatalf("RemoveSiteTLS() error = %v", err)
	}

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

	// Should not error on non-existent files.
	if err := RemoveSiteTLS("nonexistent.test"); err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
}

func TestRemoveLinkedCerts(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	certsDir := filepath.Join(home, ".config", "valet", "Certificates")
	os.MkdirAll(certsDir, 0755)

	// Create certs for two "linked" apps and one "other" app.
	for _, name := range []string{"app1.test", "app2.test", "other.test"} {
		os.WriteFile(filepath.Join(certsDir, name+".crt"), []byte("cert"), 0644)
		os.WriteFile(filepath.Join(certsDir, name+".key"), []byte("key"), 0600)
	}

	// Only remove linked apps.
	if err := RemoveLinkedCerts([]string{"app1.test", "app2.test"}); err != nil {
		t.Fatalf("RemoveLinkedCerts() error = %v", err)
	}

	// Linked certs should be gone.
	for _, name := range []string{"app1.test", "app2.test"} {
		if _, err := os.Stat(filepath.Join(certsDir, name+".crt")); !os.IsNotExist(err) {
			t.Errorf("%s.crt should be removed", name)
		}
		if _, err := os.Stat(filepath.Join(certsDir, name+".key")); !os.IsNotExist(err) {
			t.Errorf("%s.key should be removed", name)
		}
	}

	// Other cert should still exist.
	if _, err := os.Stat(filepath.Join(certsDir, "other.test.crt")); err != nil {
		t.Error("other.test.crt should NOT be removed")
	}
	if _, err := os.Stat(filepath.Join(certsDir, "other.test.key")); err != nil {
		t.Error("other.test.key should NOT be removed")
	}
}

func TestRemoveConfig(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := EnsureValetConfig("test"); err != nil {
		t.Fatal(err)
	}

	if err := RemoveConfig(); err != nil {
		t.Fatalf("RemoveConfig() error = %v", err)
	}

	configPath := filepath.Join(home, ".config", "valet", "config.json")
	if _, err := os.Stat(configPath); !os.IsNotExist(err) {
		t.Error("config.json should be removed")
	}

	// Certificates directory should still exist.
	certsDir := filepath.Join(home, ".config", "valet", "Certificates")
	if _, err := os.Stat(certsDir); err != nil {
		t.Error("Certificates dir should NOT be removed")
	}
}

func TestRemoveConfig_NonExistent(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	// Should not error when config doesn't exist.
	if err := RemoveConfig(); err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
}

func TestValetConfigDir_ReturnsAbsolutePath(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	dir, err := ValetConfigDir()
	if err != nil {
		t.Fatalf("ValetConfigDir() error = %v", err)
	}
	if !filepath.IsAbs(dir) {
		t.Errorf("ValetConfigDir() = %q, want absolute path", dir)
	}
}
