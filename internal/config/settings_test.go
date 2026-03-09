package config

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func TestLoadSettings_DefaultWhenMissing(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	s, err := LoadSettings()
	if err != nil {
		t.Fatalf("LoadSettings() error = %v", err)
	}
	if s.Defaults.TLD != "test" {
		t.Errorf("TLD = %q, want %q", s.Defaults.TLD, "test")
	}
}

func TestSettings_SaveAndLoad(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	s := &Settings{Defaults: Defaults{TLD: "pv-test"}}
	if err := s.Save(); err != nil {
		t.Fatalf("Save() error = %v", err)
	}

	loaded, err := LoadSettings()
	if err != nil {
		t.Fatalf("LoadSettings() error = %v", err)
	}
	if loaded.Defaults.TLD != "pv-test" {
		t.Errorf("TLD = %q, want %q", loaded.Defaults.TLD, "pv-test")
	}
}

func TestLoadSettings_EmptyTLDDefaultsToTest(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	s := &Settings{Defaults: Defaults{TLD: ""}}
	if err := s.Save(); err != nil {
		t.Fatalf("Save() error = %v", err)
	}

	loaded, err := LoadSettings()
	if err != nil {
		t.Fatalf("LoadSettings() error = %v", err)
	}
	if loaded.Defaults.TLD != "test" {
		t.Errorf("TLD = %q, want %q", loaded.Defaults.TLD, "test")
	}
}

func TestDefaultSettings(t *testing.T) {
	s := DefaultSettings()
	if s.Defaults.TLD != "test" {
		t.Errorf("TLD = %q, want %q", s.Defaults.TLD, "test")
	}
}

func TestValidateTLD(t *testing.T) {
	tests := []struct {
		tld     string
		wantErr bool
	}{
		{"test", false},
		{"pv-test", false},
		{"dev", false},
		{"my-tld", false},
		{"a", false},
		{"abc123", false},
		{"", true},
		{"-bad", true},
		{"bad-", true},
		{"UPPER", true},
		{"has.dot", true},
		{"has space", true},
		{"has_underscore", true},
	}

	for _, tt := range tests {
		t.Run(tt.tld, func(t *testing.T) {
			err := ValidateTLD(tt.tld)
			if (err != nil) != tt.wantErr {
				t.Errorf("ValidateTLD(%q) error = %v, wantErr %v", tt.tld, err, tt.wantErr)
			}
		})
	}
}

func TestSettings_SaveAndLoad_WithPHP(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	s := &Settings{Defaults: Defaults{TLD: "test", PHP: "8.4"}}
	if err := s.Save(); err != nil {
		t.Fatalf("Save() error = %v", err)
	}

	loaded, err := LoadSettings()
	if err != nil {
		t.Fatalf("LoadSettings() error = %v", err)
	}
	if loaded.Defaults.PHP != "8.4" {
		t.Errorf("PHP = %q, want %q", loaded.Defaults.PHP, "8.4")
	}
}

func TestLoadSettings_MigratesFromJSON(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	// Write old-format settings.json
	old := filepath.Join(home, ".pv", "config", "settings.json")
	data, _ := json.Marshal(struct {
		TLD       string `json:"tld"`
		GlobalPHP string `json:"global_php,omitempty"`
	}{TLD: "dev", GlobalPHP: "8.3"})
	if err := os.WriteFile(old, data, 0644); err != nil {
		t.Fatal(err)
	}

	loaded, err := LoadSettings()
	if err != nil {
		t.Fatalf("LoadSettings() error = %v", err)
	}
	if loaded.Defaults.TLD != "dev" {
		t.Errorf("TLD = %q, want %q", loaded.Defaults.TLD, "dev")
	}
	if loaded.Defaults.PHP != "8.3" {
		t.Errorf("PHP = %q, want %q", loaded.Defaults.PHP, "8.3")
	}

	// Old file should be removed
	if _, err := os.Stat(old); !os.IsNotExist(err) {
		t.Error("expected old settings.json to be removed after migration")
	}

	// New pv.yml should exist
	if _, err := os.Stat(SettingsPath()); err != nil {
		t.Errorf("expected pv.yml to exist after migration: %v", err)
	}
}
