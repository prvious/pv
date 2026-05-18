package installer

import (
	"os"
	"path/filepath"
	"testing"
)

func TestWriteShimAtomicReplacesExistingExecutable(t *testing.T) {
	path := filepath.Join(t.TempDir(), "bin", "composer")
	if err := WriteShimAtomic(path, []byte("#!/bin/sh\necho old\n")); err != nil {
		t.Fatalf("initial WriteShimAtomic returned error: %v", err)
	}
	if err := WriteShimAtomic(path, []byte("#!/bin/sh\necho new\n")); err != nil {
		t.Fatalf("second WriteShimAtomic returned error: %v", err)
	}

	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("ReadFile returned error: %v", err)
	}
	if string(data) != "#!/bin/sh\necho new\n" {
		t.Fatalf("shim content = %q", data)
	}
	info, err := os.Stat(path)
	if err != nil {
		t.Fatalf("Stat returned error: %v", err)
	}
	if info.Mode().Perm() != 0o755 {
		t.Fatalf("shim mode = %v, want 0755", info.Mode().Perm())
	}

	matches, err := filepath.Glob(filepath.Join(filepath.Dir(path), ".pv-shim-*"))
	if err != nil {
		t.Fatalf("Glob returned error: %v", err)
	}
	if len(matches) != 0 {
		t.Fatalf("temporary shims left behind: %v", matches)
	}
}
