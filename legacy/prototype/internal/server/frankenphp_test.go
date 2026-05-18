package server

import (
	"path/filepath"
	"slices"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestFrankenphpEnv_VersionedSetsPhpEnv(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := frankenphpEnv("8.4")
	wantPHPRC := "PHPRC=" + filepath.Join(home, ".pv", "php", "8.4", "etc")
	wantScan := "PHP_INI_SCAN_DIR=" + filepath.Join(home, ".pv", "php", "8.4", "conf.d")

	if !slices.Contains(got, wantPHPRC) {
		t.Errorf("frankenphpEnv(\"8.4\") missing %q; got: %v", wantPHPRC, got)
	}
	if !slices.Contains(got, wantScan) {
		t.Errorf("frankenphpEnv(\"8.4\") missing %q; got: %v", wantScan, got)
	}

	// Should also still include CaddyEnv entries (XDG_DATA_HOME etc.).
	for _, want := range config.CaddyEnv() {
		if !slices.Contains(got, want) {
			t.Errorf("frankenphpEnv missing CaddyEnv entry %q", want)
		}
	}
}

func TestFrankenphpEnv_EmptyVersionOmitsPhpEnv(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := frankenphpEnv("")

	for _, e := range got {
		if strings.HasPrefix(e, "PHPRC=") || strings.HasPrefix(e, "PHP_INI_SCAN_DIR=") {
			t.Errorf("frankenphpEnv(\"\") leaked PHP env var: %q", e)
		}
	}
}
