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
		// Caret constraints — always allow up to next major.
		{"caret 8.2", "^8.2", "8.5"},
		{"caret 8.3", "^8.3", "8.5"},
		{"caret 8.2.0 (with patch)", "^8.2.0", "8.5"},
		{"caret 8.3.1 (with patch)", "^8.3.1", "8.5"},
		{"caret 8.4.99 (with patch)", "^8.4.99", "8.5"},

		// Tilde without patch — same as caret (next major break).
		{"tilde 8.2 (no patch)", "~8.2", "8.5"},
		{"tilde 8.3 (no patch)", "~8.3", "8.5"},

		// Tilde WITH patch — locks to minor version (the key fix).
		{"tilde 8.2.0 locks to 8.2.x", "~8.2.0", "8.2"},
		{"tilde 8.3.0 locks to 8.3.x", "~8.3.0", "8.3"},
		{"tilde 8.4.0 locks to 8.4.x", "~8.4.0", "8.4"},
		{"tilde 8.4.5 locks to 8.4.x", "~8.4.5", "8.4"},
		{"tilde 8.5.0 locks to 8.5.x", "~8.5.0", "8.5"},
		{"tilde 8.5.99 locks to 8.5.x", "~8.5.99", "8.5"},

		// Range constraints.
		{"gte 8.3", ">=8.3", "8.5"},
		{"gte range", ">=8.2 <8.4", "8.3"},
		{"gte range inclusive", ">=8.3 <8.5", "8.4"},
		{"gte range tight", ">=8.4 <8.5", "8.4"},

		// Wildcard.
		{"wildcard", "8.3.*", "8.3"},

		// Exact.
		{"exact", "8.4", "8.4"},
		{"exact with patch", "8.4.1", "8.4"},

		// OR constraints.
		{"or constraint", "8.2|8.4", "8.4"},
		{"or with caret", "^8.2 || ^8.3", "8.5"},
		{"or with tilde patch", "~8.2.0 || ~8.4.0", "8.4"},

		// No match.
		{"no match", ">=9.0", ""},
		{"tilde no match", "~9.0.0", ""},
		{"caret no match", "^9.0", ""},
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

func TestMatchConstraint_TildePatchBoundary(t *testing.T) {
	// Focused test: with 8.3 and 8.4 installed, ~8.3.0 MUST pick 8.3 (not 8.4).
	installed := []string{"8.3", "8.4"}

	got := matchConstraint("~8.3.0", installed)
	if got != "8.3" {
		t.Errorf("~8.3.0 with [8.3, 8.4] installed = %q, want %q — tilde with patch must lock to minor", got, "8.3")
	}

	// But ^8.3.0 should pick 8.4.
	got = matchConstraint("^8.3.0", installed)
	if got != "8.4" {
		t.Errorf("^8.3.0 with [8.3, 8.4] installed = %q, want %q — caret should allow minor bumps", got, "8.4")
	}
}

func TestMatchConstraint_TildeVsCaretSameInput(t *testing.T) {
	installed := []string{"8.2", "8.3", "8.4"}

	// ~8.2.0 → only 8.2 (locked to minor)
	// ^8.2.0 → 8.4 (allows up to 9.0)
	// ~8.2   → 8.4 (no patch, same as caret)
	// ^8.2   → 8.4 (same as always)
	tests := []struct {
		constraint string
		want       string
	}{
		{"~8.2.0", "8.2"},
		{"^8.2.0", "8.4"},
		{"~8.2", "8.4"},
		{"^8.2", "8.4"},
	}

	for _, tt := range tests {
		t.Run(tt.constraint, func(t *testing.T) {
			got := matchConstraint(tt.constraint, installed)
			if got != tt.want {
				t.Errorf("matchConstraint(%q, %v) = %q, want %q", tt.constraint, installed, got, tt.want)
			}
		})
	}
}

func TestResolveVersion_PvYml(t *testing.T) {
	scaffold(t)
	installFakeVersion(t, "8.3")
	installFakeVersion(t, "8.4")
	if err := SetGlobal("8.4"); err != nil {
		t.Fatal(err)
	}

	projDir := t.TempDir()
	if err := os.WriteFile(filepath.Join(projDir, "pv.yml"), []byte("php: \"8.3\"\n"), 0644); err != nil {
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

func TestResolveVersion_PvYmlUnquoted(t *testing.T) {
	scaffold(t)
	installFakeVersion(t, "8.4")
	if err := SetGlobal("8.4"); err != nil {
		t.Fatal(err)
	}

	projDir := t.TempDir()
	// Unquoted YAML value — should still work.
	if err := os.WriteFile(filepath.Join(projDir, "pv.yml"), []byte("php: 8.4\n"), 0644); err != nil {
		t.Fatal(err)
	}

	v, err := ResolveVersion(projDir)
	if err != nil {
		t.Fatalf("ResolveVersion() error = %v", err)
	}
	if v != "8.4" {
		t.Errorf("ResolveVersion() = %q, want %q", v, "8.4")
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

func TestResolveVersion_PvYmlTakesPriority(t *testing.T) {
	scaffold(t)
	installFakeVersion(t, "8.3")
	installFakeVersion(t, "8.4")
	if err := SetGlobal("8.4"); err != nil {
		t.Fatal(err)
	}

	// Project has both pv.yml and composer.json — pv.yml wins.
	projDir := t.TempDir()
	if err := os.WriteFile(filepath.Join(projDir, "pv.yml"), []byte("php: \"8.3\"\n"), 0644); err != nil {
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
		t.Errorf("ResolveVersion() = %q, want %q (pv.yml should take priority)", v, "8.3")
	}
}

func TestResolveVersionWalkUp_PvYmlInParent(t *testing.T) {
	scaffold(t)
	installFakeVersion(t, "8.3")
	installFakeVersion(t, "8.4")
	if err := SetGlobal("8.4"); err != nil {
		t.Fatal(err)
	}

	projDir := t.TempDir()
	subDir := filepath.Join(projDir, "src", "deep")
	if err := os.MkdirAll(subDir, 0755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(projDir, "pv.yml"), []byte("php: \"8.3\"\n"), 0644); err != nil {
		t.Fatal(err)
	}

	v, err := ResolveVersionWalkUp(subDir)
	if err != nil {
		t.Fatalf("ResolveVersionWalkUp() error = %v", err)
	}
	if v != "8.3" {
		t.Errorf("ResolveVersionWalkUp() = %q, want %q", v, "8.3")
	}
}

func TestResolveVersionWalkUp_ComposerInParent(t *testing.T) {
	scaffold(t)
	installFakeVersion(t, "8.3")
	installFakeVersion(t, "8.4")
	if err := SetGlobal("8.3"); err != nil {
		t.Fatal(err)
	}

	projDir := t.TempDir()
	subDir := filepath.Join(projDir, "app", "Http")
	if err := os.MkdirAll(subDir, 0755); err != nil {
		t.Fatal(err)
	}
	composer := `{"require": {"php": "^8.3"}}`
	if err := os.WriteFile(filepath.Join(projDir, "composer.json"), []byte(composer), 0644); err != nil {
		t.Fatal(err)
	}

	v, err := ResolveVersionWalkUp(subDir)
	if err != nil {
		t.Fatalf("ResolveVersionWalkUp() error = %v", err)
	}
	if v != "8.4" {
		t.Errorf("ResolveVersionWalkUp() = %q, want %q (highest matching ^8.3)", v, "8.4")
	}
}

func TestResolveVersionWalkUp_PvYmlBeatsComposerHigher(t *testing.T) {
	scaffold(t)
	installFakeVersion(t, "8.3")
	installFakeVersion(t, "8.4")
	if err := SetGlobal("8.4"); err != nil {
		t.Fatal(err)
	}

	// pv.yml in project root, composer.json in a subdirectory.
	// pv.yml should be found first (walking up from subDir hits both dirs,
	// but pv.yml takes priority over composer.json).
	projDir := t.TempDir()
	if err := os.WriteFile(filepath.Join(projDir, "pv.yml"), []byte("php: \"8.3\"\n"), 0644); err != nil {
		t.Fatal(err)
	}
	composer := `{"require": {"php": "^8.4"}}`
	if err := os.WriteFile(filepath.Join(projDir, "composer.json"), []byte(composer), 0644); err != nil {
		t.Fatal(err)
	}

	subDir := filepath.Join(projDir, "src")
	if err := os.MkdirAll(subDir, 0755); err != nil {
		t.Fatal(err)
	}

	v, err := ResolveVersionWalkUp(subDir)
	if err != nil {
		t.Fatalf("ResolveVersionWalkUp() error = %v", err)
	}
	if v != "8.3" {
		t.Errorf("ResolveVersionWalkUp() = %q, want %q (pv.yml should take priority)", v, "8.3")
	}
}

func TestResolveVersionWalkUp_FallsBackToGlobal(t *testing.T) {
	scaffold(t)
	installFakeVersion(t, "8.4")
	if err := SetGlobal("8.4"); err != nil {
		t.Fatal(err)
	}

	dir := t.TempDir()

	v, err := ResolveVersionWalkUp(dir)
	if err != nil {
		t.Fatalf("ResolveVersionWalkUp() error = %v", err)
	}
	if v != "8.4" {
		t.Errorf("ResolveVersionWalkUp() = %q, want %q", v, "8.4")
	}
}

func TestResolveVersionWalkUp_ClosestPvYmlWins(t *testing.T) {
	scaffold(t)
	installFakeVersion(t, "8.3")
	installFakeVersion(t, "8.4")
	if err := SetGlobal("8.4"); err != nil {
		t.Fatal(err)
	}

	// Parent has pv.yml with 8.4, child has pv.yml with 8.3.
	parent := t.TempDir()
	child := filepath.Join(parent, "sub")
	if err := os.MkdirAll(child, 0755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(parent, "pv.yml"), []byte("php: \"8.4\"\n"), 0644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(child, "pv.yml"), []byte("php: \"8.3\"\n"), 0644); err != nil {
		t.Fatal(err)
	}

	v, err := ResolveVersionWalkUp(child)
	if err != nil {
		t.Fatalf("ResolveVersionWalkUp() error = %v", err)
	}
	if v != "8.3" {
		t.Errorf("ResolveVersionWalkUp() = %q, want %q (closest pv.yml should win)", v, "8.3")
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
