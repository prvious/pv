package phpenv

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestMatchConstraint(t *testing.T) {
	installed := []string{"8.2", "8.3", "8.4", "8.5"}

	tests := []struct {
		name       string
		constraint string
		want       string
	}{
		{"caret 8.2", "^8.2", "8.5"},
		{"caret 8.3", "^8.3", "8.5"},
		{"tilde 8.2", "~8.2", "8.5"},
		{"tilde 8.4.0", "~8.4.0", "8.5"},
		{"gte 8.3", ">=8.3", "8.5"},
		{"gte range", ">=8.2 <8.4", "8.3"},
		{"gte range inclusive", ">=8.3 <8.5", "8.4"},
		{"wildcard", "8.3.*", "8.3"},
		{"exact", "8.4", "8.4"},
		{"exact with patch", "8.4.1", "8.4"},
		{"or constraint", "8.2|8.4", "8.4"},
		{"or with caret", "^8.2 || ^8.3", "8.5"},
		{"no match", ">=9.0", ""},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := matchConstraint(tt.constraint, installed)
			if got != tt.want {
				t.Errorf("matchConstraint(%q, %v) = %q, want %q", tt.constraint, installed, got, tt.want)
			}
		})
	}
}

func TestResolveVersion_PvPhpFile(t *testing.T) {
	scaffold(t)
	installFakeVersion(t, "8.3")
	installFakeVersion(t, "8.4")
	if err := SetGlobal("8.4"); err != nil {
		t.Fatal(err)
	}

	projDir := t.TempDir()
	if err := os.WriteFile(filepath.Join(projDir, ".pv-php"), []byte("8.3\n"), 0644); err != nil {
		t.Fatal(err)
	}

	v, err := ResolveVersion(projDir)
	if err != nil {
		t.Fatalf("ResolveVersion() error = %v", err)
	}
	if v != "8.3" {
		t.Errorf("ResolveVersion() = %q, want %q", v, "8.3")
	}
}

func TestResolveVersion_ComposerJSON(t *testing.T) {
	scaffold(t)
	installFakeVersion(t, "8.3")
	installFakeVersion(t, "8.4")
	if err := SetGlobal("8.3"); err != nil {
		t.Fatal(err)
	}

	projDir := t.TempDir()
	composer := `{"require": {"php": "^8.3"}}`
	if err := os.WriteFile(filepath.Join(projDir, "composer.json"), []byte(composer), 0644); err != nil {
		t.Fatal(err)
	}

	v, err := ResolveVersion(projDir)
	if err != nil {
		t.Fatalf("ResolveVersion() error = %v", err)
	}
	if v != "8.4" {
		t.Errorf("ResolveVersion() = %q, want %q (highest matching ^8.3)", v, "8.4")
	}
}

func TestResolveVersion_FallsBackToGlobal(t *testing.T) {
	scaffold(t)
	installFakeVersion(t, "8.4")
	if err := SetGlobal("8.4"); err != nil {
		t.Fatal(err)
	}

	projDir := t.TempDir()

	v, err := ResolveVersion(projDir)
	if err != nil {
		t.Fatalf("ResolveVersion() error = %v", err)
	}
	if v != "8.4" {
		t.Errorf("ResolveVersion() = %q, want %q", v, "8.4")
	}
}

func TestResolveVersion_PvPhpTakesPriority(t *testing.T) {
	scaffold(t)
	installFakeVersion(t, "8.3")
	installFakeVersion(t, "8.4")
	if err := SetGlobal("8.4"); err != nil {
		t.Fatal(err)
	}

	// Project has both .pv-php and composer.json — .pv-php wins.
	projDir := t.TempDir()
	if err := os.WriteFile(filepath.Join(projDir, ".pv-php"), []byte("8.3"), 0644); err != nil {
		t.Fatal(err)
	}
	composer := `{"require": {"php": "^8.4"}}`
	if err := os.WriteFile(filepath.Join(projDir, "composer.json"), []byte(composer), 0644); err != nil {
		t.Fatal(err)
	}

	v, err := ResolveVersion(projDir)
	if err != nil {
		t.Fatalf("ResolveVersion() error = %v", err)
	}
	if v != "8.3" {
		t.Errorf("ResolveVersion() = %q, want %q (.pv-php should take priority)", v, "8.3")
	}
}

func TestCompareVersions(t *testing.T) {
	tests := []struct {
		a, b string
		want int
	}{
		{"8.3", "8.4", -1},
		{"8.4", "8.3", 1},
		{"8.4", "8.4", 0},
		{"8.3", "9.0", -1},
		{"9.0", "8.5", 1},
	}
	for _, tt := range tests {
		got := compareVersions(tt.a, tt.b)
		// Normalize to sign.
		sign := 0
		if got > 0 {
			sign = 1
		} else if got < 0 {
			sign = -1
		}
		if sign != tt.want {
			t.Errorf("compareVersions(%q, %q) sign = %d, want %d", tt.a, tt.b, sign, tt.want)
		}
	}
}

func TestVersionSitesDir_CreatedOnEnsure(t *testing.T) {
	scaffold(t)

	// EnsureDirs should create PhpDir but not per-version dirs.
	dir := config.PhpDir()
	info, err := os.Stat(dir)
	if err != nil {
		t.Fatalf("PhpDir should exist after EnsureDirs: %v", err)
	}
	if !info.IsDir() {
		t.Error("PhpDir should be a directory")
	}
}
