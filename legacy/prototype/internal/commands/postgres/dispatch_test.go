package postgres

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func install(t *testing.T, major string) {
	t.Helper()
	bin := config.PostgresBinDir(major)
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(bin, "postgres"), []byte{}, 0o755); err != nil {
		t.Fatal(err)
	}
}

func TestResolveMajor_NoArgs_OneInstalled(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	install(t, "17")
	got, err := resolveMajor(nil)
	if err != nil {
		t.Fatalf("resolveMajor: %v", err)
	}
	if got != "17" {
		t.Errorf("resolveMajor = %q, want 17", got)
	}
}

func TestResolveMajor_NoArgs_NoneInstalled(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if _, err := resolveMajor(nil); err == nil {
		t.Error("expected error when nothing installed")
	}
}

func TestResolveMajor_NoArgs_MultipleInstalled(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	install(t, "17")
	install(t, "18")
	if _, err := resolveMajor(nil); err == nil {
		t.Error("expected error when multiple installed and no arg given")
	}
}

func TestResolveMajor_ExplicitArg(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	install(t, "17")
	install(t, "18")
	got, err := resolveMajor([]string{"17"})
	if err != nil {
		t.Fatalf("resolveMajor: %v", err)
	}
	if got != "17" {
		t.Errorf("resolveMajor = %q, want 17", got)
	}
}

func TestResolveMajor_ExplicitNotInstalled(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	install(t, "17")
	if _, err := resolveMajor([]string{"18"}); err == nil {
		t.Error("expected error when explicit major not installed")
	}
}
