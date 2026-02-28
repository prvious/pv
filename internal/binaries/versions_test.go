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
	if !NeedsUpdate(vs, FrankenPHP, "1.11.3") {
		t.Error("NeedsUpdate() = false, want true for uninstalled binary")
	}
}

func TestNeedsUpdate_SameVersion(t *testing.T) {
	vs := &VersionState{Versions: map[string]string{
		"frankenphp": "1.11.3",
	}}
	if NeedsUpdate(vs, FrankenPHP, "1.11.3") {
		t.Error("NeedsUpdate() = true, want false for same version")
	}
}

func TestNeedsUpdate_NormalizesVPrefix(t *testing.T) {
	vs := &VersionState{Versions: map[string]string{
		"frankenphp": "v1.11.3",
	}}
	if NeedsUpdate(vs, FrankenPHP, "1.11.3") {
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

	version, err := FetchLatestVersion(client, FrankenPHP)
	if err != nil {
		t.Fatalf("FetchLatestVersion() error = %v", err)
	}
	if version != "v1.11.3" {
		t.Errorf("FetchLatestVersion() = %q, want %q", version, "v1.11.3")
	}
}

func TestParseFrankenPHPPhpVersion(t *testing.T) {
	tests := []struct {
		name    string
		input   string
		want    string
		wantErr bool
	}{
		{
			name:  "standard output",
			input: "FrankenPHP v1.11.3 PHP 8.5.3 Caddy/v2.9.1 h1:abc",
			want:  "8.5.3",
		},
		{
			name:  "different version",
			input: "FrankenPHP v1.4.0 PHP 8.4.2 Caddy/v2.8.4 h1:xyz",
			want:  "8.4.2",
		},
		{
			name:  "multiline output",
			input: "FrankenPHP v1.11.3 PHP 8.5.3 Caddy/v2.9.1 h1:abc\nsome other line",
			want:  "8.5.3",
		},
		{
			name:    "no PHP version",
			input:   "FrankenPHP v1.11.3 Caddy/v2.9.1",
			wantErr: true,
		},
		{
			name:    "empty input",
			input:   "",
			wantErr: true,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := ParseFrankenPHPPhpVersion(tt.input)
			if tt.wantErr {
				if err == nil {
					t.Errorf("ParseFrankenPHPPhpVersion() error = nil, want error")
				}
				return
			}
			if err != nil {
				t.Fatalf("ParseFrankenPHPPhpVersion() error = %v", err)
			}
			if got != tt.want {
				t.Errorf("ParseFrankenPHPPhpVersion() = %q, want %q", got, tt.want)
			}
		})
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
