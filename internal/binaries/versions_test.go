package binaries

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestLoadVersions_NoFile(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	vs, err := LoadVersions()
	if err != nil {
		t.Fatalf("LoadVersions() error = %v", err)
	}
	if len(vs.Versions) != 0 {
		t.Errorf("expected empty versions, got %v", vs.Versions)
	}
}

func TestVersions_SaveLoadRoundTrip(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	vs := &VersionState{Versions: map[string]string{
		"frankenphp": "1.11.3",
		"mago":       "1.13.2",
	}}

	if err := vs.Save(); err != nil {
		t.Fatalf("Save() error = %v", err)
	}

	loaded, err := LoadVersions()
	if err != nil {
		t.Fatalf("LoadVersions() error = %v", err)
	}

	if loaded.Get("frankenphp") != "1.11.3" {
		t.Errorf("frankenphp version = %q, want %q", loaded.Get("frankenphp"), "1.11.3")
	}
	if loaded.Get("mago") != "1.13.2" {
		t.Errorf("mago version = %q, want %q", loaded.Get("mago"), "1.13.2")
	}
}

func TestNeedsUpdate_NoInstalledVersion(t *testing.T) {
	vs := &VersionState{Versions: make(map[string]string)}
	if !NeedsUpdate(vs, Mago, "1.13.2") {
		t.Error("NeedsUpdate() = false, want true for uninstalled binary")
	}
}

func TestNeedsUpdate_SameVersion(t *testing.T) {
	vs := &VersionState{Versions: map[string]string{
		"mago": "1.13.2",
	}}
	if NeedsUpdate(vs, Mago, "1.13.2") {
		t.Error("NeedsUpdate() = true, want false for same version")
	}
}

func TestNeedsUpdate_NormalizesVPrefix(t *testing.T) {
	vs := &VersionState{Versions: map[string]string{
		"mago": "v1.13.2",
	}}
	if NeedsUpdate(vs, Mago, "1.13.2") {
		t.Error("NeedsUpdate() = true, want false after v-prefix normalization")
	}
}

func TestFetchLatestVersion_Composer(t *testing.T) {
	version, err := FetchLatestVersion(http.DefaultClient, Composer)
	if err != nil {
		t.Fatalf("FetchLatestVersion() error = %v", err)
	}
	if version != "latest" {
		t.Errorf("FetchLatestVersion(Composer) = %q, want %q", version, "latest")
	}
}

func TestFetchLatestVersion_GitHub(t *testing.T) {
	release := struct {
		TagName string `json:"tag_name"`
	}{TagName: "v1.11.3"}

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		json.NewEncoder(w).Encode(release)
	}))
	defer srv.Close()

	// Override the URL by using a binary with a mock URL.
	// We test through the function by pointing the client at our test server.
	// For this, we create a custom binary and call FetchLatestVersion with a
	// patched approach — but since FetchLatestVersion uses LatestVersionURL internally,
	// we test it differently: construct the request manually.
	// Instead, test with a real binary but intercept via transport.
	client := srv.Client()
	transport := &urlRewriteTransport{
		base:    http.DefaultTransport,
		testURL: srv.URL,
	}
	client.Transport = transport

	version, err := FetchLatestVersion(client, Mago)
	if err != nil {
		t.Fatalf("FetchLatestVersion() error = %v", err)
	}
	if version != "v1.11.3" {
		t.Errorf("FetchLatestVersion() = %q, want %q", version, "v1.11.3")
	}
}

// urlRewriteTransport redirects all requests to a test server URL.
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
