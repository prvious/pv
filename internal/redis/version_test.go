package redis

import (
	"os"
	"os/exec"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestParseRedisVersion(t *testing.T) {
	tests := []struct {
		in   string
		want string
	}{
		{"Redis server v=7.4.1 sha=00000000:0 malloc=libc bits=64 build=abc123", "7.4.1"},
		{"Redis server v=8.0.0 sha=ffffffff:0 malloc=jemalloc bits=64 build=000", "8.0.0"},
		{"  Redis server v=7.2.5 sha=12345678:0 malloc=libc bits=64 build=...  ", "7.2.5"},
	}
	for _, tt := range tests {
		got, err := parseRedisVersion(tt.in)
		if err != nil {
			t.Errorf("parseRedisVersion(%q): %v", tt.in, err)
			continue
		}
		if got != tt.want {
			t.Errorf("parseRedisVersion(%q) = %q, want %q", tt.in, got, tt.want)
		}
	}
}

func TestParseRedisVersion_Invalid(t *testing.T) {
	for _, in := range []string{"", "garbage output", "Redis server but no version"} {
		if _, err := parseRedisVersion(in); err == nil {
			t.Errorf("parseRedisVersion(%q) should error", in)
		}
	}
}

func TestProbeVersion_AgainstFake(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if err := os.MkdirAll(config.RedisDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	bin := filepath.Join(config.RedisDir(), "redis-server")
	cmd := exec.Command("go", "build", "-o", bin,
		filepath.Join("testdata", "fake-redis-server.go"))
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("build fake redis-server: %v\n%s", err, out)
	}
	v, err := ProbeVersion()
	if err != nil {
		t.Fatalf("ProbeVersion: %v", err)
	}
	if v != "7.4.1" {
		t.Errorf("ProbeVersion = %q, want 7.4.1", v)
	}
}
