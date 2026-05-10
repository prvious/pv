package redis

import (
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestUpdate_ReplacesBinaryTree(t *testing.T) {
	tarball := makeFakeRedisTarball(t)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write(tarball)
	}))
	defer srv.Close()

	t.Setenv("HOME", t.TempDir())
	t.Setenv("PV_REDIS_URL_OVERRIDE", srv.URL)

	if err := Install(http.DefaultClient); err != nil {
		t.Fatalf("Install: %v", err)
	}

	// Drop a sentinel into the data dir; Update must NOT touch it.
	dataFile := filepath.Join(config.RedisDataDir(), "dump.rdb")
	if err := os.WriteFile(dataFile, []byte("preserve-me"), 0o644); err != nil {
		t.Fatal(err)
	}

	if err := Update(http.DefaultClient); err != nil {
		t.Fatalf("Update: %v", err)
	}

	// Data file must still be there.
	if got, err := os.ReadFile(dataFile); err != nil {
		t.Fatalf("data file gone: %v", err)
	} else if string(got) != "preserve-me" {
		t.Errorf("data file mutated: %q", got)
	}

	// State must remain wanted=running.
	st, _ := LoadState()
	if st.Wanted != WantedRunning {
		t.Errorf("state.Wanted = %q after Update, want running", st.Wanted)
	}
}

func TestUpdate_NotInstalledErrors(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if err := Update(http.DefaultClient); err == nil {
		t.Error("Update should error when redis is not installed")
	}
}
