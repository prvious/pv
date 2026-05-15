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

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
)

// makeFakeMysqlTarball returns a minimal mysql-like tarball: bin/mysqld
// (a stub that, when --initialize-insecure is passed, creates auto.cnf
// at --datadir=...) and bin/mysql (placeholder client). The mysqld stub
// is shell-based so tests don't need to compile a Go binary on every run;
// the real fake-mysqld.go (Step 1) is for tests that need long-run mode.
func makeFakeMysqlTarball(t *testing.T) []byte {
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
	// mysqld stub: parses --datadir= out of any arg, writes auto.cnf, exits.
	// Also handles --version (Task 8 ProbeVersion calls this).
	mysqldStub := `#!/bin/sh
for a in "$@"; do
  case "$a" in
    --version) echo "mysqld  Ver 8.4.3 for macos14 on arm64 (MySQL Community Server - GPL)"; exit 0 ;;
    --datadir=*) d="${a#--datadir=}" ;;
  esac
done
if [ -n "$d" ]; then
  mkdir -p "$d"
  printf '[auto]\nserver-uuid=fake\n' > "$d/auto.cnf"
fi
`
	add("bin/mysqld", 0o755, mysqldStub)
	add("bin/mysql", 0o755, "#!/bin/sh\nexit 0\n")
	add("share/english/errmsg.sys", 0o644, "fake errmsg\n")
	tw.Close()
	gz.Close()
	return buf.Bytes()
}

func TestInstall_HappyPath(t *testing.T) {
	tarball := makeFakeMysqlTarball(t)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/gzip")
		w.Write(tarball)
	}))
	defer srv.Close()

	t.Setenv("HOME", t.TempDir())
	t.Setenv("PV_MYSQL_URL_OVERRIDE", srv.URL)

	if err := Install(http.DefaultClient, "8.4"); err != nil {
		t.Fatalf("Install: %v", err)
	}

	// Binaries on disk.
	for _, want := range []string{"bin/mysqld", "bin/mysql"} {
		p := filepath.Join(config.MysqlVersionDir("8.4"), want)
		if _, err := os.Stat(p); err != nil {
			t.Errorf("missing %s: %v", want, err)
		}
	}

	// Data dir initialized — auto.cnf is the marker.
	if _, err := os.Stat(filepath.Join(config.MysqlDataDir("8.4"), "auto.cnf")); err != nil {
		t.Errorf("auto.cnf not created: %v", err)
	}

	// State recorded as wanted=running.
	st, _ := LoadState()
	if st.Versions["8.4"].Wanted != WantedRunning {
		t.Errorf("state.wanted = %q, want running", st.Versions["8.4"].Wanted)
	}

	// Version recorded in versions.json under key mysql-8.4.
	vs, _ := binaries.LoadVersions()
	if got := vs.Get("mysql-8.4"); got == "" {
		t.Errorf("versions.json mysql-8.4 not recorded")
	}
}

func TestInstall_AlreadyInstalled_Idempotent(t *testing.T) {
	tarball := makeFakeMysqlTarball(t)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write(tarball)
	}))
	defer srv.Close()

	t.Setenv("HOME", t.TempDir())
	t.Setenv("PV_MYSQL_URL_OVERRIDE", srv.URL)

	if err := Install(http.DefaultClient, "8.4"); err != nil {
		t.Fatalf("first Install: %v", err)
	}
	if err := Install(http.DefaultClient, "8.4"); err != nil {
		t.Fatalf("second Install (idempotent): %v", err)
	}

	// State should still be wanted=running after the second install.
	st, _ := LoadState()
	if st.Versions["8.4"].Wanted != WantedRunning {
		t.Errorf("idempotent re-install did not preserve wanted=running")
	}
}
