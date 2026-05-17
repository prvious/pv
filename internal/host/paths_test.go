package host

import (
	"path/filepath"
	"testing"
)

func TestPathsBuildCanonicalFamilies(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	paths, err := NewPaths()
	if err != nil {
		t.Fatalf("NewPaths returned error: %v", err)
	}

	php, err := paths.PHPRuntimeDir("8.4.1")
	if err != nil {
		t.Fatalf("PHPRuntimeDir returned error: %v", err)
	}
	tool, err := paths.ToolDir("composer", "2.9.2")
	if err != nil {
		t.Fatalf("ToolDir returned error: %v", err)
	}
	serviceBin, err := paths.ServiceBinDir("postgres", "18.0")
	if err != nil {
		t.Fatalf("ServiceBinDir returned error: %v", err)
	}
	data, err := paths.DataDir("postgres", "18.0")
	if err != nil {
		t.Fatalf("DataDir returned error: %v", err)
	}
	log, err := paths.LogPath("postgres", "18.0")
	if err != nil {
		t.Fatalf("LogPath returned error: %v", err)
	}

	root := filepath.Join(t.TempDir(), ".pv")
	explicit, err := NewPathsFromRoot(root)
	if err != nil {
		t.Fatalf("NewPathsFromRoot returned error: %v", err)
	}

	for _, path := range []string{
		filepath.Join(paths.BinDir(), "pv"),
		php,
		tool,
		serviceBin,
		data,
		log,
		paths.StateDBPath(),
		paths.CacheArtifactsDir(),
		paths.ConfigDir(),
	} {
		if err := paths.ValidateManagedPath(path); err != nil {
			t.Fatalf("ValidateManagedPath(%q) returned error: %v", path, err)
		}
	}
	if explicit.Root() != root {
		t.Fatalf("explicit root = %q, want %q", explicit.Root(), root)
	}
}

func TestPathsRejectUnsafeSegments(t *testing.T) {
	paths, err := NewPathsFromRoot(filepath.Join(t.TempDir(), ".pv"))
	if err != nil {
		t.Fatalf("NewPathsFromRoot returned error: %v", err)
	}

	for _, version := range []string{"", "../8.4", "8.4/bad", "two words"} {
		if _, err := paths.PHPRuntimeDir(version); err == nil {
			t.Fatalf("PHPRuntimeDir(%q) returned nil error", version)
		}
	}
	for _, name := range []string{"", "../postgres", "post/gres", "two words"} {
		if _, err := paths.ServiceBinDir(name, "18.0"); err == nil {
			t.Fatalf("ServiceBinDir(%q) returned nil error", name)
		}
	}
}

func TestPathsRejectAmbiguousManagedLocations(t *testing.T) {
	root := filepath.Join(t.TempDir(), ".pv")
	paths, err := NewPathsFromRoot(root)
	if err != nil {
		t.Fatalf("NewPathsFromRoot returned error: %v", err)
	}

	for _, path := range []string{
		filepath.Join(root, "bin"),
		filepath.Join(root, "services", "postgres", "18.0", "data", "cluster"),
		filepath.Join(t.TempDir(), "outside"),
	} {
		if err := paths.ValidateManagedPath(path); err == nil {
			t.Fatalf("ValidateManagedPath(%q) returned nil error", path)
		}
	}
}
