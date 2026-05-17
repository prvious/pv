package redis

import (
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"time"

	"github.com/prvious/pv/internal/config"
)

func TestBuildSupervisorProcess_NotInstalled(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if _, err := BuildSupervisorProcess("8.6"); err == nil {
		t.Error("BuildSupervisorProcess should error when redis is not installed")
	}
}

func TestBuildSupervisorProcess_FlagComposition(t *testing.T) {
	if runtime.GOOS != "darwin" && runtime.GOOS != "linux" {
		t.Skip("test requires exec")
	}
	t.Setenv("HOME", t.TempDir())

	version := "8.6"
	versionDir := config.RedisVersionDir(version)
	os.MkdirAll(versionDir, 0o755)

	out := filepath.Join(versionDir, "redis-server")
	cmd := exec.Command("go", "build", "-o", out,
		filepath.Join("..", "..", "internal", "redis", "testdata", "fake-redis-server.go"))
	if b, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("build fake redis-server: %v\n%s", err, b)
	}

	proc, err := BuildSupervisorProcess(version)
	if err != nil {
		t.Fatalf("BuildSupervisorProcess: %v", err)
	}

	if proc.Name != "redis-8.6" {
		t.Errorf("Name = %q, want redis-8.6", proc.Name)
	}
	if !strings.Contains(proc.Binary, "8.6/redis-server") {
		t.Errorf("Binary = %q, should contain 8.6/redis-server", proc.Binary)
	}
	if !strings.Contains(proc.LogFile, "redis-8.6.log") {
		t.Errorf("LogFile = %q, should contain redis-8.6.log", proc.LogFile)
	}
	if proc.ReadyTimeout != 10*time.Second {
		t.Errorf("ReadyTimeout = %v, want 10s", proc.ReadyTimeout)
	}
}
