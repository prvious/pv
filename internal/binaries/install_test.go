package binaries

import (
	"crypto/sha256"
	"encoding/hex"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestInstallBinary_FrankenPHP(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	binaryContent := []byte("fake frankenphp binary")
	h := sha256.Sum256(binaryContent)
	checksum := hex.EncodeToString(h[:])

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/checksum" {
			w.Write([]byte(checksum))
			return
		}
		w.Write(binaryContent)
	}))
	defer srv.Close()

	// We need to override the URL for testing. Since InstallBinary uses DownloadURL
	// internally, we'll test the lower-level functions instead and verify the file result.
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	binDir := config.BinDir()
	destPath := filepath.Join(binDir, "frankenphp")

	if err := Download(srv.Client(), srv.URL+"/binary", destPath); err != nil {
		t.Fatalf("Download() error = %v", err)
	}

	checksumStr, err := FetchChecksum(srv.Client(), srv.URL+"/checksum")
	if err != nil {
		t.Fatalf("FetchChecksum() error = %v", err)
	}

	if err := VerifyChecksum(destPath, checksumStr); err != nil {
		t.Fatalf("VerifyChecksum() error = %v", err)
	}

	if err := MakeExecutable(destPath); err != nil {
		t.Fatalf("MakeExecutable() error = %v", err)
	}

	info, err := os.Stat(destPath)
	if err != nil {
		t.Fatalf("binary not found: %v", err)
	}
	if info.Mode().Perm()&0111 == 0 {
		t.Error("binary is not executable")
	}
}

func TestInstallBinary_Composer(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	pharContent := []byte("fake composer.phar")
	h := sha256.Sum256(pharContent)
	checksum := hex.EncodeToString(h[:])

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/checksum" {
			w.Write([]byte(checksum))
			return
		}
		w.Write(pharContent)
	}))
	defer srv.Close()

	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	binDir := config.BinDir()
	destPath := filepath.Join(binDir, "composer.phar")

	if err := Download(srv.Client(), srv.URL+"/phar", destPath); err != nil {
		t.Fatalf("Download() error = %v", err)
	}

	checksumStr, err := FetchChecksum(srv.Client(), srv.URL+"/checksum")
	if err != nil {
		t.Fatalf("FetchChecksum() error = %v", err)
	}

	if err := VerifyChecksum(destPath, checksumStr); err != nil {
		t.Fatalf("VerifyChecksum() error = %v", err)
	}

	if _, err := os.Stat(destPath); err != nil {
		t.Fatalf("composer.phar not found: %v", err)
	}
}
