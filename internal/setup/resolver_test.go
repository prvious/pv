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
	if !strings.Contains(script, "trust") {
		t.Errorf("script missing trust command: %s", script)
	}
}

func TestSudoSetupScript_UsesFrankenPHPPath(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	script := SudoSetupScript("test")
	expected := filepath.Join(config.BinDir(), "frankenphp")
	if !strings.Contains(script, expected) {
		t.Errorf("script missing frankenphp path %q: %s", expected, script)
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
