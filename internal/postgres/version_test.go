package postgres

import (
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"testing"

	"github.com/prvious/pv/internal/config"
)

// buildFakePgConfig compiles testdata/fake-pg_config.go into binDir/pg_config.
func buildFakePgConfig(t *testing.T, binDir string) {
	t.Helper()
	if err := os.MkdirAll(binDir, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	src := filepath.Join("testdata", "fake-pg_config.go")
	dst := filepath.Join(binDir, "pg_config")
	cmd := exec.Command("go", "build", "-o", dst, src)
	cmd.Env = append(os.Environ(),
		"GOOS="+runtime.GOOS,
		"GOARCH="+runtime.GOARCH,
	)
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("go build fake-pg_config: %v\n%s", err, out)
	}
}

func TestProbeVersion(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	buildFakePgConfig(t, config.PostgresBinDir("17"))
	got, err := ProbeVersion("17")
	if err != nil {
		t.Fatalf("ProbeVersion: %v", err)
	}
	if got != "17.5" {
		t.Errorf("ProbeVersion = %q, want 17.5", got)
	}
}

func TestProbeVersion_Missing(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if _, err := ProbeVersion("17"); err == nil {
		t.Error("ProbeVersion should error when binaries are missing")
	}
}
