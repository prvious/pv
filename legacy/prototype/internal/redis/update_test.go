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

	version := "8.6"

	if err := Install(http.DefaultClient, version); err != nil {
		t.Fatalf("Install: %v", err)
	}

	dataFile := filepath.Join(config.RedisDataDirV(version), "dump.rdb")
	if err := os.WriteFile(dataFile, []byte("preserve-me"), 0o644); err != nil {
		t.Fatal(err)
	}

	if err := Update(http.DefaultClient, version); err != nil {
		t.Fatalf("Update: %v", err)
	}

	if got, err := os.ReadFile(dataFile); err != nil {
		t.Fatalf("data file gone: %v", err)
	} else if string(got) != "preserve-me" {
		t.Errorf("data file mutated: %q", got)
	}

	st, _ := LoadState()
	if st.Versions[version].Wanted != WantedRunning {
		t.Errorf("state should have version %s marked wanted=running after Update", version)
	}
}

func TestUpdate_NotInstalledErrors(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if err := Update(http.DefaultClient, "8.6"); err == nil {
		t.Error("Update should error when redis is not installed")
	}
}
