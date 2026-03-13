package packages

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestFetchLatestRelease(t *testing.T) {
	release := gitHubRelease{
		TagName: "v5.3.0",
		Assets: []gitHubAsset{
			{Name: "laravel.phar", DownloadURL: "https://example.com/laravel.phar"},
		},
	}

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		json.NewEncoder(w).Encode(release)
	}))
	defer srv.Close()

	client := srv.Client()
	client.Transport = &urlRewriteTransport{base: http.DefaultTransport, testURL: srv.URL}

	pkg := Package{Name: "laravel", Repo: "laravel/installer", Asset: "laravel.phar"}
	tag, downloadURL, err := fetchLatestRelease(client, pkg)
	if err != nil {
		t.Fatalf("fetchLatestRelease() error = %v", err)
	}
	if tag != "v5.3.0" {
		t.Errorf("tag = %q, want %q", tag, "v5.3.0")
	}
	if downloadURL == "" {
		t.Error("downloadURL is empty")
	}
}

func TestInstall(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := config.EnsureDirs(); err != nil {
		t.Fatalf("EnsureDirs() error = %v", err)
	}

	pharContent := "#!/usr/bin/env php\n<?php echo 'hello';\n"

	var srv *httptest.Server
	srv = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/repos/laravel/installer/releases/latest" {
			release := gitHubRelease{
				TagName: "v5.3.0",
				Assets: []gitHubAsset{
					{Name: "laravel.phar", DownloadURL: srv.URL + "/download/laravel.phar"},
				},
			}
			json.NewEncoder(w).Encode(release)
			return
		}
		w.Write([]byte(pharContent))
	}))
	defer srv.Close()

	client := srv.Client()
	client.Transport = &urlRewriteTransport{base: http.DefaultTransport, testURL: srv.URL}

	pkg := Package{Name: "laravel", Repo: "laravel/installer", Asset: "laravel.phar"}
	version, err := Install(client, pkg, nil)
	if err != nil {
		t.Fatalf("Install() error = %v", err)
	}
	if version != "v5.3.0" {
		t.Errorf("Install() version = %q, want %q", version, "v5.3.0")
	}

	// Verify PHAR exists and is executable.
	info, err := os.Stat(pkg.PharPath())
	if err != nil {
		t.Fatalf("PHAR file not found: %v", err)
	}
	if info.Mode()&0111 == 0 {
		t.Error("PHAR is not executable")
	}

	// Verify symlink exists and points to PHAR.
	target, err := os.Readlink(pkg.SymlinkPath())
	if err != nil {
		t.Fatalf("symlink not found: %v", err)
	}
	if target != pkg.PharPath() {
		t.Errorf("symlink target = %q, want %q", target, pkg.PharPath())
	}

	// Verify version was saved.
	data, err := os.ReadFile(filepath.Join(home, ".pv", "data", "versions.json"))
	if err != nil {
		t.Fatalf("versions.json not found: %v", err)
	}
	var vs struct {
		Versions map[string]string `json:"versions"`
	}
	json.Unmarshal(data, &vs)
	if vs.Versions["laravel"] != "v5.3.0" {
		t.Errorf("versions.json[laravel] = %q, want %q", vs.Versions["laravel"], "v5.3.0")
	}
}

// urlRewriteTransport redirects all requests to a test server URL.
// This type is shared across all test files in this package.
type urlRewriteTransport struct {
	base    http.RoundTripper
	testURL string
}

func (t *urlRewriteTransport) RoundTrip(req *http.Request) (*http.Response, error) {
	req = req.Clone(req.Context())
	req.URL.Scheme = "http"
	req.URL.Host = t.testURL[len("http://"):]
	return t.base.RoundTrip(req)
}
