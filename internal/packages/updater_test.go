package packages

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"
	"time"

	"github.com/prvious/pv/internal/config"
)

func TestStartBackgroundUpdater_RunsImmediately(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	config.EnsureDirs()

	var callCount atomic.Int32

	var srv *httptest.Server
	srv = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		callCount.Add(1)
		if r.URL.Path == "/repos/laravel/installer/releases/latest" {
			release := gitHubRelease{
				TagName: "v5.3.0",
				Assets:  []gitHubAsset{{Name: "laravel.phar", DownloadURL: srv.URL + "/dl"}},
			}
			json.NewEncoder(w).Encode(release)
			return
		}
		w.Write([]byte("phar-content"))
	}))
	defer srv.Close()

	client := srv.Client()
	client.Transport = &urlRewriteTransport{base: http.DefaultTransport, testURL: srv.URL}

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	StartBackgroundUpdater(ctx, client, 1*time.Hour)

	// Give the immediate check time to complete.
	time.Sleep(500 * time.Millisecond)

	if callCount.Load() == 0 {
		t.Error("background updater did not run immediately")
	}
}

func TestStartBackgroundUpdater_StopsOnCancel(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	config.EnsureDirs()

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
		w.Write([]byte("phar-content"))
	}))
	defer srv.Close()

	client := srv.Client()
	client.Transport = &urlRewriteTransport{base: http.DefaultTransport, testURL: srv.URL}

	ctx, cancel := context.WithCancel(context.Background())
	StartBackgroundUpdater(ctx, client, 50*time.Millisecond)

	time.Sleep(200 * time.Millisecond)
	cancel()

	// Should not panic or hang after cancel.
	time.Sleep(100 * time.Millisecond)
}
