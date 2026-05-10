package redis

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestBuildSupervisorProcess_NotInstalled(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if _, err := BuildSupervisorProcess(); err == nil {
		t.Error("BuildSupervisorProcess should error when redis is not installed")
	}
}

func TestBuildSupervisorProcess_FlagComposition(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if err := os.MkdirAll(config.RedisDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(config.RedisDir(), "redis-server"), []byte{}, 0o755); err != nil {
		t.Fatal(err)
	}

	proc, err := BuildSupervisorProcess()
	if err != nil {
		t.Fatalf("BuildSupervisorProcess: %v", err)
	}

	if proc.Name != "redis" {
		t.Errorf("Name = %q, want redis", proc.Name)
	}
	if proc.Binary != filepath.Join(config.RedisDir(), "redis-server") {
		t.Errorf("Binary = %q", proc.Binary)
	}
	if proc.LogFile != config.RedisLogPath() {
		t.Errorf("LogFile = %q, want %q", proc.LogFile, config.RedisLogPath())
	}
	got := strings.Join(proc.Args, " ")
	for _, want := range []string{
		"--bind 127.0.0.1",
		"--port 6379",
		"--dir " + config.RedisDataDir(),
		"--dbfilename dump.rdb",
		"--pidfile /tmp/pv-redis.pid",
		"--daemonize no",
		"--protected-mode no",
		"--appendonly no",
	} {
		if !strings.Contains(got, want) {
			t.Errorf("Args missing %q; got: %s", want, got)
		}
	}
	if strings.Contains(got, "--logfile") {
		t.Errorf("Args must NOT contain --logfile (supervisor handles stderr); got: %s", got)
	}
}
