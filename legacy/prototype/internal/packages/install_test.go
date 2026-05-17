package packages

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
)

func TestFetchLatestRelease(t *testing.T) {
	release := gitHubRelease{
		TagName: "v5.3.0",
		Assets: []gitHubAsset{
			{Name: "phpstan.phar", DownloadURL: "https://example.com/phpstan.phar"},
		},
	}

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		json.NewEncoder(w).Encode(release)
	}))
	defer srv.Close()

	client := srv.Client()
	client.Transport = &urlRewriteTransport{base: http.DefaultTransport, testURL: srv.URL}

	pkg := Package{Name: "phpstan", Repo: "phpstan/phpstan", Method: MethodPHAR, Asset: "phpstan.phar"}
	tag, downloadURL, err := fetchLatestRelease(context.Background(), client, pkg)
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

func TestInstallViaPHAR(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := config.EnsureDirs(); err != nil {
		t.Fatalf("EnsureDirs() error = %v", err)
	}

	pharContent := "#!/usr/bin/env php\n<?php echo 'hello';\n"

	var srv *httptest.Server
	srv = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/repos/phpstan/phpstan/releases/latest" {
			release := gitHubRelease{
				TagName: "v2.1.0",
				Assets: []gitHubAsset{
					{Name: "phpstan.phar", DownloadURL: srv.URL + "/download/phpstan.phar"},
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

	pkg := Package{Name: "phpstan", Repo: "phpstan/phpstan", Method: MethodPHAR, Asset: "phpstan.phar"}
	version, err := Install(context.Background(), client, pkg, nil)
	if err != nil {
		t.Fatalf("Install() error = %v", err)
	}
	if version != "v2.1.0" {
		t.Errorf("Install() version = %q, want %q", version, "v2.1.0")
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
	if vs.Versions["phpstan"] != "v2.1.0" {
		t.Errorf("versions.json[phpstan] = %q, want %q", vs.Versions["phpstan"], "v2.1.0")
	}
}

func TestInstallViaComposer(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := config.EnsureDirs(); err != nil {
		t.Fatalf("EnsureDirs() error = %v", err)
	}

	// Mock composer commands.
	orig := runComposer
	t.Cleanup(func() { runComposer = orig })
	runComposer = func(_ context.Context, args ...string) ([]byte, error) {
		if len(args) >= 3 && args[0] == "global" && args[1] == "show" {
			return json.Marshal(map[string]any{
				"versions": []string{"v5.3.0"},
			})
		}
		return []byte(""), nil
	}

	pkg := Package{Name: "laravel", Repo: "laravel/installer", Method: MethodComposer, Composer: "laravel/installer"}
	version, err := Install(context.Background(), nil, pkg, nil)
	if err != nil {
		t.Fatalf("Install() error = %v", err)
	}
	if version != "v5.3.0" {
		t.Errorf("Install() version = %q, want %q", version, "v5.3.0")
	}

	// Verify version was saved.
	loaded, err := binaries.LoadVersions()
	if err != nil {
		t.Fatalf("LoadVersions() error = %v", err)
	}
	if loaded.Get("laravel") != "v5.3.0" {
		t.Errorf("versions.json[laravel] = %q, want %q", loaded.Get("laravel"), "v5.3.0")
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
