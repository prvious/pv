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
			if pkg.Method != MethodComposer {
				t.Errorf("laravel.Method = %d, want MethodComposer", pkg.Method)
			}
			if pkg.Composer != "laravel/installer" {
				t.Errorf("laravel.Composer = %q, want %q", pkg.Composer, "laravel/installer")
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

	pkg := Package{Name: "phpstan", Method: MethodPHAR, Asset: "phpstan.phar"}
	got := pkg.PharPath()
	want := filepath.Join(home, ".pv", "internal", "packages", "phpstan.phar")
	if got != want {
		t.Errorf("PharPath() = %q, want %q", got, want)
	}
}

func TestPackageSymlinkPath(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	pkg := Package{Name: "phpstan", Method: MethodPHAR, Asset: "phpstan.phar"}
	got := pkg.SymlinkPath()
	want := filepath.Join(home, ".pv", "bin", "phpstan")
	if got != want {
		t.Errorf("SymlinkPath() = %q, want %q", got, want)
	}
}
