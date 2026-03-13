package packages

import (
	"path/filepath"
	"testing"
)

func TestManagedRegistryContainsLaravel(t *testing.T) {
	if len(Managed) == 0 {
		t.Fatal("Managed registry is empty")
	}

	found := false
	for _, pkg := range Managed {
		if pkg.Name == "laravel" {
			found = true
			if pkg.Repo != "laravel/installer" {
				t.Errorf("laravel.Repo = %q, want %q", pkg.Repo, "laravel/installer")
			}
			if pkg.Asset != "laravel.phar" {
				t.Errorf("laravel.Asset = %q, want %q", pkg.Asset, "laravel.phar")
			}
		}
	}
	if !found {
		t.Error("laravel not found in Managed registry")
	}
}

func TestPackagePharPath(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	pkg := Managed[0]
	got := pkg.PharPath()
	want := filepath.Join(home, ".pv", "internal", "packages", "laravel.phar")
	if got != want {
		t.Errorf("PharPath() = %q, want %q", got, want)
	}
}

func TestPackageSymlinkPath(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	pkg := Managed[0]
	got := pkg.SymlinkPath()
	want := filepath.Join(home, ".pv", "bin", "laravel")
	if got != want {
		t.Errorf("SymlinkPath() = %q, want %q", got, want)
	}
}
