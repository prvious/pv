package mysql

import (
	"archive/tar"
	"bytes"
	"compress/gzip"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestUpdate_LeavesDataDirIntact(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	// Pre-populate a "v1" install with a marker file in the data dir.
	dataDir := config.MysqlDataDir("8.4")
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dataDir, "auto.cnf"), []byte("[auto]\nserver-uuid=v1\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dataDir, "MARKER"), []byte("DO_NOT_TOUCH"), 0o644); err != nil {
		t.Fatal(err)
	}
	bin := config.MysqlBinDir("8.4")
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatal(err)
	}
	os.WriteFile(filepath.Join(bin, "mysqld"), []byte("v1"), 0o755)

	// Pre-set state to wanted=running so we can verify it's preserved.
	if err := SetWanted("8.4", WantedRunning); err != nil {
		t.Fatal(err)
	}

	// Serve a "v2" tarball.
	var buf bytes.Buffer
	gz := gzip.NewWriter(&buf)
	tw := tar.NewWriter(gz)
	hdr := &tar.Header{Name: "bin/mysqld", Mode: 0o755, Size: 2, Typeflag: tar.TypeReg}
	tw.WriteHeader(hdr)
	tw.Write([]byte("v2"))
	tw.Close()
	gz.Close()

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write(buf.Bytes())
	}))
	defer srv.Close()
	t.Setenv("PV_MYSQL_URL_OVERRIDE", srv.URL)

	if err := Update(http.DefaultClient, "8.4"); err != nil {
		t.Fatalf("Update: %v", err)
	}

	// Marker file in data dir should still exist.
	if _, err := os.Stat(filepath.Join(dataDir, "MARKER")); err != nil {
		t.Errorf("data dir clobbered: %v", err)
	}
	// Binary should be the new version.
	got, _ := os.ReadFile(filepath.Join(bin, "mysqld"))
	if string(got) != "v2" {
		t.Errorf("binary not updated: got %q", got)
	}
	// State should be wanted=running after update (was running before).
	st, _ := LoadState()
	if st.Versions["8.4"].Wanted != WantedRunning {
		t.Errorf("post-update state.wanted = %q, want running", st.Versions["8.4"].Wanted)
	}
}

func TestUpdate_NotInstalled_Errors(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if err := Update(http.DefaultClient, "8.4"); err == nil {
		t.Error("expected error updating a non-installed version")
	}
}
