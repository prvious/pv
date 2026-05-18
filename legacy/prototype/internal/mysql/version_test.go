package mysql

import (
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"testing"

	"github.com/prvious/pv/internal/config"
)

// buildFakeMysqld compiles testdata/fake-mysqld.go into binDir/mysqld.
func buildFakeMysqld(t *testing.T, binDir string) {
	t.Helper()
	if err := os.MkdirAll(binDir, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	src := filepath.Join("testdata", "fake-mysqld.go")
	dst := filepath.Join(binDir, "mysqld")
	cmd := exec.Command("go", "build", "-o", dst, src)
	cmd.Env = append(os.Environ(),
		"GOOS="+runtime.GOOS,
		"GOARCH="+runtime.GOARCH,
	)
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("go build fake-mysqld: %v\n%s", err, out)
	}
}

func TestProbeVersion(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	buildFakeMysqld(t, config.MysqlBinDir("8.4"))
	got, err := ProbeVersion("8.4")
	if err != nil {
		t.Fatalf("ProbeVersion: %v", err)
	}
	if got != "8.4.9" {
		t.Errorf("ProbeVersion = %q, want 8.4.9", got)
	}
}

func TestProbeVersion_Missing(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if _, err := ProbeVersion("8.4"); err == nil {
		t.Error("ProbeVersion should error when binaries are missing")
	}
}

func TestParseMysqldVersion(t *testing.T) {
	tests := []struct {
		in   string
		want string
		ok   bool
	}{
		{"mysqld  Ver 8.4.9 for macos15 on arm64 (MySQL Community Server - GPL)", "8.4.9", true},
		{"mysqld  Ver 9.7.0 for macos15 on arm64 (MySQL Community Server - GPL)", "9.7.0", true},
		{"mysqld  Ver 8.0.43 for macos15 on arm64 (MySQL Community Server - GPL)", "8.0.43", true},
		// Tab-separated layouts seen on some homebrew builds — must still parse.
		{"mysqld\tVer 8.4.9 for macos15 on arm64", "8.4.9", true},
		{"random garbage line", "", false},
		{"", "", false},
	}
	for _, tt := range tests {
		got, err := parseMysqldVersion(tt.in)
		if tt.ok && err != nil {
			t.Errorf("parseMysqldVersion(%q) err: %v", tt.in, err)
			continue
		}
		if !tt.ok && err == nil {
			t.Errorf("parseMysqldVersion(%q) expected error", tt.in)
			continue
		}
		if got != tt.want {
			t.Errorf("parseMysqldVersion(%q) = %q, want %q", tt.in, got, tt.want)
		}
	}
}
