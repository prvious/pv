package config

import (
	"testing"
)

func TestLoadSettings_DefaultWhenMissing(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	s, err := LoadSettings()
	if err != nil {
		t.Fatalf("LoadSettings() error = %v", err)
	}
	if s.TLD != "test" {
		t.Errorf("TLD = %q, want %q", s.TLD, "test")
	}
}

func TestSettings_SaveAndLoad(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	s := &Settings{TLD: "pv-test"}
	if err := s.Save(); err != nil {
		t.Fatalf("Save() error = %v", err)
	}

	loaded, err := LoadSettings()
	if err != nil {
		t.Fatalf("LoadSettings() error = %v", err)
	}
	if loaded.TLD != "pv-test" {
		t.Errorf("TLD = %q, want %q", loaded.TLD, "pv-test")
	}
}

func TestLoadSettings_EmptyTLDDefaultsToTest(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	s := &Settings{TLD: ""}
	if err := s.Save(); err != nil {
		t.Fatalf("Save() error = %v", err)
	}

	loaded, err := LoadSettings()
	if err != nil {
		t.Fatalf("LoadSettings() error = %v", err)
	}
	if loaded.TLD != "test" {
		t.Errorf("TLD = %q, want %q", loaded.TLD, "test")
	}
}

func TestDefaultSettings(t *testing.T) {
	s := DefaultSettings()
	if s.TLD != "test" {
		t.Errorf("TLD = %q, want %q", s.TLD, "test")
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
