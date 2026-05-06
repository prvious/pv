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

// makeFakeTarball returns a minimal postgres-like tarball: bin/postgres,
// bin/initdb (a stub that creates PG_VERSION), bin/pg_config (echoes a
// version), share/postgresql/postgresql.conf.sample.
func makeFakeTarball(t *testing.T) []byte {
	t.Helper()
	var buf bytes.Buffer
	gz := gzip.NewWriter(&buf)
	tw := tar.NewWriter(gz)
	add := func(name string, mode int64, body string) {
		hdr := &tar.Header{Name: name, Mode: mode, Size: int64(len(body)), Typeflag: tar.TypeReg}
		if err := tw.WriteHeader(hdr); err != nil {
			t.Fatal(err)
		}
		tw.Write([]byte(body))
	}
	add("bin/postgres", 0o755, "#!/bin/sh\nsleep 60\n")
	add("bin/initdb", 0o755, "#!/bin/sh\nfor a in \"$@\"; do prev=$x; x=$a; if [ \"$prev\" = \"-D\" ]; then mkdir -p \"$x\" && echo 17 > \"$x/PG_VERSION\" && echo \"# stub\" > \"$x/postgresql.conf\"; fi; done\n")
	add("bin/pg_config", 0o755, "#!/bin/sh\necho \"PostgreSQL 17.5\"\n")
	add("share/postgresql/postgresql.conf.sample", 0o644, "# sample\n")
	tw.Close()
	gz.Close()
	return buf.Bytes()
}

func TestInstall_HappyPath(t *testing.T) {
	tarball := makeFakeTarball(t)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/gzip")
		w.Write(tarball)
	}))
	defer srv.Close()

	t.Setenv("HOME", t.TempDir())
	t.Setenv("PV_POSTGRES_URL_OVERRIDE", srv.URL)

	if err := Install(http.DefaultClient, "17"); err != nil {
		t.Fatalf("Install: %v", err)
	}

	// Binaries on disk.
	for _, want := range []string{"bin/postgres", "bin/initdb", "bin/pg_config"} {
		p := filepath.Join(config.PostgresVersionDir("17"), want)
		if _, err := os.Stat(p); err != nil {
			t.Errorf("missing %s: %v", want, err)
		}
	}

	// Data dir initialized.
	if _, err := os.Stat(filepath.Join(config.ServiceDataDir("postgres", "17"), "PG_VERSION")); err != nil {
		t.Errorf("PG_VERSION not created: %v", err)
	}

	// State recorded.
	st, _ := LoadState()
	if st.Majors["17"].Wanted != "running" {
		t.Errorf("state.wanted = %q, want running", st.Majors["17"].Wanted)
	}
}

func TestInstall_AlreadyInstalled_Idempotent(t *testing.T) {
	tarball := makeFakeTarball(t)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write(tarball)
	}))
	defer srv.Close()

	t.Setenv("HOME", t.TempDir())
	t.Setenv("PV_POSTGRES_URL_OVERRIDE", srv.URL)

	if err := Install(http.DefaultClient, "17"); err != nil {
		t.Fatalf("first Install: %v", err)
	}
	if err := Install(http.DefaultClient, "17"); err != nil {
		t.Fatalf("second Install (idempotent): %v", err)
	}
}
