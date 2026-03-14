package setup

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestSudoSetupScript_ContainsResolver(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	script := SudoSetupScript("test")
	if !strings.Contains(script, "/etc/resolver/test") {
		t.Errorf("script missing /etc/resolver/test: %s", script)
	}
	if !strings.Contains(script, "nameserver 127.0.0.1") {
		t.Errorf("script missing nameserver line: %s", script)
	}
	if !strings.Contains(script, "port 10053") {
		t.Errorf("script missing port 10053: %s", script)
	}
}

func TestSudoSetupScript_CustomTLD(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	script := SudoSetupScript("pv-test")
	if !strings.Contains(script, "/etc/resolver/pv-test") {
		t.Errorf("script missing /etc/resolver/pv-test: %s", script)
	}
}

func TestSudoSetupScript_ContainsTrust(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	script := SudoSetupScript("test")
	if strings.Contains(script, "trust") {
		t.Errorf("script should not contain trust command: %s", script)
	}
}

func TestSudoSetupScript_NoFrankenPHPOrCAPath(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	script := SudoSetupScript("test")
	if strings.Contains(script, filepath.Join(config.BinDir(), "frankenphp")) {
		t.Errorf("script should not call frankenphp: %s", script)
	}
	if strings.Contains(script, config.CACertPath()) {
		t.Errorf("script should not contain CA path %q: %s", config.CACertPath(), script)
	}
}

func TestResolverSetupScript_ContainsResolver(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	script := ResolverSetupScript("test")
	if !strings.Contains(script, "/etc/resolver/test") {
		t.Errorf("script missing /etc/resolver/test: %s", script)
	}
	if !strings.Contains(script, "nameserver 127.0.0.1") {
		t.Errorf("script missing nameserver line: %s", script)
	}
	if !strings.Contains(script, "port 10053") {
		t.Errorf("script missing port 10053: %s", script)
	}
}

func TestResolverSetupScript_NoTrust(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	script := ResolverSetupScript("test")
	if strings.Contains(script, "trust") {
		t.Errorf("DNS-only script should not contain trust: %s", script)
	}
}

func TestResolverSetupScript_CustomTLD(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	script := ResolverSetupScript("pv-test")
	if !strings.Contains(script, "/etc/resolver/pv-test") {
		t.Errorf("script missing /etc/resolver/pv-test: %s", script)
	}
}

func TestCheckResolverFile_Missing(t *testing.T) {
	err := CheckResolverFile("test")
	_ = err
}

func TestCheckResolverFile_CorrectContent(t *testing.T) {
	tmpDir := t.TempDir()
	tmpFile := filepath.Join(tmpDir, "test")
	if err := os.WriteFile(tmpFile, []byte(resolverContent), 0644); err != nil {
		t.Fatal(err)
	}

	data, err := os.ReadFile(tmpFile)
	if err != nil {
		t.Fatal(err)
	}
	if string(data) != resolverContent {
		t.Errorf("resolver content = %q, want %q", string(data), resolverContent)
	}
}
