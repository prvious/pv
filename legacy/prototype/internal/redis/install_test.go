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

	version := "8.6"

	if err := Install(http.DefaultClient, version); err != nil {
		t.Fatalf("Install: %v", err)
	}

	for _, want := range []string{"redis-server", "redis-cli"} {
		p := filepath.Join(config.RedisVersionDir(version), want)
		if _, err := os.Stat(p); err != nil {
			t.Errorf("missing %s: %v", want, err)
		}
	}

	if _, err := os.Stat(config.RedisDataDirV(version)); err != nil {
		t.Errorf("data dir missing: %v", err)
	}

	st, _ := LoadState()
	if st.Versions[version].Wanted != WantedRunning {
		t.Errorf("version %s wanted = %q, want %q", version, st.Versions[version].Wanted, WantedRunning)
	}

	vs, _ := binaries.LoadVersions()
	if got := vs.Get("redis-" + version); got == "" {
		t.Errorf("versions.json redis-%s not recorded", version)
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

	version := "8.6"

	if err := Install(http.DefaultClient, version); err != nil {
		t.Fatalf("first Install: %v", err)
	}
	if err := Install(http.DefaultClient, version); err != nil {
		t.Fatalf("second Install (idempotent): %v", err)
	}

	st, _ := LoadState()
	if st.Versions[version].Wanted != WantedRunning {
		t.Errorf("idempotent re-install did not preserve wanted=running")
	}
}
