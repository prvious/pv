package redis

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

// makeFakeRedisTarball returns a minimal redis-like tarball with
// redis-server (a stub that handles --version) and redis-cli.
// Layout: flat — files at the tarball root, no `bin/` subdir, matching
// the Task 1 verification of the artifact layout. If the artifact later
// changes shape, update this helper and Install in lockstep.
func makeFakeRedisTarball(t *testing.T) []byte {
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
	// redis-server stub: handles --version (Task 8 ProbeVersion calls
	// this). For the install path we only need ProbeVersion to succeed
	// — long-run is exercised by process_test.go via the Go fake.
	redisServerStub := `#!/bin/sh
for a in "$@"; do
  case "$a" in
    --version) echo "Redis server v=7.4.1 sha=00000000:0 malloc=libc bits=64 build=stub"; exit 0 ;;
  esac
done
exit 0
`
	add("redis-server", 0o755, redisServerStub)
	add("redis-cli", 0o755, "#!/bin/sh\nexit 0\n")
	tw.Close()
	gz.Close()
	return buf.Bytes()
}

func TestInstall_HappyPath(t *testing.T) {
	tarball := makeFakeRedisTarball(t)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/gzip")
		w.Write(tarball)
	}))
	defer srv.Close()

	t.Setenv("HOME", t.TempDir())
	t.Setenv("PV_REDIS_URL_OVERRIDE", srv.URL)

	if err := Install(http.DefaultClient); err != nil {
		t.Fatalf("Install: %v", err)
	}

	// Binaries on disk.
	for _, want := range []string{"redis-server", "redis-cli"} {
		p := filepath.Join(config.RedisDir(), want)
		if _, err := os.Stat(p); err != nil {
			t.Errorf("missing %s: %v", want, err)
		}
	}

	// Data dir present.
	if _, err := os.Stat(config.RedisDataDir()); err != nil {
		t.Errorf("data dir missing: %v", err)
	}

	// State recorded as wanted=running.
	st, _ := LoadState()
	if st.Wanted != WantedRunning {
		t.Errorf("state.Wanted = %q, want running", st.Wanted)
	}

	// Version recorded in versions.json under key redis.
	vs, _ := binaries.LoadVersions()
	if got := vs.Get("redis"); got == "" {
		t.Errorf("versions.json redis not recorded")
	}
}

func TestInstall_AlreadyInstalled_Idempotent(t *testing.T) {
	tarball := makeFakeRedisTarball(t)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write(tarball)
	}))
	defer srv.Close()

	t.Setenv("HOME", t.TempDir())
	t.Setenv("PV_REDIS_URL_OVERRIDE", srv.URL)

	if err := Install(http.DefaultClient); err != nil {
		t.Fatalf("first Install: %v", err)
	}
	if err := Install(http.DefaultClient); err != nil {
		t.Fatalf("second Install (idempotent): %v", err)
	}

	st, _ := LoadState()
	if st.Wanted != WantedRunning {
		t.Errorf("idempotent re-install did not preserve wanted=running")
	}
}
