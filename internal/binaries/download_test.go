package binaries

import (
	"archive/tar"
	"bytes"
	"compress/gzip"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestDownload_Success(t *testing.T) {
	content := "hello binary content"
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write([]byte(content))
	}))
	defer srv.Close()

	dest := filepath.Join(t.TempDir(), "downloaded")
	if err := Download(srv.Client(), srv.URL, dest); err != nil {
		t.Fatalf("Download() error = %v", err)
	}

	data, err := os.ReadFile(dest)
	if err != nil {
		t.Fatalf("ReadFile() error = %v", err)
	}
	if string(data) != content {
		t.Errorf("content = %q, want %q", string(data), content)
	}
}

func TestDownload_HTTP404(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusNotFound)
	}))
	defer srv.Close()

	dest := filepath.Join(t.TempDir(), "downloaded")
	err := Download(srv.Client(), srv.URL, dest)
	if err == nil {
		t.Fatal("expected error for HTTP 404, got nil")
	}
	if !strings.Contains(err.Error(), "404") {
		t.Errorf("error = %q, want to contain '404'", err.Error())
	}
}

func TestVerifyChecksum_Match(t *testing.T) {
	content := []byte("test content for checksum")
	f := filepath.Join(t.TempDir(), "file")
	if err := os.WriteFile(f, content, 0644); err != nil {
		t.Fatal(err)
	}

	h := sha256.Sum256(content)
	expected := hex.EncodeToString(h[:])

	if err := VerifyChecksum(f, expected); err != nil {
		t.Fatalf("VerifyChecksum() error = %v", err)
	}
}

func TestVerifyChecksum_Mismatch(t *testing.T) {
	content := []byte("test content")
	f := filepath.Join(t.TempDir(), "file")
	if err := os.WriteFile(f, content, 0644); err != nil {
		t.Fatal(err)
	}

	err := VerifyChecksum(f, "0000000000000000000000000000000000000000000000000000000000000000")
	if err == nil {
		t.Fatal("expected error for checksum mismatch, got nil")
	}
	if !strings.Contains(err.Error(), "checksum mismatch") {
		t.Errorf("error = %q, want to contain 'checksum mismatch'", err.Error())
	}
}

func TestVerifyChecksum_HashFilenameFormat(t *testing.T) {
	content := []byte("hash filename format test")
	f := filepath.Join(t.TempDir(), "file")
	if err := os.WriteFile(f, content, 0644); err != nil {
		t.Fatal(err)
	}

	h := sha256.Sum256(content)
	expected := hex.EncodeToString(h[:]) + "  somefile.bin"

	if err := VerifyChecksum(f, expected); err != nil {
		t.Fatalf("VerifyChecksum() with hash+filename format error = %v", err)
	}
}

func TestFetchChecksum(t *testing.T) {
	checksum := "abc123def456  frankenphp-mac-arm64"
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write([]byte("  " + checksum + "\n"))
	}))
	defer srv.Close()

	got, err := FetchChecksum(srv.Client(), srv.URL)
	if err != nil {
		t.Fatalf("FetchChecksum() error = %v", err)
	}
	if got != checksum {
		t.Errorf("FetchChecksum() = %q, want %q", got, checksum)
	}
}

func createTestTarGz(t *testing.T, files map[string]string) string {
	t.Helper()
	var buf bytes.Buffer
	gw := gzip.NewWriter(&buf)
	tw := tar.NewWriter(gw)

	for name, content := range files {
		hdr := &tar.Header{
			Name:     name,
			Mode:     0755,
			Size:     int64(len(content)),
			Typeflag: tar.TypeReg,
		}
		if err := tw.WriteHeader(hdr); err != nil {
			t.Fatal(err)
		}
		if _, err := tw.Write([]byte(content)); err != nil {
			t.Fatal(err)
		}
	}

	tw.Close()
	gw.Close()

	archivePath := filepath.Join(t.TempDir(), "test.tar.gz")
	if err := os.WriteFile(archivePath, buf.Bytes(), 0644); err != nil {
		t.Fatal(err)
	}
	return archivePath
}

func TestExtractTarGz_Success(t *testing.T) {
	archivePath := createTestTarGz(t, map[string]string{
		"mago-1.13.2/mago":      "mago binary content",
		"mago-1.13.2/README.md": "readme content",
	})

	dest := filepath.Join(t.TempDir(), "mago")
	if err := ExtractTarGz(archivePath, dest, "mago"); err != nil {
		t.Fatalf("ExtractTarGz() error = %v", err)
	}

	data, err := os.ReadFile(dest)
	if err != nil {
		t.Fatalf("ReadFile() error = %v", err)
	}
	if string(data) != "mago binary content" {
		t.Errorf("content = %q, want %q", string(data), "mago binary content")
	}
}

func TestExtractTarGz_BinaryNotFound(t *testing.T) {
	archivePath := createTestTarGz(t, map[string]string{
		"some-dir/other-file": "content",
	})

	dest := filepath.Join(t.TempDir(), "mago")
	err := ExtractTarGz(archivePath, dest, "mago")
	if err == nil {
		t.Fatal("expected error for missing binary, got nil")
	}
	if !errors.Is(err, ErrEntryNotFound) {
		t.Errorf("error = %v, want errors.Is(err, ErrEntryNotFound)", err)
	}
}

func TestMakeExecutable(t *testing.T) {
	f := filepath.Join(t.TempDir(), "binary")
	if err := os.WriteFile(f, []byte("binary"), 0644); err != nil {
		t.Fatal(err)
	}

	if err := MakeExecutable(f); err != nil {
		t.Fatalf("MakeExecutable() error = %v", err)
	}

	info, err := os.Stat(f)
	if err != nil {
		t.Fatal(err)
	}
	if info.Mode().Perm()&0111 == 0 {
		t.Errorf("file mode = %v, want execute bits set", info.Mode())
	}
}

// makeTarGz writes a single-file tarball at archivePath containing entry
// `entryName` with the given content. Used to keep tests hermetic.
func makeTarGz(t *testing.T, archivePath, entryName, content string) {
	t.Helper()
	f, err := os.Create(archivePath)
	if err != nil {
		t.Fatal(err)
	}
	defer f.Close()
	gz := gzip.NewWriter(f)
	tw := tar.NewWriter(gz)
	hdr := &tar.Header{Name: entryName, Mode: 0644, Size: int64(len(content)), Typeflag: tar.TypeReg}
	if err := tw.WriteHeader(hdr); err != nil {
		t.Fatal(err)
	}
	if _, err := tw.Write([]byte(content)); err != nil {
		t.Fatal(err)
	}
	if err := tw.Close(); err != nil {
		t.Fatal(err)
	}
	if err := gz.Close(); err != nil {
		t.Fatal(err)
	}
}

func TestExtractTarGz_EntryNotFound(t *testing.T) {
	dir := t.TempDir()
	archive := filepath.Join(dir, "a.tar.gz")
	makeTarGz(t, archive, "php", "binary content")

	err := ExtractTarGz(archive, filepath.Join(dir, "out"), "php.ini-development")
	if err == nil {
		t.Fatal("ExtractTarGz returned nil for missing entry, want error")
	}
	if !errors.Is(err, ErrEntryNotFound) {
		t.Errorf("ExtractTarGz error = %v, want errors.Is(err, ErrEntryNotFound)", err)
	}
}

func TestExtractTarGz_EntryFound(t *testing.T) {
	dir := t.TempDir()
	archive := filepath.Join(dir, "a.tar.gz")
	makeTarGz(t, archive, "php", "hello")

	dest := filepath.Join(dir, "out")
	if err := ExtractTarGz(archive, dest, "php"); err != nil {
		t.Fatalf("ExtractTarGz error = %v", err)
	}

	got, err := os.ReadFile(dest)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != "hello" {
		t.Errorf("extracted content = %q, want %q", string(got), "hello")
	}
}
