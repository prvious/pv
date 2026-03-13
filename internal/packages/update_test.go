package packages

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
)

func TestUpdate_AlreadyUpToDate(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	config.EnsureDirs()

	vs := &binaries.VersionState{Versions: map[string]string{"laravel": "v5.3.0"}}
	vs.Save()

	var srv *httptest.Server
	srv = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		release := gitHubRelease{
			TagName: "v5.3.0",
			Assets:  []gitHubAsset{{Name: "laravel.phar", DownloadURL: srv.URL + "/dl"}},
		}
		json.NewEncoder(w).Encode(release)
	}))
	defer srv.Close()

	client := srv.Client()
	client.Transport = &urlRewriteTransport{base: http.DefaultTransport, testURL: srv.URL}

	pkg := Package{Name: "laravel", Repo: "laravel/installer", Asset: "laravel.phar"}
	updated, version, err := Update(client, pkg)
	if err != nil {
		t.Fatalf("Update() error = %v", err)
	}
	if updated {
		t.Error("Update() updated = true, want false (already up to date)")
	}
	if version != "v5.3.0" {
		t.Errorf("Update() version = %q, want %q", version, "v5.3.0")
	}
}

func TestUpdate_NewVersionAvailable(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	config.EnsureDirs()

	vs := &binaries.VersionState{Versions: map[string]string{"laravel": "v5.2.0"}}
	vs.Save()

	os.WriteFile(filepath.Join(config.PackagesDir(), "laravel.phar"), []byte("old"), 0755)

	pharContent := "#!/usr/bin/env php\n<?php echo 'new';\n"

	var srv *httptest.Server
	srv = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/repos/laravel/installer/releases/latest" {
			release := gitHubRelease{
				TagName: "v5.3.0",
				Assets:  []gitHubAsset{{Name: "laravel.phar", DownloadURL: srv.URL + "/dl"}},
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
	updated, version, err := Update(client, pkg)
	if err != nil {
		t.Fatalf("Update() error = %v", err)
	}
	if !updated {
		t.Error("Update() updated = false, want true")
	}
	if version != "v5.3.0" {
		t.Errorf("Update() version = %q, want %q", version, "v5.3.0")
	}

	content, _ := os.ReadFile(pkg.PharPath())
	if string(content) != pharContent {
		t.Errorf("PHAR content = %q, want new content", string(content))
	}

	loaded, _ := binaries.LoadVersions()
	if loaded.Get("laravel") != "v5.3.0" {
		t.Errorf("versions.json[laravel] = %q, want %q", loaded.Get("laravel"), "v5.3.0")
	}
}

func TestUpdate_NormalizesVPrefix(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	config.EnsureDirs()

	vs := &binaries.VersionState{Versions: map[string]string{"laravel": "5.3.0"}}
	vs.Save()

	var srv *httptest.Server
	srv = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		release := gitHubRelease{
			TagName: "v5.3.0",
			Assets:  []gitHubAsset{{Name: "laravel.phar", DownloadURL: srv.URL + "/dl"}},
		}
		json.NewEncoder(w).Encode(release)
	}))
	defer srv.Close()

	client := srv.Client()
	client.Transport = &urlRewriteTransport{base: http.DefaultTransport, testURL: srv.URL}

	pkg := Package{Name: "laravel", Repo: "laravel/installer", Asset: "laravel.phar"}
	updated, _, err := Update(client, pkg)
	if err != nil {
		t.Fatalf("Update() error = %v", err)
	}
	if updated {
		t.Error("Update() updated = true, want false (v-prefix normalization)")
	}
}
