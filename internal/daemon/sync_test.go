package daemon

import (
	"os"
	"path/filepath"
	"testing"
)

func TestNeedsSync_NoPlistOnDisk(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	cfg := PlistConfig{
		Label:        Label,
		PvBinaryPath: filepath.Join(home, ".pv", "bin", "pv"),
		LogDir:       filepath.Join(home, ".pv", "logs"),
		HomeDir:      filepath.Join(home, ".pv"),
		EnvVars:      map[string]string{"PATH": "/usr/bin"},
	}

	needs, err := NeedsSync(cfg)
	if err != nil {
		t.Fatalf("NeedsSync error: %v", err)
	}
	if !needs {
		t.Error("expected NeedsSync=true when no plist on disk")
	}
}

func TestNeedsSync_Identical(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	cfg := PlistConfig{
		Label:        Label,
		PvBinaryPath: filepath.Join(home, ".pv", "bin", "pv"),
		LogDir:       filepath.Join(home, ".pv", "logs"),
		HomeDir:      filepath.Join(home, ".pv"),
		EnvVars:      map[string]string{"PATH": "/usr/bin"},
	}

	// Write the plist.
	if err := WritePlist(cfg); err != nil {
		t.Fatalf("WritePlist error: %v", err)
	}

	// Same config — should not need sync.
	needs, err := NeedsSync(cfg)
	if err != nil {
		t.Fatalf("NeedsSync error: %v", err)
	}
	if needs {
		t.Error("expected NeedsSync=false when plist matches")
	}
}

func TestNeedsSync_Different(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	cfg := PlistConfig{
		Label:        Label,
		PvBinaryPath: filepath.Join(home, ".pv", "bin", "pv"),
		LogDir:       filepath.Join(home, ".pv", "logs"),
		HomeDir:      filepath.Join(home, ".pv"),
		EnvVars:      map[string]string{"PATH": "/usr/bin"},
	}

	// Write plist with original config.
	if err := WritePlist(cfg); err != nil {
		t.Fatalf("WritePlist error: %v", err)
	}

	// Change config — should need sync.
	cfg.PvBinaryPath = filepath.Join(home, ".pv", "php", "8.3", "frankenphp")
	needs, err := NeedsSync(cfg)
	if err != nil {
		t.Fatalf("NeedsSync error: %v", err)
	}
	if !needs {
		t.Error("expected NeedsSync=true when config changed")
	}
}

func TestNeedsSync_RunAtLoadChange(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	cfg := PlistConfig{
		Label:        Label,
		PvBinaryPath: filepath.Join(home, ".pv", "bin", "pv"),
		LogDir:       filepath.Join(home, ".pv", "logs"),
		HomeDir:      filepath.Join(home, ".pv"),
		RunAtLoad:    false,
		EnvVars:      map[string]string{"PATH": "/usr/bin"},
	}

	if err := WritePlist(cfg); err != nil {
		t.Fatalf("WritePlist error: %v", err)
	}

	// Toggle RunAtLoad.
	cfg.RunAtLoad = true
	needs, err := NeedsSync(cfg)
	if err != nil {
		t.Fatalf("NeedsSync error: %v", err)
	}
	if !needs {
		t.Error("expected NeedsSync=true when RunAtLoad changed")
	}
}

func TestNeedsSync_CorruptedPlist(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	// Write garbage to the plist location.
	plistDir := filepath.Join(home, "Library", "LaunchAgents")
	os.MkdirAll(plistDir, 0755)
	os.WriteFile(filepath.Join(plistDir, Label+".plist"), []byte("garbage"), 0644)

	cfg := PlistConfig{
		Label:        Label,
		PvBinaryPath: filepath.Join(home, ".pv", "bin", "pv"),
		LogDir:       filepath.Join(home, ".pv", "logs"),
		HomeDir:      filepath.Join(home, ".pv"),
		EnvVars:      map[string]string{"PATH": "/usr/bin"},
	}

	needs, err := NeedsSync(cfg)
	if err != nil {
		t.Fatalf("NeedsSync error: %v", err)
	}
	if !needs {
		t.Error("expected NeedsSync=true when plist is corrupted")
	}
}
