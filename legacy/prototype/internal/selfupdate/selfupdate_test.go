package selfupdate

import (
	"fmt"
	"net/http"
	"net/http/httptest"
	"runtime"
	"testing"
)

func TestNeedsUpdate_NewerVersion(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		fmt.Fprint(w, `{"tag_name":"v2.0.0"}`)
	}))
	defer srv.Close()

	origURL := githubAPIURL
	githubAPIURL = srv.URL + "/"
	defer func() { githubAPIURL = origURL }()

	latest, needed, err := NeedsUpdate(srv.Client(), "1.0.0")
	if err != nil {
		t.Fatalf("NeedsUpdate() error = %v", err)
	}
	if latest != "2.0.0" {
		t.Errorf("latest = %q, want %q", latest, "2.0.0")
	}
	if !needed {
		t.Error("expected update needed")
	}
}

func TestNeedsUpdate_SameVersion(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		fmt.Fprint(w, `{"tag_name":"v1.0.0"}`)
	}))
	defer srv.Close()

	origURL := githubAPIURL
	githubAPIURL = srv.URL + "/"
	defer func() { githubAPIURL = origURL }()

	_, needed, err := NeedsUpdate(srv.Client(), "1.0.0")
	if err != nil {
		t.Fatalf("NeedsUpdate() error = %v", err)
	}
	if needed {
		t.Error("expected no update needed for same version")
	}
}

func TestNeedsUpdate_DevVersion(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		fmt.Fprint(w, `{"tag_name":"v2.0.0"}`)
	}))
	defer srv.Close()

	origURL := githubAPIURL
	githubAPIURL = srv.URL + "/"
	defer func() { githubAPIURL = origURL }()

	_, needed, err := NeedsUpdate(srv.Client(), "dev")
	if err != nil {
		t.Fatalf("NeedsUpdate() error = %v", err)
	}
	if needed {
		t.Error("expected no update for dev version")
	}
}

func TestNeedsUpdate_EmptyVersion(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		fmt.Fprint(w, `{"tag_name":"v2.0.0"}`)
	}))
	defer srv.Close()

	origURL := githubAPIURL
	githubAPIURL = srv.URL + "/"
	defer func() { githubAPIURL = origURL }()

	_, needed, err := NeedsUpdate(srv.Client(), "")
	if err != nil {
		t.Fatalf("NeedsUpdate() error = %v", err)
	}
	if needed {
		t.Error("expected no update for empty version")
	}
}

func TestNeedsUpdate_VPrefixNormalization(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		fmt.Fprint(w, `{"tag_name":"v1.0.0"}`)
	}))
	defer srv.Close()

	origURL := githubAPIURL
	githubAPIURL = srv.URL + "/"
	defer func() { githubAPIURL = origURL }()

	_, needed, err := NeedsUpdate(srv.Client(), "v1.0.0")
	if err != nil {
		t.Fatalf("NeedsUpdate() error = %v", err)
	}
	if needed {
		t.Error("v1.0.0 should match v1.0.0 after normalization")
	}
}

func TestDownloadURL(t *testing.T) {
	url := downloadURL("1.2.3")
	expected := fmt.Sprintf("https://github.com/prvious/pv/releases/download/v1.2.3/pv-%s-%s", runtime.GOOS, runtime.GOARCH)
	if url != expected {
		t.Errorf("downloadURL(1.2.3) = %q, want %q", url, expected)
	}

	// Should strip v prefix.
	url2 := downloadURL("v1.2.3")
	if url2 != expected {
		t.Errorf("downloadURL(v1.2.3) = %q, want %q", url2, expected)
	}
}

func TestPlatformString(t *testing.T) {
	result := platformString()
	expected := fmt.Sprintf("%s-%s", runtime.GOOS, runtime.GOARCH)
	if result != expected {
		t.Errorf("platformString() = %q, want %q", result, expected)
	}
}
