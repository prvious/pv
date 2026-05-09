package mysql

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func install(t *testing.T, version string) {
	t.Helper()
	bin := config.MysqlBinDir(version)
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(bin, "mysqld"), []byte{}, 0o755); err != nil {
		t.Fatal(err)
	}
}

func TestResolveVersion_NoArgs_OneInstalled(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	install(t, "8.4")
	got, err := ResolveVersion(nil)
	if err != nil {
		t.Fatalf("ResolveVersion: %v", err)
	}
	if got != "8.4" {
		t.Errorf("ResolveVersion = %q, want 8.4", got)
	}
}

func TestResolveVersion_NoArgs_NoneInstalled(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if _, err := ResolveVersion(nil); err == nil {
		t.Error("expected error when nothing installed")
	}
}

func TestResolveVersion_NoArgs_MultipleInstalled(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	install(t, "8.4")
	install(t, "9.7")
	if _, err := ResolveVersion(nil); err == nil {
		t.Error("expected error when multiple installed and no arg given")
	}
}

func TestResolveVersion_ExplicitArg(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	install(t, "8.4")
	install(t, "9.7")
	got, err := ResolveVersion([]string{"8.4"})
	if err != nil {
		t.Fatalf("ResolveVersion: %v", err)
	}
	if got != "8.4" {
		t.Errorf("ResolveVersion = %q, want 8.4", got)
	}
}

func TestResolveVersion_ExplicitNotInstalled(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	install(t, "8.4")
	if _, err := ResolveVersion([]string{"9.7"}); err == nil {
		t.Error("expected error when explicit version not installed")
	}
}
