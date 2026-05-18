package binaries

import (
	"archive/zip"
	"os"
	"path/filepath"
	"testing"
)

func TestExtractZip_FlattensSingleBinary(t *testing.T) {
	tmp := t.TempDir()
	zipPath := filepath.Join(tmp, "test.zip")
	f, err := os.Create(zipPath)
	if err != nil {
		t.Fatal(err)
	}
	w := zip.NewWriter(f)
	fw, err := w.Create("nested/dir/rustfs")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := fw.Write([]byte("#!/bin/sh\necho hi\n")); err != nil {
		t.Fatal(err)
	}
	if err := w.Close(); err != nil {
		t.Fatal(err)
	}
	if err := f.Close(); err != nil {
		t.Fatal(err)
	}

	destPath := filepath.Join(tmp, "out", "rustfs")
	if err := os.MkdirAll(filepath.Dir(destPath), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := ExtractZip(zipPath, destPath, "rustfs"); err != nil {
		t.Fatal(err)
	}

	info, err := os.Stat(destPath)
	if err != nil {
		t.Fatalf("expected %s to exist: %v", destPath, err)
	}
	if info.Mode().Perm()&0o100 == 0 {
		t.Errorf("expected %s to be executable, got mode %v", destPath, info.Mode())
	}
}

func TestExtractZip_MissingBinary(t *testing.T) {
	tmp := t.TempDir()
	zipPath := filepath.Join(tmp, "test.zip")
	f, _ := os.Create(zipPath)
	w := zip.NewWriter(f)
	fw, _ := w.Create("something-else")
	_, _ = fw.Write([]byte("x"))
	w.Close()
	f.Close()

	err := ExtractZip(zipPath, filepath.Join(tmp, "out", "rustfs"), "rustfs")
	if err == nil {
		t.Fatal("expected error when binary not found in zip")
	}
}
