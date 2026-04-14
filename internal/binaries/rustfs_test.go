package binaries

import (
	"runtime"
	"strings"
	"testing"
)

func TestRustfsURL_CurrentPlatform(t *testing.T) {
	url, err := rustfsURL("1.0.0-alpha.93")
	if err != nil {
		t.Fatalf("unexpected error for %s/%s: %v", runtime.GOOS, runtime.GOARCH, err)
	}
	if !strings.HasPrefix(url, "https://github.com/rustfs/rustfs/releases/download/1.0.0-alpha.93/") {
		t.Errorf("URL = %q; missing expected prefix", url)
	}
	if !strings.HasSuffix(url, ".zip") {
		t.Errorf("URL = %q; expected .zip suffix", url)
	}
}

func TestRustfsArchiveName_AllPlatforms(t *testing.T) {
	tests := []struct {
		goos, goarch, want string
	}{
		{"darwin", "arm64", "rustfs-macos-aarch64-latest.zip"},
		{"darwin", "amd64", "rustfs-macos-x86_64-latest.zip"},
		{"linux", "amd64", "rustfs-linux-x86_64-gnu-latest.zip"},
		{"linux", "arm64", "rustfs-linux-aarch64-gnu-latest.zip"},
	}
	for _, tc := range tests {
		archMap, ok := rustfsPlatformNames[tc.goos]
		if !ok {
			t.Errorf("no entry for GOOS=%s", tc.goos)
			continue
		}
		platform, ok := archMap[tc.goarch]
		if !ok {
			t.Errorf("no entry for GOARCH=%s on %s", tc.goarch, tc.goos)
			continue
		}
		got := "rustfs-" + platform + "-latest.zip"
		if got != tc.want {
			t.Errorf("%s/%s: got %q, want %q", tc.goos, tc.goarch, got, tc.want)
		}
	}
}

func TestDownloadURL_RustfsCase(t *testing.T) {
	url, err := DownloadURL(Rustfs, "1.0.0-alpha.93")
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
