package daemon

import (
	"fmt"
	"os"
	"path/filepath"
	"testing"
)

func TestDomainTarget(t *testing.T) {
	target := domainTarget()
	expected := fmt.Sprintf("gui/%d", os.Getuid())
	if target != expected {
		t.Errorf("domainTarget() = %q, want %q", target, expected)
	}
}

func TestServiceTarget(t *testing.T) {
	target := serviceTarget()
	expected := fmt.Sprintf("gui/%d/%s", os.Getuid(), Label)
	if target != expected {
		t.Errorf("serviceTarget() = %q, want %q", target, expected)
	}
}

func TestInstall(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	cfg := PlistConfig{
		Label:        Label,
		PvBinaryPath: filepath.Join(home, ".pv", "bin", "pv"),
		LogDir:       filepath.Join(home, ".pv", "logs"),
		HomeDir:      filepath.Join(home, ".pv"),
		EnvVars:      map[string]string{"PATH": "/usr/bin"},
	}

	if err := Install(cfg); err != nil {
		t.Fatalf("Install error: %v", err)
	}

	plistPath := filepath.Join(home, "Library", "LaunchAgents", Label+".plist")
	if _, err := os.Stat(plistPath); err != nil {
		t.Fatalf("plist file not created: %v", err)
	}
}

func TestUninstall(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	// Install first.
	cfg := PlistConfig{
		Label:        Label,
		PvBinaryPath: "/usr/local/bin/pv",
		LogDir:       "/tmp/logs",
		HomeDir:      "/tmp",
		EnvVars:      map[string]string{},
	}
	if err := Install(cfg); err != nil {
		t.Fatalf("Install error: %v", err)
	}

	// Uninstall.
	if err := Uninstall(); err != nil {
		t.Fatalf("Uninstall error: %v", err)
	}

	plistPath := filepath.Join(home, "Library", "LaunchAgents", Label+".plist")
	if _, err := os.Stat(plistPath); !os.IsNotExist(err) {
		t.Error("plist file should not exist after Uninstall")
	}
}

func TestUninstall_NoFileIsOk(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := Uninstall(); err != nil {
		t.Fatalf("Uninstall on missing file should not error: %v", err)
	}
}
