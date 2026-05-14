package binaries

import (
	"runtime"
	"testing"
)

func TestRustfsURL(t *testing.T) {
	if runtime.GOOS != "darwin" || runtime.GOARCH != "arm64" {
		t.Skip("rustfs binaries only published for darwin/arm64 in v1")
	}
	t.Setenv("PV_RUSTFS_URL_OVERRIDE", "")
	url, err := RustfsURL("1.0.0-beta")
	if err != nil {
		t.Fatalf("RustfsURL: %v", err)
	}
	want := "https://github.com/prvious/pv/releases/download/artifacts/rustfs-mac-arm64-1.0.0-beta.tar.gz"
	if url != want {
		t.Fatalf("RustfsURL = %q, want %q", url, want)
	}
}

func TestRustfsURL_Override(t *testing.T) {
	t.Setenv("PV_RUSTFS_URL_OVERRIDE", "http://example.test/rustfs.tar.gz")
	url, err := RustfsURL("1.0.0-beta")
	if err != nil {
		t.Fatalf("RustfsURL override: %v", err)
	}
	if url != "http://example.test/rustfs.tar.gz" {
		t.Fatalf("RustfsURL override = %q", url)
	}
}

func TestDownloadURL_RustfsCase(t *testing.T) {
	if runtime.GOOS != "darwin" || runtime.GOARCH != "arm64" {
		t.Skip("rustfs binaries only published for darwin/arm64 in v1")
	}
	url, err := DownloadURL(Rustfs, "1.0.0-beta")
	if err != nil {
		t.Fatalf("DownloadURL returned error: %v", err)
	}
	if url == "" {
		t.Error("DownloadURL returned empty string")
	}
}

func TestLatestVersionURL_RustfsCase(t *testing.T) {
	got := LatestVersionURL(Rustfs)
	want := "https://api.github.com/repos/rustfs/rustfs/releases?per_page=1"
	if got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}
