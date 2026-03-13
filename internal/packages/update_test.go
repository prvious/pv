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

func TestUpdatePHAR_AlreadyUpToDate(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	config.EnsureDirs()

	vs := &binaries.VersionState{Versions: map[string]string{"phpstan": "v2.1.0"}}
	vs.Save()

	var srv *httptest.Server
	srv = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		release := gitHubRelease{
			TagName: "v2.1.0",
			Assets:  []gitHubAsset{{Name: "phpstan.phar", DownloadURL: srv.URL + "/dl"}},
		}
		json.NewEncoder(w).Encode(release)
	}))
	defer srv.Close()

	client := srv.Client()
	client.Transport = &urlRewriteTransport{base: http.DefaultTransport, testURL: srv.URL}

	pkg := Package{Name: "phpstan", Repo: "phpstan/phpstan", Method: MethodPHAR, Asset: "phpstan.phar"}
	updated, version, err := Update(client, pkg)
	if err != nil {
		t.Fatalf("Update() error = %v", err)
	}
	if updated {
		t.Error("Update() updated = true, want false (already up to date)")
	}
	if version != "v2.1.0" {
		t.Errorf("Update() version = %q, want %q", version, "v2.1.0")
	}
}

func TestUpdatePHAR_NewVersionAvailable(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	config.EnsureDirs()

	vs := &binaries.VersionState{Versions: map[string]string{"phpstan": "v2.0.0"}}
	vs.Save()

	os.WriteFile(filepath.Join(config.PackagesDir(), "phpstan.phar"), []byte("old"), 0755)

	pharContent := "#!/usr/bin/env php\n<?php echo 'new';\n"

	var srv *httptest.Server
	srv = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/repos/phpstan/phpstan/releases/latest" {
			release := gitHubRelease{
				TagName: "v2.1.0",
				Assets:  []gitHubAsset{{Name: "phpstan.phar", DownloadURL: srv.URL + "/dl"}},
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
	updated, version, err := Update(client, pkg)
	if err != nil {
		t.Fatalf("Update() error = %v", err)
	}
	if !updated {
		t.Error("Update() updated = false, want true")
	}
	if version != "v2.1.0" {
		t.Errorf("Update() version = %q, want %q", version, "v2.1.0")
	}

	content, _ := os.ReadFile(pkg.PharPath())
	if string(content) != pharContent {
		t.Errorf("PHAR content = %q, want new content", string(content))
	}

	loaded, _ := binaries.LoadVersions()
	if loaded.Get("phpstan") != "v2.1.0" {
		t.Errorf("versions.json[phpstan] = %q, want %q", loaded.Get("phpstan"), "v2.1.0")
	}
}

func TestUpdatePHAR_NormalizesVPrefix(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	config.EnsureDirs()

	vs := &binaries.VersionState{Versions: map[string]string{"phpstan": "2.1.0"}}
	vs.Save()

	var srv *httptest.Server
	srv = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		release := gitHubRelease{
			TagName: "v2.1.0",
			Assets:  []gitHubAsset{{Name: "phpstan.phar", DownloadURL: srv.URL + "/dl"}},
		}
		json.NewEncoder(w).Encode(release)
	}))
	defer srv.Close()

	client := srv.Client()
	client.Transport = &urlRewriteTransport{base: http.DefaultTransport, testURL: srv.URL}

	pkg := Package{Name: "phpstan", Repo: "phpstan/phpstan", Method: MethodPHAR, Asset: "phpstan.phar"}
	updated, _, err := Update(client, pkg)
	if err != nil {
		t.Fatalf("Update() error = %v", err)
	}
	if updated {
		t.Error("Update() updated = true, want false (v-prefix normalization)")
	}
}

func TestUpdateComposer_AlreadyUpToDate(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	config.EnsureDirs()

	vs := &binaries.VersionState{Versions: map[string]string{"laravel": "v5.3.0"}}
	vs.Save()

	orig := runComposer
	t.Cleanup(func() { runComposer = orig })
	runComposer = func(args ...string) ([]byte, error) {
		if len(args) >= 3 && args[0] == "global" && args[1] == "show" {
			return json.Marshal(map[string]any{
				"versions": []string{"v5.3.0"},
			})
		}
		return []byte(""), nil
	}

	pkg := Package{Name: "laravel", Repo: "laravel/installer", Method: MethodComposer, Composer: "laravel/installer"}
	updated, version, err := Update(nil, pkg)
	if err != nil {
		t.Fatalf("Update() error = %v", err)
	}
	if updated {
		t.Error("Update() updated = true, want false")
	}
	if version != "v5.3.0" {
		t.Errorf("Update() version = %q, want %q", version, "v5.3.0")
	}
}

func TestUpdateComposer_NewVersionAvailable(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	config.EnsureDirs()

	vs := &binaries.VersionState{Versions: map[string]string{"laravel": "v5.2.0"}}
	vs.Save()

	orig := runComposer
	t.Cleanup(func() { runComposer = orig })
	runComposer = func(args ...string) ([]byte, error) {
		if len(args) >= 3 && args[0] == "global" && args[1] == "show" {
			return json.Marshal(map[string]any{
				"versions": []string{"v5.3.0"},
			})
		}
		return []byte(""), nil
	}

	pkg := Package{Name: "laravel", Repo: "laravel/installer", Method: MethodComposer, Composer: "laravel/installer"}
	updated, version, err := Update(nil, pkg)
	if err != nil {
		t.Fatalf("Update() error = %v", err)
	}
	if !updated {
		t.Error("Update() updated = false, want true")
	}
	if version != "v5.3.0" {
		t.Errorf("Update() version = %q, want %q", version, "v5.3.0")
	}

	loaded, _ := binaries.LoadVersions()
	if loaded.Get("laravel") != "v5.3.0" {
		t.Errorf("versions.json[laravel] = %q, want %q", loaded.Get("laravel"), "v5.3.0")
	}
}
