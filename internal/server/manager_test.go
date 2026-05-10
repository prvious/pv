package server

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"sync"
	"testing"
	"time"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/mysql"
	"github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/supervisor"
)

// fakeBinaryBuildOnce caches the compiled fake-binary path across tests in
// this package — go build is expensive and the helper is stateless.
var (
	fakeBinaryPathOnce sync.Once
	fakeBinaryPath     string
	fakeBinaryErr      error
)

// compiledFakeBinary compiles testdata/fakebinary/main.go once per test run
// and returns the absolute path to the resulting executable. The binary
// binds a TCP port on 127.0.0.1 and sleeps — matching what the RustFS
// supervisor expects for its TCP ready-check.
func compiledFakeBinary(t *testing.T) string {
	t.Helper()
	fakeBinaryPathOnce.Do(func() {
		dir, err := os.MkdirTemp("", "pv-fake-binary-*")
		if err != nil {
			fakeBinaryErr = err
			return
		}
		// Keep this around for the life of the test process; the OS will
		// clean it up or leave it in the user's tmp — either is fine.
		out := filepath.Join(dir, "fakebinary")
		src := filepath.Join("testdata", "fakebinary", "main.go")
		cmd := exec.Command("go", "build", "-o", out, src)
		if output, err := cmd.CombinedOutput(); err != nil {
			fakeBinaryErr = err
			t.Logf("go build output: %s", output)
			return
		}
		fakeBinaryPath = out
	})
	if fakeBinaryErr != nil {
		t.Fatalf("compile fake binary: %v", fakeBinaryErr)
	}
	return fakeBinaryPath
}

// stageFakeBinaryAsRustfs copies the compiled fake binary into
// ~/.pv/internal/bin/rustfs so the supervisor finds it via the normal path.
func stageFakeBinaryAsRustfs(t *testing.T) {
	t.Helper()
	src := compiledFakeBinary(t)
	binDir := config.InternalBinDir()
	if err := os.MkdirAll(binDir, 0o755); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(src)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(binDir, "rustfs"), data, 0o755); err != nil {
		t.Fatal(err)
	}
}

func TestReconcile_SpawnsBinaryServices(t *testing.T) {
	if runtime.GOOS != "darwin" && runtime.GOOS != "linux" {
		t.Skipf("fake binary helper not supported on %s", runtime.GOOS)
	}
	t.Setenv("HOME", t.TempDir())

	// Seed a registry with s3 as a binary service.
	enabled := true
	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{
			"s3": {Port: 9000, ConsolePort: 9001, Enabled: &enabled},
		},
	}
	if err := reg.Save(); err != nil {
		t.Fatal(err)
	}

	stageFakeBinaryAsRustfs(t)

	sup := supervisor.New()
	m := &ServerManager{supervisor: sup, secondaries: map[string]*FrankenPHP{}}
	defer m.supervisor.StopAll(2 * time.Second)

	if err := m.reconcileBinaryServices(context.Background()); err != nil {
		t.Fatalf("reconcileBinaryServices: %v", err)
	}
	if !sup.IsRunning("rustfs") {
		t.Error("expected rustfs to be supervised after reconcile")
	}
}

func TestReconcile_StopsDisabledBinaryServices(t *testing.T) {
	if runtime.GOOS != "darwin" && runtime.GOOS != "linux" {
		t.Skipf("fake binary helper not supported on %s", runtime.GOOS)
	}
	t.Setenv("HOME", t.TempDir())
	stageFakeBinaryAsRustfs(t)

	sup := supervisor.New()
	m := &ServerManager{supervisor: sup, secondaries: map[string]*FrankenPHP{}}
	defer sup.StopAll(2 * time.Second)

	// Phase 1: enabled, should start.
	enabled := true
	reg1 := &registry.Registry{Services: map[string]*registry.ServiceInstance{
		"s3": {Port: 9000, Enabled: &enabled},
	}}
	if err := reg1.Save(); err != nil {
		t.Fatal(err)
	}
	if err := m.reconcileBinaryServices(context.Background()); err != nil {
		t.Fatal(err)
	}
	if !sup.IsRunning("rustfs") {
		t.Fatal("expected rustfs running after first reconcile")
	}

	// Phase 2: disabled, should stop.
	disabled := false
	reg2 := &registry.Registry{Services: map[string]*registry.ServiceInstance{
		"s3": {Port: 9000, Enabled: &disabled},
	}}
	if err := reg2.Save(); err != nil {
		t.Fatal(err)
	}
	if err := m.reconcileBinaryServices(context.Background()); err != nil {
		t.Fatal(err)
	}
	if sup.IsRunning("rustfs") {
		t.Error("expected rustfs stopped after disabling via reconcile")
	}
}

func TestReconcileBinaryServices_StartsWantedPostgres(t *testing.T) {
	if runtime.GOOS != "darwin" && runtime.GOOS != "linux" {
		t.Skipf("fake binary helper not supported on %s", runtime.GOOS)
	}
	t.Setenv("HOME", t.TempDir())

	bin := config.PostgresBinDir("17")
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatal(err)
	}
	cmd := exec.Command("go", "build", "-o", filepath.Join(bin, "postgres"),
		filepath.Join("..", "..", "internal", "postgres", "testdata", "fake-postgres-server.go"))
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("build fake postgres: %v\n%s", err, out)
	}
	dataDir := config.ServiceDataDir("postgres", "17")
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		t.Fatal(err)
	}
	os.WriteFile(filepath.Join(dataDir, "PG_VERSION"), []byte("17"), 0o644)
	os.WriteFile(filepath.Join(dataDir, "postgresql.conf"), []byte("# placeholder\n"), 0o644)
	if err := postgres.WriteOverrides("17"); err != nil {
		t.Fatal(err)
	}
	if err := postgres.SetWanted("17", "running"); err != nil {
		t.Fatal(err)
	}

	sup := supervisor.New()
	mgr := NewServerManager(nil, sup)

	if err := mgr.reconcileBinaryServices(context.Background()); err != nil {
		t.Fatalf("reconcileBinaryServices: %v", err)
	}

	if !sup.IsRunning("postgres-17") {
		t.Error("expected postgres-17 to be supervised after reconcile")
	}
	_ = sup.StopAll(2 * time.Second)
}

func TestReconcileBinaryServices_StartsWantedMysql(t *testing.T) {
	if runtime.GOOS != "darwin" && runtime.GOOS != "linux" {
		t.Skipf("fake binary helper not supported on %s", runtime.GOOS)
	}
	t.Setenv("HOME", t.TempDir())

	// Pre-stage an installed version. The supervisor's TCP ready-check needs
	// a live listener on PortFor(version), so we compile a tiny Go fake that
	// reads --port=N from argv and binds it.
	bin := config.MysqlBinDir("8.4")
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatal(err)
	}
	cmd := exec.Command("go", "build", "-o", filepath.Join(bin, "mysqld"),
		filepath.Join("..", "..", "internal", "mysql", "testdata", "fake-mysqld.go"))
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("build fake mysqld: %v\n%s", err, out)
	}

	// Datadir + auto.cnf marker — BuildSupervisorProcess refuses to build
	// without it (datadir-not-initialized guard).
	dataDir := config.MysqlDataDir("8.4")
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dataDir, "auto.cnf"), []byte("[auto]\nserver-uuid=00000000-0000-0000-0000-000000000000\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	if err := mysql.SetWanted("8.4", mysql.WantedRunning); err != nil {
		t.Fatal(err)
	}

	sup := supervisor.New()
	mgr := NewServerManager(nil, sup)

	if err := mgr.reconcileBinaryServices(context.Background()); err != nil {
		t.Fatalf("reconcileBinaryServices: %v", err)
	}

	if !sup.IsRunning("mysql-8.4") {
		t.Error("expected mysql-8.4 to be supervised after reconcile")
	}
	_ = sup.StopAll(2 * time.Second)
}

func TestReconcileBinaryServices_StopsRemovedMysql(t *testing.T) {
	if runtime.GOOS != "darwin" && runtime.GOOS != "linux" {
		t.Skipf("fake binary helper not supported on %s", runtime.GOOS)
	}
	t.Setenv("HOME", t.TempDir())

	bin := config.MysqlBinDir("8.4")
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatal(err)
	}
	cmd := exec.Command("go", "build", "-o", filepath.Join(bin, "mysqld"),
		filepath.Join("..", "..", "internal", "mysql", "testdata", "fake-mysqld.go"))
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("build fake mysqld: %v\n%s", err, out)
	}
	dataDir := config.MysqlDataDir("8.4")
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		t.Fatal(err)
	}
	os.WriteFile(filepath.Join(dataDir, "auto.cnf"), []byte("[auto]\nserver-uuid=00000000-0000-0000-0000-000000000000\n"), 0o644)
	if err := mysql.SetWanted("8.4", mysql.WantedRunning); err != nil {
		t.Fatal(err)
	}

	sup := supervisor.New()
	mgr := NewServerManager(nil, sup)
	defer sup.StopAll(2 * time.Second)

	// Phase 1: wanted=running → reconcile starts mysql-8.4.
	if err := mgr.reconcileBinaryServices(context.Background()); err != nil {
		t.Fatal(err)
	}
	if !sup.IsRunning("mysql-8.4") {
		t.Fatal("expected mysql-8.4 running after first reconcile")
	}

	// Phase 2: flip to stopped → reconcile must stop mysql-8.4.
	if err := mysql.SetWanted("8.4", mysql.WantedStopped); err != nil {
		t.Fatal(err)
	}
	if err := mgr.reconcileBinaryServices(context.Background()); err != nil {
		t.Fatal(err)
	}
	if sup.IsRunning("mysql-8.4") {
		t.Error("expected mysql-8.4 stopped after wanted flipped to stopped")
	}
}

func TestReconcileBinaryServices_StartsWantedRedis(t *testing.T) {
	if runtime.GOOS != "darwin" && runtime.GOOS != "linux" {
		t.Skipf("fake binary helper not supported on %s", runtime.GOOS)
	}
	t.Setenv("HOME", t.TempDir())

	if err := os.MkdirAll(config.RedisDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	cmd := exec.Command("go", "build", "-o", filepath.Join(config.RedisDir(), "redis-server"),
		filepath.Join("..", "..", "internal", "redis", "testdata", "fake-redis-server.go"))
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("build fake redis-server: %v\n%s", err, out)
	}

	if err := redis.SetWanted(redis.WantedRunning); err != nil {
		t.Fatal(err)
	}

	sup := supervisor.New()
	mgr := NewServerManager(nil, sup)
	defer sup.StopAll(2 * time.Second)

	if err := mgr.reconcileBinaryServices(context.Background()); err != nil {
		t.Fatalf("reconcileBinaryServices: %v", err)
	}

	if !sup.IsRunning("redis") {
		t.Error("expected redis to be supervised after reconcile")
	}
}

func TestReconcileBinaryServices_StopsRemovedRedis(t *testing.T) {
	if runtime.GOOS != "darwin" && runtime.GOOS != "linux" {
		t.Skipf("fake binary helper not supported on %s", runtime.GOOS)
	}
	t.Setenv("HOME", t.TempDir())

	if err := os.MkdirAll(config.RedisDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	cmd := exec.Command("go", "build", "-o", filepath.Join(config.RedisDir(), "redis-server"),
		filepath.Join("..", "..", "internal", "redis", "testdata", "fake-redis-server.go"))
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("build fake redis-server: %v\n%s", err, out)
	}
	if err := redis.SetWanted(redis.WantedRunning); err != nil {
		t.Fatal(err)
	}

	sup := supervisor.New()
	mgr := NewServerManager(nil, sup)
	defer sup.StopAll(2 * time.Second)

	if err := mgr.reconcileBinaryServices(context.Background()); err != nil {
		t.Fatal(err)
	}
	if !sup.IsRunning("redis") {
		t.Fatal("expected redis running after first reconcile")
	}

	if err := redis.SetWanted(redis.WantedStopped); err != nil {
		t.Fatal(err)
	}
	if err := mgr.reconcileBinaryServices(context.Background()); err != nil {
		t.Fatal(err)
	}
	if sup.IsRunning("redis") {
		t.Error("expected redis stopped after wanted flipped to stopped")
	}
}
