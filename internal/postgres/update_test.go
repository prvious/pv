package postgres

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
	// Pre-populate a "v1" install with a marker file.
	t.Setenv("HOME", t.TempDir())
	dataDir := config.ServiceDataDir("postgres", "17")
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dataDir, "PG_VERSION"), []byte("17"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dataDir, "postgresql.conf"), []byte("# pre-existing\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dataDir, "MARKER"), []byte("DO_NOT_TOUCH"), 0o644); err != nil {
		t.Fatal(err)
	}
	bin := config.PostgresBinDir("17")
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatal(err)
	}
	os.WriteFile(filepath.Join(bin, "postgres"), []byte("v1"), 0o755)

	// Serve a "v2" tarball.
	var buf bytes.Buffer
	gz := gzip.NewWriter(&buf)
	tw := tar.NewWriter(gz)
	hdr := &tar.Header{Name: "bin/postgres", Mode: 0o755, Size: 2, Typeflag: tar.TypeReg}
	tw.WriteHeader(hdr)
	tw.Write([]byte("v2"))
	hdr2 := &tar.Header{Name: "bin/pg_config", Mode: 0o755, Size: 33, Typeflag: tar.TypeReg}
	tw.WriteHeader(hdr2)
	tw.Write([]byte("#!/bin/sh\necho \"PostgreSQL 17.6\"\n"))
	tw.Close()
	gz.Close()

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write(buf.Bytes())
	}))
	defer srv.Close()
	t.Setenv("PV_POSTGRES_URL_OVERRIDE", srv.URL)

	if err := Update(http.DefaultClient, "17"); err != nil {
		t.Fatalf("Update: %v", err)
	}

	// Marker file in data dir should still exist.
	if _, err := os.Stat(filepath.Join(dataDir, "MARKER")); err != nil {
		t.Errorf("data dir clobbered: %v", err)
	}
	// Binary should be the new version.
	got, _ := os.ReadFile(filepath.Join(bin, "postgres"))
	if string(got) != "v2" {
		t.Errorf("binary not updated: got %q", got)
	}
	// state should be wanted=running after update.
	st, _ := LoadState()
	if st.Majors["17"].Wanted != "running" {
		t.Errorf("post-update state.wanted = %q, want running", st.Majors["17"].Wanted)
	}
}
