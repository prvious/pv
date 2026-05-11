package config

import (
	"os"
	"strings"
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

func TestLoadSettings_EmptyTLDDefaultsToTest(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	// Write a pv.yml with empty TLD directly to bypass Save() validation.
	if err := EnsureDirs(); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(SettingsPath(), []byte("defaults:\n    tld: \"\"\n"), 0644); err != nil {
		t.Fatal(err)
	}

	loaded, err := LoadSettings()
	if err != nil {
		t.Fatalf("LoadSettings() error = %v", err)
	}
	if loaded.Defaults.TLD != "test" {
		t.Errorf("TLD = %q, want %q", loaded.Defaults.TLD, "test")
	}
}

func TestLoadSettings_CorruptYAML(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := EnsureDirs(); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(SettingsPath(), []byte("defaults: [broken\n"), 0644); err != nil {
		t.Fatal(err)
	}

	_, err := LoadSettings()
	if err == nil {
		t.Error("expected error for corrupt YAML")
	}
}

func TestDefaultSettings(t *testing.T) {
	s := DefaultSettings()
	if s.Defaults.TLD != "test" {
		t.Errorf("TLD = %q, want %q", s.Defaults.TLD, "test")
	}
}

func TestSettings_SaveValidatesTLD(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	s := &Settings{Defaults: Defaults{TLD: "-bad-"}}
	if err := s.Save(); err == nil {
		t.Error("expected Save() to reject invalid TLD")
	}
}

func TestSettings_SaveDefaultsEmptyTLD(t *testing.T) {
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

func TestSettings_SaveWritesExpectedYAML(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	s := &Settings{Defaults: Defaults{TLD: "test", PHP: "8.4"}}
	if err := s.Save(); err != nil {
		t.Fatalf("Save() error = %v", err)
	}

	data, err := os.ReadFile(SettingsPath())
	if err != nil {
		t.Fatal(err)
	}
	content := string(data)
	if !strings.Contains(content, "tld: test") {
		t.Errorf("expected 'tld: test' in YAML, got:\n%s", content)
	}
	if !strings.Contains(content, `php: "8.4"`) {
		t.Errorf("expected 'php: \"8.4\"' in YAML, got:\n%s", content)
	}
}

func TestLoadSettings_EmptyFile(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	if err := EnsureDirs(); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(SettingsPath(), []byte{}, 0644); err != nil {
		t.Fatal(err)
	}

	loaded, err := LoadSettings()
	if err != nil {
		t.Fatalf("LoadSettings() error = %v", err)
	}
	if loaded.Defaults.TLD != "test" {
		t.Errorf("TLD = %q, want %q", loaded.Defaults.TLD, "test")
	}
	if loaded.Defaults.PHP != "" {
		t.Errorf("PHP = %q, want empty", loaded.Defaults.PHP)
	}
}

func TestSettings_SaveWithPHPOnlyDefaultsTLD(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	s := &Settings{Defaults: Defaults{PHP: "8.3"}}
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
	if loaded.Defaults.PHP != "8.3" {
		t.Errorf("PHP = %q, want %q", loaded.Defaults.PHP, "8.3")
	}
}

func TestDefaultSettings_HasInstallPHPVersion(t *testing.T) {
	s := DefaultSettings()
	if s.Automation.InstallPHPVersion != AutoOn {
		t.Errorf("InstallPHPVersion = %q, want %q", s.Automation.InstallPHPVersion, AutoOn)
	}
}

func TestDefaultSettings_HasAutomationDefaults(t *testing.T) {
	s := DefaultSettings()

	a := s.Automation
	if a.InstallPHPVersion != AutoOn {
		t.Errorf("InstallPHPVersion = %q, want %q", a.InstallPHPVersion, AutoOn)
	}
	if a.SetAppURL != AutoOn {
		t.Errorf("SetAppURL = %q, want %q", a.SetAppURL, AutoOn)
	}
	if a.SetViteTLS != AutoOn {
		t.Errorf("SetViteTLS = %q, want %q", a.SetViteTLS, AutoOn)
	}
	if a.ServiceFallback != AutoOn {
		t.Errorf("ServiceFallback = %q, want %q", a.ServiceFallback, AutoOn)
	}
	if a.GenerateSiteConfig != AutoOn {
		t.Errorf("GenerateSiteConfig = %q, want %q", a.GenerateSiteConfig, AutoOn)
	}
	if a.GenerateCaddyfile != AutoOn {
		t.Errorf("GenerateCaddyfile = %q, want %q", a.GenerateCaddyfile, AutoOn)
	}
	if a.GenerateTLSCert != AutoOn {
		t.Errorf("GenerateTLSCert = %q, want %q", a.GenerateTLSCert, AutoOn)
	}
}

func TestSettings_AutomationRoundTrip(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	s := DefaultSettings()
	s.Automation.SetAppURL = AutoOff
	s.Automation.SetViteTLS = AutoAsk
	s.Automation.ServiceFallback = AutoOff

	if err := s.Save(); err != nil {
		t.Fatalf("Save() error = %v", err)
	}

	loaded, err := LoadSettings()
	if err != nil {
		t.Fatalf("LoadSettings() error = %v", err)
	}

	if loaded.Automation.SetAppURL != AutoOff {
		t.Errorf("SetAppURL = %q, want %q", loaded.Automation.SetAppURL, AutoOff)
	}
	if loaded.Automation.SetViteTLS != AutoAsk {
		t.Errorf("SetViteTLS = %q, want %q", loaded.Automation.SetViteTLS, AutoAsk)
	}
	if loaded.Automation.ServiceFallback != AutoOff {
		t.Errorf("ServiceFallback = %q, want %q", loaded.Automation.ServiceFallback, AutoOff)
	}
	// Verify unmodified fields kept their defaults
	if loaded.Automation.InstallPHPVersion != AutoOn {
		t.Errorf("InstallPHPVersion = %q, want %q", loaded.Automation.InstallPHPVersion, AutoOn)
	}
}

func TestLoadSettings_MissingAutomationGetsDefaults(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	if err := EnsureDirs(); err != nil {
		t.Fatal(err)
	}
	// Write YAML with only defaults section, no automation
	if err := os.WriteFile(SettingsPath(), []byte("defaults:\n    tld: test\n"), 0644); err != nil {
		t.Fatal(err)
	}

	loaded, err := LoadSettings()
	if err != nil {
		t.Fatalf("LoadSettings() error = %v", err)
	}

	a := loaded.Automation
	if a.InstallPHPVersion != AutoOn {
		t.Errorf("InstallPHPVersion = %q, want %q", a.InstallPHPVersion, AutoOn)
	}
	if a.SetAppURL != AutoOn {
		t.Errorf("SetAppURL = %q, want %q", a.SetAppURL, AutoOn)
	}
	if a.SetViteTLS != AutoOn {
		t.Errorf("SetViteTLS = %q, want %q", a.SetViteTLS, AutoOn)
	}
	if a.ServiceFallback != AutoOn {
		t.Errorf("ServiceFallback = %q, want %q", a.ServiceFallback, AutoOn)
	}
	if a.GenerateSiteConfig != AutoOn {
		t.Errorf("GenerateSiteConfig = %q, want %q", a.GenerateSiteConfig, AutoOn)
	}
	if a.GenerateCaddyfile != AutoOn {
		t.Errorf("GenerateCaddyfile = %q, want %q", a.GenerateCaddyfile, AutoOn)
	}
	if a.GenerateTLSCert != AutoOn {
		t.Errorf("GenerateTLSCert = %q, want %q", a.GenerateTLSCert, AutoOn)
	}
}

func TestLoadSettings_InvalidAutoModeResetToDefault(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := EnsureDirs(); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(SettingsPath(), []byte("automation:\n    set_app_url: banana\n    service_fallback: \"false\"\n"), 0644); err != nil {
		t.Fatal(err)
	}

	loaded, err := LoadSettings()
	if err != nil {
		t.Fatalf("LoadSettings() error = %v", err)
	}
	// "banana" is invalid → should be reset to default ("true").
	if loaded.Automation.SetAppURL != AutoOn {
		t.Errorf("SetAppURL = %q, want %q (invalid value should reset to default)", loaded.Automation.SetAppURL, AutoOn)
	}
	// "false" is valid → should be preserved.
	if loaded.Automation.ServiceFallback != AutoOff {
		t.Errorf("ServiceFallback = %q, want %q", loaded.Automation.ServiceFallback, AutoOff)
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
