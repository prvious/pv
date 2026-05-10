# Redis Native Binary Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the docker-backed Redis service with a single native binary supervised by pv. One version (whatever the artifacts pipeline ships from upstream `redis/redis`), one port (6379), one data dir (`~/.pv/data/redis/`). No Colima VM, no Docker for redis.

**Architecture:** New `internal/redis/` package mirroring `internal/postgres/` and `internal/mysql/`, but flat (single-record) — no `Versions` map, no version arg in commands, no version disambiguation. Reconciler in `internal/server/manager.go` gains a fourth wanted-set source for redis alongside postgres, mysql, and the existing single-version binary services.

**Tech Stack:** Go 1.24+, cobra (CLI), charm.land/fang+huh+lipgloss/v2 (UI), pv supervisor (process management), pv `internal/state` package (runtime state at `~/.pv/data/state.json`), pv binaries pipeline (artifact download).

---

## File Structure

| Path | Action | Responsibility |
|------|--------|---------------|
| `internal/state/state.go` | Already exists | Generic per-service state file at `~/.pv/data/state.json`. Redis wraps it via `internal/redis/state.go` under key `"redis"`. |
| `internal/config/paths.go` | Modify | Add `RedisDir()`, `RedisDataDir()`, `RedisLogPath()` helpers. Register `RedisDir()` and `RedisDataDir()` in `EnsureDirs()`. |
| `internal/config/paths_test.go` | Modify | Add tests for the new redis path helpers. |
| `internal/binaries/redis.go` | Create | `RedisURL() (string, error)` returning the rolling-release URL with `PV_REDIS_URL_OVERRIDE` env override. Single version — no `IsValidRedisVersion`. |
| `internal/binaries/redis_test.go` | Create | URL construction + override env var. |
| `internal/redis/port.go` | Create | `PortFor() int` — returns the constant 6379. |
| `internal/redis/port_test.go` | Create | Asserts the constant. |
| `internal/redis/installed.go` | Create | `IsInstalled() bool` checks `RedisDir()/redis-server`. |
| `internal/redis/installed_test.go` | Create | Filesystem stat tests. |
| `internal/redis/state.go` | Create | Wraps `internal/state` with key `"redis"`. Sub-record `State{Wanted string}` (flat; no versions map). `LoadState`/`SaveState`/`SetWanted`/`RemoveState`. |
| `internal/redis/state_test.go` | Create | Round-trip; invalid wanted rejected; RemoveState. |
| `internal/redis/wanted.go` | Create | `IsWanted() bool` — true iff installed AND state's `Wanted == WantedRunning`. Drift case warns once. |
| `internal/redis/wanted_test.go` | Create | Intersection rules + missing-binary-with-stale-state warning. |
| `internal/redis/version.go` | Create | `ProbeVersion()` runs `redis-server --version`, parses `Redis server v=X.Y.Z ...`. |
| `internal/redis/version_test.go` | Create | Parser tests against real-world output. |
| `internal/redis/testdata/fake-redis-server.go` | Create | Go `main` test fake — `--version` mode + long-run mode (binds `--port` on 127.0.0.1 until SIGTERM). |
| `internal/redis/privileges.go` | Create | `chownToTarget`, `dropCredential`, `dropSysProcAttr`. Identical shape to `internal/mysql/privileges.go`. |
| `internal/redis/waitstopped.go` | Create | `WaitStopped(timeout)` polls 127.0.0.1:6379 until refused. |
| `internal/redis/install.go` | Create | `Install(client) error` — orchestrates download → extract → atomic rename → chown → version-record → state-update. No init step (redis has no initdb equivalent). |
| `internal/redis/install_test.go` | Create | End-to-end install path against a fake tarball; idempotent re-install. |
| `internal/redis/uninstall.go` | Create | `Uninstall(force bool) error` — stop, remove binaries, remove log, optionally remove datadir, drop state, drop versions.json entry, `reg.UnbindService("redis")`. |
| `internal/redis/uninstall_test.go` | Create | Force vs non-force; datadir kept by default. |
| `internal/redis/update.go` | Create | `Update(client) error` — stop, redownload (atomic), restore wanted=running if was running. Datadir untouched. |
| `internal/redis/update_test.go` | Create | Atomic-rename behavior; datadir untouched. |
| `internal/redis/envvars.go` | Create | `EnvVars(projectName) map[string]string` returns `REDIS_*`. `projectName` arg is unused (kept for parallel signature with mysql/postgres). |
| `internal/redis/envvars_test.go` | Create | Golden test for the map. |
| `internal/redis/process.go` | Create | `BuildSupervisorProcess() (supervisor.Process, error)`. Boot flags: `--bind 127.0.0.1`, `--port 6379`, `--dir`, `--dbfilename dump.rdb`, `--pidfile`, `--daemonize no`, `--protected-mode no`, `--appendonly no`. |
| `internal/redis/process_test.go` | Create | Refuses missing binary; flag composition; LogFile path. |
| `internal/redis/database.go` | Create | `BindLinkedProjects() error` — walks Laravel projects, sets `Services.Redis = true`, writes envvars. |
| `internal/redis/database_test.go` | Create | Walks projects; binds Laravel + laravel-octane; skips other types. |
| `internal/server/manager.go` | Modify | `reconcileBinaryServices` gains a fourth wanted-set source: `redis.IsWanted()` + `redis.BuildSupervisorProcess()`. Supervisor key `"redis"`. |
| `internal/server/manager_test.go` | Modify | Reconcile picks up redis from `IsWanted()`; stops on transition to wanted=stopped. |
| `internal/commands/redis/register.go` | Create | `Register(parent)` wires the `redis:*` group; exports `RunInstall(args)`, `RunUpdate(args)`, `RunUninstall(args)`, `UninstallForce()`. |
| `internal/commands/redis/install.go` | Create | `redis:install` cobra command — no version arg. Auto-binds Laravel projects. |
| `internal/commands/redis/uninstall.go` | Create | `redis:uninstall [--force]` cobra command. |
| `internal/commands/redis/update.go` | Create | `redis:update` cobra command. |
| `internal/commands/redis/start.go` | Create | `redis:start`. |
| `internal/commands/redis/stop.go` | Create | `redis:stop`. |
| `internal/commands/redis/restart.go` | Create | `redis:restart`. |
| `internal/commands/redis/list.go` | Create | `redis:list` — single-row table. |
| `internal/commands/redis/logs.go` | Create | `redis:logs [-f]`. |
| `internal/commands/redis/status.go` | Create | `redis:status`. |
| `internal/commands/redis/download.go` | Create | `redis:download` (hidden). |
| `cmd/redis.go` | Create | Bridge: `init() { redis.Register(rootCmd) }` + adds the `redis` group. |
| `internal/laravel/env.go` | Modify | Add `UpdateProjectEnvForRedis(projectPath, projectName string, bound *registry.ProjectServices) error`. |
| `internal/laravel/env_test.go` | Modify | Test the helper. |
| `internal/laravel/steps.go` | Modify | `DetectServicesStep` calls `UpdateProjectEnvForRedis` when `Services.Redis == true`. |
| `internal/automation/steps/detect_services.go` | Modify | Auto-bind redis on every Laravel project unconditionally when `redis.IsInstalled()` (not via `.env` heuristic). |
| `internal/automation/steps/detect_services_test.go` | Modify | Update redis-binding test fixtures. |
| `internal/services/redis.go` | Delete | Old docker `Redis` struct. |
| `internal/services/redis_test.go` | Delete | Tests for the deleted struct. |
| `internal/services/service.go` | Modify | Drop `"redis": &Redis{}` from the docker `registry` map (now empty). |
| `internal/services/lookup_test.go` | Modify | Drop redis-specific cases (or migrate to assert `LookupAny("redis")` errors). |
| `internal/services/service_test.go` | Modify | Drop redis from the docker-registry assertions; expect empty docker map. |
| `cmd/install.go` | Modify | Drop `service[redis:...]` parser leftovers (and tests in `install_test.go`). Pass to redis is wizard-gated. |
| `cmd/install_test.go` | Modify | Replace `service[redis:7]` test fixtures with `service[mail]` or remove. |
| `cmd/setup.go` | Modify | Add "Redis (native binary)" checkbox alongside the MySQL 8.4 one; install via `rediscmd.RunInstall([]string{})` if checked. |
| `cmd/setup_test.go` | Modify | Drop redis from the docker-services assertion (now empty docker registry). |
| `cmd/update.go` | Modify | After mysql pass, if `redis.IsInstalled()` call `rediscmd.RunUpdate([]string{})`. |
| `cmd/uninstall.go` | Modify | After mysql pass, if `redis.IsInstalled()` call `rediscmd.UninstallForce()`. |
| `scripts/e2e/redis-binary.sh` | Create | E2E lifecycle test (install, port, ping, set/get, env-binding, uninstall). |
| `scripts/e2e/diagnostics.sh` | Modify | Append redis log + datadir + state diagnostics blocks. |
| `.github/workflows/e2e.yml` | Modify | Add redis-binary phase after the mysql one. |

---

## Task 1: Verify redis tarball exists on the artifacts release

Research-only. Confirm assumptions before any code changes.

The artifacts pipeline (PR #78 added the `redis:` job) builds `redis-mac-arm64.tar.gz` containing `redis-server` and `redis-cli`. **Important:** at time of writing, the artifact may not yet exist on the rolling `artifacts` release. If missing, dispatch the build then stop and resume implementation once it's published.

- [ ] **Step 1: Check the artifacts release**

```bash
curl -s https://api.github.com/repos/prvious/pv/releases/tags/artifacts \
  | jq -r '.assets[].name' | grep '^redis-'
```

Expected output:
```
redis-mac-arm64.tar.gz
```

If missing, dispatch the workflow (skip-flagged so only the redis job runs):

```bash
gh workflow run build-artifacts.yml --ref main \
  -f skip_frankenphp=true -f skip_postgres=true -f skip_mysql=true \
  -f skip_mailpit=true -f skip_rustfs=true
```

Wait for the run to complete (`gh run watch`) and re-check. **Halt this plan until the asset is present** — every subsequent task depends on a real download URL.

- [ ] **Step 2: Download and inspect the tarball**

```bash
cd /tmp
rm -rf redis-extract && mkdir redis-extract
curl -fsSL -o redis.tar.gz "https://github.com/prvious/pv/releases/download/artifacts/redis-mac-arm64.tar.gz"
tar -xzf redis.tar.gz -C redis-extract
ls redis-extract
ls redis-extract/bin 2>/dev/null || ls redis-extract
```

Expected: a directory containing `redis-server` and `redis-cli`. If they live under `bin/` the `Install` orchestrator (Task 12) must extract preserving the `bin/` structure; if they're at the root, `RedisDir()/redis-server` and `RedisDir()/redis-cli` are the binary paths. Record the layout — `internal/redis/process.go` and `internal/redis/install.go` use it.

- [ ] **Step 3: Verify binaries run cleanly**

```bash
/tmp/redis-extract/redis-server --version 2>/dev/null || /tmp/redis-extract/bin/redis-server --version
```

Expected output shape: `Redis server v=7.4.1 sha=00000000:0 malloc=libc bits=64 build=...`. Record the exact prefix; Task 8 (`ProbeVersion`) parses against this.

```bash
otool -L /tmp/redis-extract/redis-server 2>/dev/null || otool -L /tmp/redis-extract/bin/redis-server
```

Expected: no `/opt/homebrew` or `/Users/runner` paths. If LEAK, halt — the artifacts pipeline regressed.

- [ ] **Step 4: Smoke-test boot + ping + shutdown**

```bash
DATA=/tmp/redis-extract-data
rm -rf "$DATA" && mkdir -p "$DATA"
BIN="/tmp/redis-extract/redis-server"
[ -x "$BIN" ] || BIN="/tmp/redis-extract/bin/redis-server"
CLI="/tmp/redis-extract/redis-cli"
[ -x "$CLI" ] || CLI="/tmp/redis-extract/bin/redis-cli"

"$BIN" --bind 127.0.0.1 --port 6379 --dir "$DATA" --dbfilename dump.rdb \
  --pidfile /tmp/pv-redis.pid --daemonize no --protected-mode no --appendonly no \
  >/tmp/redis-test.log 2>&1 &
RPID=$!
sleep 2
"$CLI" -h 127.0.0.1 -p 6379 PING
"$CLI" -h 127.0.0.1 -p 6379 SET k v
"$CLI" -h 127.0.0.1 -p 6379 GET k
kill $RPID
wait $RPID 2>/dev/null
rm -rf "$DATA" /tmp/redis-extract /tmp/redis.tar.gz /tmp/pv-redis.pid /tmp/redis-test.log
```

Expected: `PONG`, `OK`, `v`. If anything fails, halt and amend the spec / Task 8 / Task 15.

---

## Task 2: Redis path helpers

**Files:**
- Modify: `/Users/clovismuneza/Apps/pv/internal/config/paths.go`
- Modify: `/Users/clovismuneza/Apps/pv/internal/config/paths_test.go`

Centralize the redis paths next to the existing mysql/postgres helpers. `EnsureDirs()` learns to create both `RedisDir()` and `RedisDataDir()` so first-run installs and first-boot supervisor starts don't trip over missing parents.

- [ ] **Step 1: Write failing tests**

Append to `/Users/clovismuneza/Apps/pv/internal/config/paths_test.go`:

```go
func TestRedisDir(t *testing.T) {
	t.Setenv("HOME", "/home/test")
	got := RedisDir()
	want := "/home/test/.pv/redis"
	if got != want {
		t.Errorf("RedisDir = %q, want %q", got, want)
	}
}

func TestRedisDataDir(t *testing.T) {
	t.Setenv("HOME", "/home/test")
	got := RedisDataDir()
	want := "/home/test/.pv/data/redis"
	if got != want {
		t.Errorf("RedisDataDir = %q, want %q", got, want)
	}
}

func TestRedisLogPath(t *testing.T) {
	t.Setenv("HOME", "/home/test")
	got := RedisLogPath()
	want := "/home/test/.pv/logs/redis.log"
	if got != want {
		t.Errorf("RedisLogPath = %q, want %q", got, want)
	}
}

func TestEnsureDirs_CreatesRedisDirs(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if err := EnsureDirs(); err != nil {
		t.Fatalf("EnsureDirs: %v", err)
	}
	if _, err := os.Stat(RedisDir()); err != nil {
		t.Errorf("RedisDir not created: %v", err)
	}
	if _, err := os.Stat(RedisDataDir()); err != nil {
		t.Errorf("RedisDataDir not created: %v", err)
	}
}
```

If `os` isn't already imported in the test file, add it.

- [ ] **Step 2: Run tests, confirm failure**

```bash
go test ./internal/config/ -v -run 'TestRedis|TestEnsureDirs_CreatesRedisDirs'
```

Expected: build error (functions undefined).

- [ ] **Step 3: Implement helpers**

Append to `/Users/clovismuneza/Apps/pv/internal/config/paths.go` (after `MysqlLogPath`):

```go
// RedisDir is the root for the native redis binary tree:
// ~/.pv/redis/{redis-server,redis-cli}.
// Single-version — no per-version subdir.
func RedisDir() string {
	return filepath.Join(PvDir(), "redis")
}

// RedisDataDir is the redis-server data dir, kept under
// ~/.pv/data/redis/ so it survives a binary uninstall (unless
// --force is used). RDB snapshots land in <RedisDataDir>/dump.rdb.
func RedisDataDir() string {
	return filepath.Join(DataDir(), "redis")
}

// RedisLogPath returns the supervisor log file for redis.
func RedisLogPath() string {
	return filepath.Join(LogsDir(), "redis.log")
}
```

Modify `EnsureDirs` in the same file to register `RedisDir()` and `RedisDataDir()`:

```go
func EnsureDirs() error {
	dirs := []string{
		ConfigDir(),
		SitesDir(),
		LogsDir(),
		DataDir(),
		BinDir(),
		PhpDir(),
		ComposerDir(),
		ComposerCacheDir(),
		ServicesDir(),
		InternalBinDir(),
		PackagesDir(),
		ColimaHomeDir(),
		MysqlDir(),
		RedisDir(),
		RedisDataDir(),
	}
	for _, dir := range dirs {
		if err := os.MkdirAll(dir, 0755); err != nil {
			return err
		}
	}
	return nil
}
```

- [ ] **Step 4: Run tests, confirm pass**

```bash
go test ./internal/config/ -v -run 'TestRedis|TestEnsureDirs_CreatesRedisDirs'
```

- [ ] **Step 5: gofmt + vet + build**

```bash
gofmt -w internal/config/
go vet ./...
go build ./...
```

- [ ] **Step 6: Commit**

```bash
git add internal/config/paths.go internal/config/paths_test.go
git commit -m "feat(config): add redis path helpers"
```

---

## Task 3: `binaries.Redis` descriptor + URL builder

**Files:**
- Create: `/Users/clovismuneza/Apps/pv/internal/binaries/redis.go`
- Create: `/Users/clovismuneza/Apps/pv/internal/binaries/redis_test.go`

Add a `Binary` descriptor + URL builder. Single-version — no `IsValidRedisVersion` validator, no version arg. `PV_REDIS_URL_OVERRIDE` provides a test hook (used by the install/update tests to point at an httptest server).

- [ ] **Step 1: Write failing tests**

Create `/Users/clovismuneza/Apps/pv/internal/binaries/redis_test.go`:

```go
package binaries

import (
	"runtime"
	"testing"
)

func TestRedisURL(t *testing.T) {
	if runtime.GOOS != "darwin" || runtime.GOARCH != "arm64" {
		t.Skip("redis binaries only published for darwin/arm64 in v1")
	}
	got, err := RedisURL()
	if err != nil {
		t.Fatalf("RedisURL: %v", err)
	}
	want := "https://github.com/prvious/pv/releases/download/artifacts/redis-mac-arm64.tar.gz"
	if got != want {
		t.Errorf("RedisURL = %q, want %q", got, want)
	}
}

func TestRedisURL_UnsupportedPlatform(t *testing.T) {
	if runtime.GOOS == "darwin" && runtime.GOARCH == "arm64" {
		t.Skip("on supported platform; this test only runs elsewhere")
	}
	if _, err := RedisURL(); err == nil {
		t.Error("RedisURL should error on unsupported platform")
	}
}

func TestRedisURL_OverrideEnv(t *testing.T) {
	t.Setenv("PV_REDIS_URL_OVERRIDE", "http://127.0.0.1:9999/redis-test.tar.gz")
	got, err := RedisURL()
	if err != nil {
		t.Fatalf("RedisURL: %v", err)
	}
	want := "http://127.0.0.1:9999/redis-test.tar.gz"
	if got != want {
		t.Errorf("RedisURL with override = %q, want %q", got, want)
	}
}
```

- [ ] **Step 2: Run test, confirm failure**

```bash
go test ./internal/binaries/ -v -run TestRedisURL
```

Expected: `undefined: RedisURL`.

- [ ] **Step 3: Implement `internal/binaries/redis.go`**

Create `/Users/clovismuneza/Apps/pv/internal/binaries/redis.go`:

```go
package binaries

import (
	"fmt"
	"os"
	"runtime"
)

// Redis descriptor. Single-version — there is no version arg; the URL
// resolves to the rolling artifacts-release asset which always carries
// the latest GA upstream redis.
var Redis = Binary{
	Name:         "redis",
	DisplayName:  "Redis",
	NeedsExtract: true,
}

var redisPlatformNames = map[string]map[string]string{
	"darwin": {
		"arm64": "mac-arm64",
	},
}

// RedisURL returns the artifacts-release URL for redis. Today only
// darwin/arm64 is published; other platforms error.
//
// The PV_REDIS_URL_OVERRIDE environment variable, when set, replaces the
// computed URL outright. Tests use this to point installs at a local
// HTTP server. The override is applied before platform validation, so a
// test override works on any platform.
func RedisURL() (string, error) {
	if override := os.Getenv("PV_REDIS_URL_OVERRIDE"); override != "" {
		return override, nil
	}
	archMap, ok := redisPlatformNames[runtime.GOOS]
	if !ok {
		return "", fmt.Errorf("unsupported OS for Redis: %s", runtime.GOOS)
	}
	platform, ok := archMap[runtime.GOARCH]
	if !ok {
		return "", fmt.Errorf("unsupported architecture for Redis: %s/%s", runtime.GOOS, runtime.GOARCH)
	}
	return fmt.Sprintf("https://github.com/prvious/pv/releases/download/artifacts/redis-%s.tar.gz", platform), nil
}
```

- [ ] **Step 4: Run test, confirm pass**

```bash
go test ./internal/binaries/ -v -run TestRedisURL
```

- [ ] **Step 5: gofmt + vet + build**

```bash
gofmt -w internal/binaries/
go vet ./...
go build ./...
```

- [ ] **Step 6: Commit**

```bash
git add internal/binaries/redis.go internal/binaries/redis_test.go
git commit -m "feat(binaries): add Redis descriptor + URL builder with PV_REDIS_URL_OVERRIDE"
```

---

## Task 4: `internal/redis/port.go`

**Files:**
- Create: `/Users/clovismuneza/Apps/pv/internal/redis/port.go`
- Create: `/Users/clovismuneza/Apps/pv/internal/redis/port_test.go`

Single-version → constant 6379, no formula, no version arg.

- [ ] **Step 1: Write failing test**

Create `/Users/clovismuneza/Apps/pv/internal/redis/port_test.go`:

```go
package redis

import "testing"

func TestPortFor(t *testing.T) {
	if got := PortFor(); got != 6379 {
		t.Errorf("PortFor() = %d, want 6379", got)
	}
}
```

- [ ] **Step 2: Run test, confirm failure**

```bash
go test ./internal/redis/ -v -run TestPortFor
```

Expected: package doesn't exist yet — build error.

- [ ] **Step 3: Implement `internal/redis/port.go`**

Create `/Users/clovismuneza/Apps/pv/internal/redis/port.go`:

```go
// Package redis owns the lifecycle of the native redis binary managed by
// pv. Mirrors internal/postgres/ and internal/mysql/ but flat:
// single-version, no per-version map. State at ~/.pv/redis/ and
// ~/.pv/data/redis/.
package redis

// RedisPort is the TCP port pv binds redis-server to. Constant 6379 —
// the upstream default and the value every Laravel app expects out of
// the box. Single-version means there's no collision risk.
const RedisPort = 6379

// PortFor returns the TCP port redis-server should bind to.
// Kept as a function (not just exposing the const) for parallel API
// shape with mysql.PortFor / postgres.PortFor — callers don't have to
// branch on which package they're talking to.
func PortFor() int { return RedisPort }
```

- [ ] **Step 4: Run test, confirm pass**

```bash
go test ./internal/redis/ -v -run TestPortFor
```

- [ ] **Step 5: gofmt + vet + build**

```bash
gofmt -w internal/redis/
go vet ./...
go build ./...
```

- [ ] **Step 6: Commit**

```bash
git add internal/redis/port.go internal/redis/port_test.go
git commit -m "feat(redis): add PortFor constant (6379)"
```

---

## Task 5: `internal/redis/installed.go`

**Files:**
- Create: `/Users/clovismuneza/Apps/pv/internal/redis/installed.go`
- Create: `/Users/clovismuneza/Apps/pv/internal/redis/installed_test.go`

A redis is "installed" when `RedisDir()/redis-server` is a regular file. (No `bin/` subdir — the upstream tarball ships flat at the artifacts pipeline level. If Task 1 Step 2 revealed otherwise, adjust the path here.)

- [ ] **Step 1: Write failing test**

Create `/Users/clovismuneza/Apps/pv/internal/redis/installed_test.go`:

```go
package redis

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestIsInstalled_Empty(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if IsInstalled() {
		t.Error("IsInstalled should be false on empty home")
	}
}

func TestIsInstalled_FindsBinary(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if err := os.MkdirAll(config.RedisDir(), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	bin := filepath.Join(config.RedisDir(), "redis-server")
	if err := os.WriteFile(bin, []byte("#!/bin/sh\n"), 0o755); err != nil {
		t.Fatalf("write: %v", err)
	}
	if !IsInstalled() {
		t.Error("IsInstalled should be true after writing redis-server")
	}
}

func TestIsInstalled_DirWithoutBinary(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if err := os.MkdirAll(config.RedisDir(), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if IsInstalled() {
		t.Error("dir without redis-server should not count as installed")
	}
}
```

- [ ] **Step 2: Run test, confirm failure**

```bash
go test ./internal/redis/ -v -run TestIsInstalled
```

- [ ] **Step 3: Implement `internal/redis/installed.go`**

Create `/Users/clovismuneza/Apps/pv/internal/redis/installed.go`:

```go
package redis

import (
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/config"
)

// ServerBinary returns the absolute path to the bundled redis-server.
// Used by callers that need the path (e.g. process.BuildSupervisorProcess);
// keeps the join in one place.
func ServerBinary() string {
	return filepath.Join(config.RedisDir(), "redis-server")
}

// CLIBinary returns the absolute path to the bundled redis-cli.
// Not on PATH — internal use only (e2e tests, debugging).
func CLIBinary() string {
	return filepath.Join(config.RedisDir(), "redis-cli")
}

// IsInstalled reports whether redis-server exists at the expected path.
// A directory at config.RedisDir() with no redis-server is treated as
// not-installed (incomplete extraction, etc.).
func IsInstalled() bool {
	info, err := os.Stat(ServerBinary())
	return err == nil && !info.IsDir()
}
```

- [ ] **Step 4: Run test, confirm pass**

```bash
go test ./internal/redis/ -v -run TestIsInstalled
```

- [ ] **Step 5: gofmt + vet + build + commit**

```bash
gofmt -w internal/redis/
go vet ./...
go build ./...
git add internal/redis/installed.go internal/redis/installed_test.go
git commit -m "feat(redis): add IsInstalled + ServerBinary/CLIBinary helpers"
```

---

## Task 6: `internal/redis/state.go` — flat single-record state wrapper

**Files:**
- Create: `/Users/clovismuneza/Apps/pv/internal/redis/state.go`
- Create: `/Users/clovismuneza/Apps/pv/internal/redis/state_test.go`

Wraps the generic `internal/state` package under key `"redis"`. Unlike mysql/postgres, the sub-record is **flat** — a `State{Wanted string}` rather than a `Versions` map. Callers go through `SetWanted` to validate against the `WantedRunning`/`WantedStopped` constants so a typo can't silently persist.

- [ ] **Step 1: Write failing test**

Create `/Users/clovismuneza/Apps/pv/internal/redis/state_test.go`:

```go
package redis

import (
	"testing"
)

func TestState_RoundTrip(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	// Empty home → empty state.
	s, err := LoadState()
	if err != nil {
		t.Fatalf("LoadState: %v", err)
	}
	if s.Wanted != "" {
		t.Errorf("LoadState on empty home: Wanted = %q, want empty", s.Wanted)
	}

	if err := SetWanted(WantedRunning); err != nil {
		t.Fatalf("SetWanted: %v", err)
	}

	s, err = LoadState()
	if err != nil {
		t.Fatalf("LoadState after SetWanted: %v", err)
	}
	if s.Wanted != WantedRunning {
		t.Errorf("LoadState.Wanted = %q, want %q", s.Wanted, WantedRunning)
	}
}

func TestSetWanted_InvalidRejected(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if err := SetWanted("not-a-real-state"); err == nil {
		t.Error("SetWanted should reject unknown values")
	}
}

func TestRemoveState(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if err := SetWanted(WantedRunning); err != nil {
		t.Fatalf("SetWanted: %v", err)
	}
	if err := RemoveState(); err != nil {
		t.Fatalf("RemoveState: %v", err)
	}
	s, err := LoadState()
	if err != nil {
		t.Fatalf("LoadState after RemoveState: %v", err)
	}
	if s.Wanted != "" {
		t.Errorf("LoadState.Wanted after RemoveState = %q, want empty", s.Wanted)
	}
}
```

- [ ] **Step 2: Run test, confirm failure**

```bash
go test ./internal/redis/ -v -run 'TestState|TestSetWanted|TestRemoveState'
```

- [ ] **Step 3: Implement `internal/redis/state.go`**

Create `/Users/clovismuneza/Apps/pv/internal/redis/state.go`:

```go
package redis

import (
	"encoding/json"
	"fmt"
	"os"

	"github.com/prvious/pv/internal/state"
)

const stateKey = "redis"

// Wanted-state values for State.Wanted. Bare strings would let typos
// silently persist (and be silently read as "not running"), so callers
// go through SetWanted which validates against this set.
const (
	WantedRunning = "running"
	WantedStopped = "stopped"
)

// State is the redis slice of ~/.pv/data/state.json.
//
// Note the shape is FLAT (no Versions map) — redis is single-version, so
// a per-version sub-record would just add a layer of indirection over a
// single record. Compare with internal/mysql/state.go which uses a
// Versions map to disambiguate 8.0/8.4/9.7.
//
// On-disk JSON shape:
//
//	{
//	  "redis": { "wanted": "running" }
//	}
type State struct {
	Wanted string `json:"wanted"`
}

// LoadState reads the redis slice. Missing or empty → zero-value state.
// A corrupt slice is treated as empty with a one-time stderr warning,
// the same posture postgres/mysql take — recovery is `redis:start`.
func LoadState() (State, error) {
	all, err := state.Load()
	if err != nil {
		return State{}, err
	}
	raw, ok := all[stateKey]
	if !ok {
		return State{}, nil
	}
	var s State
	if err := json.Unmarshal(raw, &s); err != nil {
		fmt.Fprintf(os.Stderr, "redis: state slice corrupt (%v); treating as empty\n", err)
		return State{}, nil
	}
	return s, nil
}

// SaveState writes the redis slice, preserving other services' slices.
func SaveState(s State) error {
	all, err := state.Load()
	if err != nil {
		return err
	}
	payload, err := json.Marshal(s)
	if err != nil {
		return err
	}
	all[stateKey] = payload
	return state.Save(all)
}

// SetWanted updates the wanted-state and persists. Rejects values
// outside the WantedRunning/WantedStopped set so a typo can't silently
// persist garbage that IsWanted will later read as "not running" (and
// stop the process).
func SetWanted(wanted string) error {
	if wanted != WantedRunning && wanted != WantedStopped {
		return fmt.Errorf("redis: invalid wanted state %q (want %q or %q)", wanted, WantedRunning, WantedStopped)
	}
	return SaveState(State{Wanted: wanted})
}

// RemoveState drops the redis entry from state.json entirely. Used by
// `redis:uninstall` so a fresh install doesn't inherit a stale wanted
// flag from before.
func RemoveState() error {
	all, err := state.Load()
	if err != nil {
		return err
	}
	delete(all, stateKey)
	return state.Save(all)
}
```

- [ ] **Step 4: Run test, confirm pass**

```bash
go test ./internal/redis/ -v -run 'TestState|TestSetWanted|TestRemoveState'
```

- [ ] **Step 5: gofmt + vet + build + commit**

```bash
gofmt -w internal/redis/
go vet ./...
go build ./...
git add internal/redis/state.go internal/redis/state_test.go
git commit -m "feat(redis): flat state wrapper around internal/state"
```

---

## Task 7: `internal/redis/wanted.go`

**Files:**
- Create: `/Users/clovismuneza/Apps/pv/internal/redis/wanted.go`
- Create: `/Users/clovismuneza/Apps/pv/internal/redis/wanted_test.go`

`IsWanted()` is the reconciler's source of truth: returns true iff state's `Wanted == WantedRunning` AND the binary is on disk. The drift case (wanted=running but binary missing) emits a one-time stderr warning and returns false — recovery is `redis:install`.

- [ ] **Step 1: Write failing test**

Create `/Users/clovismuneza/Apps/pv/internal/redis/wanted_test.go`:

```go
package redis

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func install(t *testing.T) {
	t.Helper()
	if err := os.MkdirAll(config.RedisDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(config.RedisDir(), "redis-server"), []byte{}, 0o755); err != nil {
		t.Fatal(err)
	}
}

func TestIsWanted_NotInstalled(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if IsWanted() {
		t.Error("IsWanted should be false when binary missing")
	}
}

func TestIsWanted_InstalledButStopped(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	install(t)
	if err := SetWanted(WantedStopped); err != nil {
		t.Fatal(err)
	}
	if IsWanted() {
		t.Error("IsWanted should be false when wanted=stopped")
	}
}

func TestIsWanted_InstalledAndRunning(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	install(t)
	if err := SetWanted(WantedRunning); err != nil {
		t.Fatal(err)
	}
	if !IsWanted() {
		t.Error("IsWanted should be true when binary present and wanted=running")
	}
}

func TestIsWanted_StaleStateNoBinary(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	// SetWanted to running without installing the binary — drift case.
	if err := SetWanted(WantedRunning); err != nil {
		t.Fatal(err)
	}
	if IsWanted() {
		t.Error("IsWanted should be false when binary missing despite state=running")
	}
}
```

- [ ] **Step 2: Run test, confirm failure**

```bash
go test ./internal/redis/ -v -run TestIsWanted
```

- [ ] **Step 3: Implement `internal/redis/wanted.go`**

Create `/Users/clovismuneza/Apps/pv/internal/redis/wanted.go`:

```go
package redis

import (
	"fmt"
	"os"
)

// IsWanted reports whether redis should currently be supervised:
// state says wanted=running AND the binary is on disk. Stale entries
// (state says running but binary is missing) emit a stderr warning and
// return false — recovery is `redis:install` after the binary is
// restored.
func IsWanted() bool {
	st, err := LoadState()
	if err != nil {
		fmt.Fprintf(os.Stderr, "redis: load state: %v\n", err)
		return false
	}
	if st.Wanted != WantedRunning {
		return false
	}
	if !IsInstalled() {
		fmt.Fprintln(os.Stderr, "redis: state.json wants redis running but binary is missing; skipping")
		return false
	}
	return true
}
```

- [ ] **Step 4: Run test, confirm pass**

```bash
go test ./internal/redis/ -v -run TestIsWanted
```

- [ ] **Step 5: gofmt + vet + build + commit**

```bash
gofmt -w internal/redis/
go vet ./...
go build ./...
git add internal/redis/wanted.go internal/redis/wanted_test.go
git commit -m "feat(redis): IsWanted intersects state with installed-on-disk"
```

---

## Task 8: `internal/redis/version.go` — `redis-server --version` probe + fake test binary

**Files:**
- Create: `/Users/clovismuneza/Apps/pv/internal/redis/version.go`
- Create: `/Users/clovismuneza/Apps/pv/internal/redis/version_test.go`
- Create: `/Users/clovismuneza/Apps/pv/internal/redis/testdata/fake-redis-server.go`

`redis-server --version` output looks like:

```
Redis server v=7.4.1 sha=00000000:0 malloc=libc bits=64 build=...
```

The parser pulls the `v=X.Y.Z` token out. Test fakes are Go `main` programs under `testdata/` (per CLAUDE.md — never python/ruby/node).

- [ ] **Step 1: Write failing test**

Create `/Users/clovismuneza/Apps/pv/internal/redis/version_test.go`:

```go
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
```

- [ ] **Step 2: Implement the test fake**

Create `/Users/clovismuneza/Apps/pv/internal/redis/testdata/fake-redis-server.go`:

```go
//go:build ignore

// Synthetic redis-server used by version_test.go, install_test.go,
// process_test.go, and the server manager reconcile tests. Two modes:
//
//  1. --version: prints a real-looking redis-server version banner, exits 0.
//  2. long-run:  parse --port=<n> (or "--port <n>"), bind 127.0.0.1:<n>,
//                sleep until SIGTERM.
//
// This is a Go program, not a shell/python/ruby/node stub — per CLAUDE.md
// the only allowed runtime dependency is `go`.
package main

import (
	"fmt"
	"net"
	"os"
	"os/signal"
	"strconv"
	"strings"
	"syscall"
)

func main() {
	var (
		versionMode bool
		port        int
	)
	args := os.Args[1:]
	for i := 0; i < len(args); i++ {
		a := args[i]
		switch {
		case a == "--version":
			versionMode = true
		case a == "--port" && i+1 < len(args):
			if n, err := strconv.Atoi(args[i+1]); err == nil {
				port = n
			}
			i++
		case strings.HasPrefix(a, "--port="):
			if n, err := strconv.Atoi(strings.TrimPrefix(a, "--port=")); err == nil {
				port = n
			}
		}
	}

	if versionMode {
		// Mirror real redis-server output verbatim. parseRedisVersion in
		// internal/redis/version.go must match this.
		fmt.Println("Redis server v=7.4.1 sha=00000000:0 malloc=libc bits=64 build=fakefakefakefake")
		return
	}

	if port == 0 {
		port = 6379
	}
	l, err := net.Listen("tcp", "127.0.0.1:"+strconv.Itoa(port))
	if err != nil {
		os.Exit(4)
	}
	sigs := make(chan os.Signal, 1)
	signal.Notify(sigs, syscall.SIGTERM, syscall.SIGINT)
	go func() {
		for {
			c, err := l.Accept()
			if err != nil {
				return
			}
			c.Close()
		}
	}()
	<-sigs
	l.Close()
}
```

- [ ] **Step 3: Run test, confirm failure**

```bash
go test ./internal/redis/ -v -run 'TestParseRedisVersion|TestProbeVersion'
```

Expected: `parseRedisVersion`/`ProbeVersion` undefined.

- [ ] **Step 4: Implement `internal/redis/version.go`**

Create `/Users/clovismuneza/Apps/pv/internal/redis/version.go`:

```go
package redis

import (
	"fmt"
	"os/exec"
	"regexp"
	"strings"
)

// redisVersionRE pulls the version token out of `redis-server --version`.
// Real-world output:
//
//	Redis server v=7.4.1 sha=00000000:0 malloc=libc bits=64 build=...
//
// The regexp anchors on `v=` to avoid matching version-looking
// substrings elsewhere on the line (e.g. "build=v1234"-style tokens).
var redisVersionRE = regexp.MustCompile(`v=(\d+\.\d+\.\d+)\b`)

// ProbeVersion runs `<RedisDir>/redis-server --version` and returns the
// precise version string (e.g. "7.4.1"). Used at install/update time to
// record the patch level into versions.json.
func ProbeVersion() (string, error) {
	out, err := exec.Command(ServerBinary(), "--version").Output()
	if err != nil {
		return "", fmt.Errorf("redis-server --version: %w", err)
	}
	return parseRedisVersion(string(out))
}

// parseRedisVersion is exposed (lowercase) to the test in version_test.go
// so the parser can be exercised against many real-world output lines
// without having to compile a fake redis-server for each one.
func parseRedisVersion(out string) (string, error) {
	s := strings.TrimSpace(out)
	if s == "" {
		return "", fmt.Errorf("empty redis-server --version output")
	}
	m := redisVersionRE.FindStringSubmatch(s)
	if m == nil {
		return "", fmt.Errorf("unexpected redis-server --version output: %q", s)
	}
	return m[1], nil
}
```

- [ ] **Step 5: Run test, confirm pass**

```bash
go test ./internal/redis/ -v -run 'TestParseRedisVersion|TestProbeVersion'
```

- [ ] **Step 6: gofmt + vet + build + commit**

```bash
gofmt -w internal/redis/
go vet ./...
go build ./...
git add internal/redis/version.go internal/redis/version_test.go internal/redis/testdata/fake-redis-server.go
git commit -m "feat(redis): ProbeVersion + Go fake redis-server for tests"
```

---

## Task 9: `internal/redis/privileges.go`

**Files:**
- Create: `/Users/clovismuneza/Apps/pv/internal/redis/privileges.go`

Identical shape to `internal/mysql/privileges.go`. When pv runs as root (e.g. `sudo pv start` to bind :443), supervised processes need to drop to SUDO_UID/SUDO_GID — redis-server doesn't refuse to run as root the way mysqld does, but the data dir and log file get inherited from whoever launched pv, so we keep ownership consistent with the user's home dir.

- [ ] **Step 1: Implement `internal/redis/privileges.go`**

Create `/Users/clovismuneza/Apps/pv/internal/redis/privileges.go`:

```go
package redis

import (
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"syscall"
)

// dropCredential returns the credential pv should drop to when launching
// redis-server. Returns nil when no drop is needed (running as a
// non-root user, the typical dev case).
//
// When running as root with SUDO_UID/SUDO_GID set in the environment
// (which is what `sudo -E` populates), returns those — the daemon often
// needs root to bind :443, but redis-server should write its dump.rdb
// as the human user.
func dropCredential() *syscall.Credential {
	if os.Geteuid() != 0 {
		return nil
	}
	uidStr := os.Getenv("SUDO_UID")
	gidStr := os.Getenv("SUDO_GID")
	if uidStr == "" || gidStr == "" {
		return nil
	}
	uid, err := strconv.ParseUint(uidStr, 10, 32)
	if err != nil {
		return nil
	}
	gid, err := strconv.ParseUint(gidStr, 10, 32)
	if err != nil {
		return nil
	}
	return &syscall.Credential{Uid: uint32(uid), Gid: uint32(gid)}
}

// dropSysProcAttr wraps dropCredential into a SysProcAttr suitable for
// supervisor.Process.SysProcAttr. Returns nil when no drop is needed.
func dropSysProcAttr() *syscall.SysProcAttr {
	cred := dropCredential()
	if cred == nil {
		return nil
	}
	return &syscall.SysProcAttr{Credential: cred}
}

// chownToTarget recursively chowns path to the SUDO_UID/SUDO_GID when
// running as root. No-op when running as a non-root user.
func chownToTarget(path string) error {
	cred := dropCredential()
	if cred == nil {
		return nil
	}
	uid := int(cred.Uid)
	gid := int(cred.Gid)
	return filepath.Walk(path, func(p string, info os.FileInfo, err error) error {
		if err != nil {
			return err
		}
		if err := os.Lchown(p, uid, gid); err != nil {
			return fmt.Errorf("chown %s: %w", p, err)
		}
		return nil
	})
}
```

No test file — these helpers are exercised indirectly by `install_test.go` (the chown path is a no-op when not running as root, which is the test environment).

- [ ] **Step 2: gofmt + vet + build + commit**

```bash
gofmt -w internal/redis/
go vet ./...
go build ./...
git add internal/redis/privileges.go
git commit -m "feat(redis): credential-drop helpers (chown + SysProcAttr)"
```

---

## Task 10: `internal/redis/waitstopped.go`

**Files:**
- Create: `/Users/clovismuneza/Apps/pv/internal/redis/waitstopped.go`

`WaitStopped(timeout)` polls 127.0.0.1:6379 until the connection is refused, or until timeout. Used by uninstall/update before destructive on-disk operations. Redis shutdown is sub-second under typical loads so the timeout budget is shorter than mysql's (10s vs 30s).

- [ ] **Step 1: Implement `internal/redis/waitstopped.go`**

Create `/Users/clovismuneza/Apps/pv/internal/redis/waitstopped.go`:

```go
package redis

import (
	"fmt"
	"net"
	"time"
)

// WaitStopped polls the redis TCP port until connections are refused,
// or until timeout. Used by uninstall/update before destructive on-disk
// operations. A fixed sleep doesn't account for "redis is in the middle
// of an RDB save" — verify shutdown directly.
//
// 10s is plenty for a typical dev-load redis: even a forced BGSAVE on
// a multi-GB dataset finishes in seconds.
func WaitStopped(timeout time.Duration) error {
	addr := fmt.Sprintf("127.0.0.1:%d", PortFor())
	deadline := time.Now().Add(timeout)
	for time.Now().Before(deadline) {
		c, err := net.DialTimeout("tcp", addr, 200*time.Millisecond)
		if err != nil {
			return nil
		}
		c.Close()
		time.Sleep(200 * time.Millisecond)
	}
	return fmt.Errorf("redis did not stop within %s", timeout)
}
```

- [ ] **Step 2: gofmt + vet + build + commit**

```bash
gofmt -w internal/redis/
go vet ./...
go build ./...
git add internal/redis/waitstopped.go
git commit -m "feat(redis): WaitStopped TCP-poll helper"
```

---

## Task 11: `internal/redis/envvars.go`

**Files:**
- Create: `/Users/clovismuneza/Apps/pv/internal/redis/envvars.go`
- Create: `/Users/clovismuneza/Apps/pv/internal/redis/envvars_test.go`

`EnvVars(projectName)` returns the Laravel `REDIS_*` map. `projectName` is unused but kept in the signature for parallel shape with `mysql.EnvVars` / `postgres.EnvVars` so the dispatcher in `laravel/env.go` doesn't have to special-case redis. Returns `map[string]string` (no error) — there's nothing redis can fail at when the values are constants.

- [ ] **Step 1: Write failing test**

Create `/Users/clovismuneza/Apps/pv/internal/redis/envvars_test.go`:

```go
package redis

import "testing"

func TestEnvVars(t *testing.T) {
	got := EnvVars("verify-app")

	want := map[string]string{
		"REDIS_HOST":     "127.0.0.1",
		"REDIS_PORT":     "6379",
		"REDIS_PASSWORD": "null",
	}
	if len(got) != len(want) {
		t.Fatalf("EnvVars returned %d keys, want %d (%v)", len(got), len(want), got)
	}
	for k, v := range want {
		if got[k] != v {
			t.Errorf("EnvVars[%q] = %q, want %q", k, got[k], v)
		}
	}
}

func TestEnvVars_ProjectNameIgnored(t *testing.T) {
	a := EnvVars("alpha")
	b := EnvVars("beta")
	for k := range a {
		if a[k] != b[k] {
			t.Errorf("EnvVars varies by projectName: %q differs (%q vs %q)", k, a[k], b[k])
		}
	}
}
```

- [ ] **Step 2: Run test, confirm failure**

```bash
go test ./internal/redis/ -v -run TestEnvVars
```

- [ ] **Step 3: Implement `internal/redis/envvars.go`**

Create `/Users/clovismuneza/Apps/pv/internal/redis/envvars.go`:

```go
package redis

import "strconv"

// EnvVars returns the REDIS_* map injected into a linked project's .env
// when redis is bound. projectName is accepted but unused — kept for
// parallel signature with mysql.EnvVars / postgres.EnvVars so the
// dispatcher in laravel/env.go can treat all three uniformly.
//
// REDIS_PASSWORD is the literal string "null" — Laravel's
// config/database.php reads that as nil, matching the no-auth /
// loopback-only spec posture. Same shape the docker Redis used, so
// projects bound under the old service experience no .env churn on
// migration.
func EnvVars(projectName string) map[string]string {
	_ = projectName // unused — redis uses no project-scoped value
	return map[string]string{
		"REDIS_HOST":     "127.0.0.1",
		"REDIS_PORT":     strconv.Itoa(PortFor()),
		"REDIS_PASSWORD": "null",
	}
}
```

- [ ] **Step 4: Run test, confirm pass**

```bash
go test ./internal/redis/ -v -run TestEnvVars
```

- [ ] **Step 5: gofmt + vet + build + commit**

```bash
gofmt -w internal/redis/
go vet ./...
go build ./...
git add internal/redis/envvars.go internal/redis/envvars_test.go
git commit -m "feat(redis): EnvVars(projectName) returns REDIS_HOST/PORT/PASSWORD"
```

---

## Task 12: `internal/redis/install.go` — install orchestrator

**Files:**
- Create: `/Users/clovismuneza/Apps/pv/internal/redis/install.go`
- Create: `/Users/clovismuneza/Apps/pv/internal/redis/install_test.go`

Orchestrates: ensure dirs → resolve URL (override-aware) → download to staging → atomic rename → chownToTarget → ensure datadir → ProbeVersion + record → SetWanted(running). NO init step (redis has no `--initialize-insecure` equivalent — RDB is created on first save by redis-server itself).

Idempotent: re-running on an already-installed redis short-circuits to "re-mark wanted=running" + version-record refresh.

- [ ] **Step 1: Write failing test**

Create `/Users/clovismuneza/Apps/pv/internal/redis/install_test.go`:

```go
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
```

- [ ] **Step 2: Run test, confirm failure**

```bash
go test ./internal/redis/ -v -run TestInstall
```

Expected: `Install` undefined.

- [ ] **Step 3: Implement `internal/redis/install.go`**

Create `/Users/clovismuneza/Apps/pv/internal/redis/install.go`:

```go
package redis

import (
	"fmt"
	"net/http"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
)

// Install downloads, extracts, and registers redis as wanted=running.
// Idempotent: re-running on an already-installed redis is a no-op for
// files (skips download/extract) and just re-records wanted=running.
//
// Note there is NO init step (redis has no `--initialize-insecure`
// equivalent — RDB persistence is created on first save by redis-server
// itself, no schema bootstrap is needed).
func Install(client *http.Client) error {
	return InstallProgress(client, nil)
}

// InstallProgress is Install with a progress callback for the download phase.
func InstallProgress(client *http.Client, progress binaries.ProgressFunc) error {
	if err := config.EnsureDirs(); err != nil {
		return err
	}

	url, err := resolveRedisURL()
	if err != nil {
		return err
	}

	dir := config.RedisDir()
	if !IsInstalled() {
		stagingDir := dir + ".new"
		os.RemoveAll(stagingDir)
		if err := os.MkdirAll(stagingDir, 0o755); err != nil {
			return fmt.Errorf("create staging: %w", err)
		}
		archive := filepath.Join(config.PvDir(), "redis.tar.gz")
		if err := binaries.DownloadProgress(client, url, archive, progress); err != nil {
			os.RemoveAll(stagingDir)
			return fmt.Errorf("download: %w", err)
		}
		if err := binaries.ExtractTarGzAll(archive, stagingDir); err != nil {
			os.RemoveAll(stagingDir)
			os.Remove(archive)
			return fmt.Errorf("extract: %w", err)
		}
		os.Remove(archive)
		os.RemoveAll(dir)
		if err := os.Rename(stagingDir, dir); err != nil {
			os.RemoveAll(stagingDir)
			return fmt.Errorf("rename staging: %w", err)
		}
		// When pv runs as root (sudo pv start), hand the binary tree to
		// SUDO_USER so the dropped redis-server process can read it.
		if err := chownToTarget(dir); err != nil {
			return fmt.Errorf("chown redis tree: %w", err)
		}
	}

	// Ensure data dir + chown to SUDO_USER so the dropped redis-server
	// can write dump.rdb. EnsureDirs already created it; chown if root.
	if err := chownToTarget(config.RedisDataDir()); err != nil {
		return fmt.Errorf("chown redis data dir: %w", err)
	}

	// Probe + record version. Best-effort: a probe failure shouldn't
	// fail the install (the binary is already on disk and runnable; the
	// version record is diagnostic).
	if v, err := ProbeVersion(); err == nil {
		if vs, err := binaries.LoadVersions(); err == nil {
			vs.Set("redis", v)
			_ = vs.Save()
		}
	}

	return SetWanted(WantedRunning)
}

// resolveRedisURL allows tests to redirect the download via env var.
// Production: returns the artifacts-release URL from binaries.RedisURL.
func resolveRedisURL() (string, error) {
	if override := os.Getenv("PV_REDIS_URL_OVERRIDE"); override != "" {
		return override, nil
	}
	return binaries.RedisURL()
}
```

- [ ] **Step 4: Run test, confirm pass**

```bash
go test ./internal/redis/ -v -run TestInstall
```

- [ ] **Step 5: gofmt + vet + build + commit**

```bash
gofmt -w internal/redis/
go vet ./...
go build ./...
git add internal/redis/install.go internal/redis/install_test.go
git commit -m "feat(redis): Install orchestrator (download → extract → state)"
```

---

## Task 13: `internal/redis/uninstall.go`

**Files:**
- Create: `/Users/clovismuneza/Apps/pv/internal/redis/uninstall.go`
- Create: `/Users/clovismuneza/Apps/pv/internal/redis/uninstall_test.go`

`Uninstall(force)`: SetWanted(stopped) → WaitStopped(10s) → rm RedisDir → rm log → if force rm RedisDataDir → RemoveState → drop versions.json entry → `reg.UnbindService("redis")` (already exists; clears the bool field) → reg.Save.

- [ ] **Step 1: Write failing test**

Create `/Users/clovismuneza/Apps/pv/internal/redis/uninstall_test.go`:

```go
package redis

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
)

func setupInstalledRedis(t *testing.T) {
	t.Helper()
	if err := os.MkdirAll(config.RedisDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(config.RedisDir(), "redis-server"), []byte{}, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(config.RedisDataDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(config.RedisDataDir(), "dump.rdb"), []byte("fake"), 0o644); err != nil {
		t.Fatal(err)
	}
	_ = SetWanted(WantedRunning)
}

func TestUninstall_NoForce_KeepsDatadir(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	setupInstalledRedis(t)

	if err := Uninstall(false); err != nil {
		t.Fatalf("Uninstall: %v", err)
	}
	if _, err := os.Stat(config.RedisDir()); !os.IsNotExist(err) {
		t.Errorf("RedisDir should be removed: err=%v", err)
	}
	if _, err := os.Stat(config.RedisDataDir()); err != nil {
		t.Errorf("RedisDataDir should remain: %v", err)
	}
	st, _ := LoadState()
	if st.Wanted != "" {
		t.Errorf("state should be cleared, got Wanted=%q", st.Wanted)
	}
}

func TestUninstall_Force_RemovesDatadir(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	setupInstalledRedis(t)

	if err := Uninstall(true); err != nil {
		t.Fatalf("Uninstall(force): %v", err)
	}
	if _, err := os.Stat(config.RedisDataDir()); !os.IsNotExist(err) {
		t.Errorf("RedisDataDir should be removed with --force: err=%v", err)
	}
}

func TestUninstall_UnbindsProjects(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	setupInstalledRedis(t)

	// Pre-load a project bound to redis.
	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{},
		Projects: []registry.Project{
			{Name: "foo", Path: "/tmp/foo", Type: "laravel", Services: &registry.ProjectServices{Redis: true}},
		},
	}
	if err := reg.Save(); err != nil {
		t.Fatal(err)
	}

	if err := Uninstall(false); err != nil {
		t.Fatalf("Uninstall: %v", err)
	}

	r2, _ := registry.Load()
	if r2.Projects[0].Services != nil && r2.Projects[0].Services.Redis {
		t.Errorf("project should have Redis=false after uninstall")
	}
}
```

- [ ] **Step 2: Run test, confirm failure**

```bash
go test ./internal/redis/ -v -run TestUninstall
```

- [ ] **Step 3: Implement `internal/redis/uninstall.go`**

Create `/Users/clovismuneza/Apps/pv/internal/redis/uninstall.go`:

```go
package redis

import (
	"os"
	"time"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
)

// Uninstall removes on-disk state for redis. With force=false: removes
// binary tree, log file, state entry, and version-tracking entry; the
// data dir at ~/.pv/data/redis/ is preserved. With force=true: also
// removes the data dir.
//
// Caller's responsibility to handle the running daemon — Uninstall sets
// wanted=stopped and waits up to 10s for the TCP port to close before
// removing files.
func Uninstall(force bool) error {
	if isInstalledOnDisk() {
		_ = SetWanted(WantedStopped)
		_ = WaitStopped(10 * time.Second)
	}

	if err := os.RemoveAll(config.RedisDir()); err != nil {
		return err
	}
	_ = os.Remove(config.RedisLogPath())
	if force {
		if err := os.RemoveAll(config.RedisDataDir()); err != nil {
			return err
		}
	}
	if err := RemoveState(); err != nil {
		return err
	}
	if vs, err := binaries.LoadVersions(); err == nil {
		delete(vs.Versions, "redis")
		_ = vs.Save()
	}
	if reg, err := registry.Load(); err == nil {
		// UnbindService("redis") already exists in registry.go and clears
		// Services.Redis on every project — we don't need a redis-specific
		// helper because redis is single-version (mysql/postgres needed
		// version-aware helpers because their bindings carry a version).
		reg.UnbindService("redis")
		_ = reg.Save()
	}
	return nil
}

// isInstalledOnDisk is a cheap pre-check used by Uninstall to skip the
// 10s wait when there's nothing on disk.
func isInstalledOnDisk() bool {
	_, err := os.Stat(config.RedisDir())
	return err == nil
}
```

- [ ] **Step 4: Run test, confirm pass**

```bash
go test ./internal/redis/ -v -run TestUninstall
```

- [ ] **Step 5: gofmt + vet + build + commit**

```bash
gofmt -w internal/redis/
go vet ./...
go build ./...
git add internal/redis/uninstall.go internal/redis/uninstall_test.go
git commit -m "feat(redis): Uninstall (force vs non-force, project unbind)"
```

---

## Task 14: `internal/redis/update.go`

**Files:**
- Create: `/Users/clovismuneza/Apps/pv/internal/redis/update.go`
- Create: `/Users/clovismuneza/Apps/pv/internal/redis/update_test.go`

`Update(client)`: snapshot prior wanted-state, stop, redownload via staging-rename, restore prior wanted-state. Data dir untouched (RDB files are forward-compatible).

- [ ] **Step 1: Write failing test**

Create `/Users/clovismuneza/Apps/pv/internal/redis/update_test.go`:

```go
package redis

import (
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestUpdate_ReplacesBinaryTree(t *testing.T) {
	tarball := makeFakeRedisTarball(t)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write(tarball)
	}))
	defer srv.Close()

	t.Setenv("HOME", t.TempDir())
	t.Setenv("PV_REDIS_URL_OVERRIDE", srv.URL)

	if err := Install(http.DefaultClient); err != nil {
		t.Fatalf("Install: %v", err)
	}

	// Drop a sentinel into the data dir; Update must NOT touch it.
	dataFile := filepath.Join(config.RedisDataDir(), "dump.rdb")
	if err := os.WriteFile(dataFile, []byte("preserve-me"), 0o644); err != nil {
		t.Fatal(err)
	}

	if err := Update(http.DefaultClient); err != nil {
		t.Fatalf("Update: %v", err)
	}

	// Data file must still be there.
	if got, err := os.ReadFile(dataFile); err != nil {
		t.Fatalf("data file gone: %v", err)
	} else if string(got) != "preserve-me" {
		t.Errorf("data file mutated: %q", got)
	}

	// State must remain wanted=running.
	st, _ := LoadState()
	if st.Wanted != WantedRunning {
		t.Errorf("state.Wanted = %q after Update, want running", st.Wanted)
	}
}

func TestUpdate_NotInstalledErrors(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if err := Update(http.DefaultClient); err == nil {
		t.Error("Update should error when redis is not installed")
	}
}
```

- [ ] **Step 2: Run test, confirm failure**

```bash
go test ./internal/redis/ -v -run TestUpdate
```

- [ ] **Step 3: Implement `internal/redis/update.go`**

Create `/Users/clovismuneza/Apps/pv/internal/redis/update.go`:

```go
package redis

import (
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"time"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
)

// Update redownloads the redis tarball and atomically replaces the
// binary tree. Data dir is untouched. If wanted=running before the
// update, restores wanted=running on success; otherwise leaves wanted
// as-is (user-driven).
func Update(client *http.Client) error {
	return UpdateProgress(client, nil)
}

// UpdateProgress is Update with a download progress callback.
func UpdateProgress(client *http.Client, progress binaries.ProgressFunc) error {
	if !IsInstalled() {
		return fmt.Errorf("redis is not installed")
	}

	// Snapshot prior wanted-state so we can restore it after a successful
	// update. A user who explicitly stopped redis before running
	// `redis:update` should NOT see it auto-start.
	prevWanted := WantedStopped
	if st, err := LoadState(); err == nil && st.Wanted != "" {
		prevWanted = st.Wanted
	}

	// Stop running daemon (if any) and wait for the TCP port to close
	// before swapping binaries.
	if prevWanted == WantedRunning {
		_ = SetWanted(WantedStopped)
		_ = WaitStopped(10 * time.Second)
	}

	url, err := resolveRedisURL()
	if err != nil {
		return err
	}

	dir := config.RedisDir()
	stagingDir := dir + ".new"
	os.RemoveAll(stagingDir)
	if err := os.MkdirAll(stagingDir, 0o755); err != nil {
		return fmt.Errorf("create staging: %w", err)
	}

	archive := filepath.Join(config.PvDir(), "redis.tar.gz")
	if err := binaries.DownloadProgress(client, url, archive, progress); err != nil {
		os.RemoveAll(stagingDir)
		return fmt.Errorf("download: %w", err)
	}
	if err := binaries.ExtractTarGzAll(archive, stagingDir); err != nil {
		os.RemoveAll(stagingDir)
		os.Remove(archive)
		return fmt.Errorf("extract: %w", err)
	}
	os.Remove(archive)

	// Two-phase swap (NOT atomic — two os.Rename calls). If the second
	// rename fails we attempt a best-effort restore; if THAT also fails
	// the user is in a half-broken state and must know about it.
	oldDir := dir + ".old"
	os.RemoveAll(oldDir)
	if err := os.Rename(dir, oldDir); err != nil {
		os.RemoveAll(stagingDir)
		return fmt.Errorf("rename old: %w", err)
	}
	if err := os.Rename(stagingDir, dir); err != nil {
		if rollbackErr := os.Rename(oldDir, dir); rollbackErr != nil {
			return fmt.Errorf("rename new failed (%w); rollback also failed (%v); redis install dir is broken — manually mv %s %s",
				err, rollbackErr, oldDir, dir)
		}
		return fmt.Errorf("rename new: %w", err)
	}
	os.RemoveAll(oldDir)

	if err := chownToTarget(dir); err != nil {
		return fmt.Errorf("chown redis tree: %w", err)
	}

	// Re-probe + record version.
	if v, err := ProbeVersion(); err == nil {
		if vs, err := binaries.LoadVersions(); err == nil {
			vs.Set("redis", v)
			_ = vs.Save()
		}
	}

	// Restore prior wanted-state.
	return SetWanted(prevWanted)
}
```

- [ ] **Step 4: Run test, confirm pass**

```bash
go test ./internal/redis/ -v -run TestUpdate
```

- [ ] **Step 5: gofmt + vet + build + commit**

```bash
gofmt -w internal/redis/
go vet ./...
go build ./...
git add internal/redis/update.go internal/redis/update_test.go
git commit -m "feat(redis): Update preserves data dir and prior wanted state"
```

---

## Task 15: `internal/redis/process.go` — `BuildSupervisorProcess`

**Files:**
- Create: `/Users/clovismuneza/Apps/pv/internal/redis/process.go`
- Create: `/Users/clovismuneza/Apps/pv/internal/redis/process_test.go`

Boot flags (no redis.conf — single source of truth):
- `--bind 127.0.0.1`
- `--port 6379`
- `--dir <RedisDataDir>`
- `--dbfilename dump.rdb`
- `--pidfile /tmp/pv-redis.pid`
- `--daemonize no` (supervised)
- `--protected-mode no` (bind 127.0.0.1 already protects)
- `--appendonly no` (RDB only)

Compiled-in save policy stays in effect (3600s/1key, 300s/100keys, 60s/10000keys). NO `--logfile` — supervisor opens `RedisLogPath()` in the parent and inherits the fd, same trick we used for mysql.

- [ ] **Step 1: Write failing test**

Create `/Users/clovismuneza/Apps/pv/internal/redis/process_test.go`:

```go
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
```

- [ ] **Step 2: Run test, confirm failure**

```bash
go test ./internal/redis/ -v -run TestBuildSupervisorProcess
```

- [ ] **Step 3: Implement `internal/redis/process.go`**

Create `/Users/clovismuneza/Apps/pv/internal/redis/process.go`:

```go
package redis

import (
	"context"
	"fmt"
	"net"
	"os"
	"strconv"
	"time"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/supervisor"
)

// BuildSupervisorProcess returns a supervisor.Process for redis. Refuses
// to build when the binary is missing — the supervisor would just fail
// to exec and we want a clearer error.
//
// All boot configuration is on the command line — no redis.conf — so pv
// is the single source of truth.
func BuildSupervisorProcess() (supervisor.Process, error) {
	binPath := ServerBinary()
	if _, err := os.Stat(binPath); err != nil {
		return supervisor.Process{}, fmt.Errorf("redis: not installed (run pv redis:install)")
	}
	return supervisor.Process{
		Name:         "redis",
		Binary:       binPath,
		Args:         buildRedisArgs(),
		LogFile:      config.RedisLogPath(),
		SysProcAttr:  dropSysProcAttr(),
		Ready:        tcpReady(PortFor()),
		ReadyTimeout: 10 * time.Second,
	}, nil
}

// buildRedisArgs returns the flag set passed to redis-server at boot.
// Single source of truth: no redis.conf — every knob pv cares about is
// here.
//
// We deliberately do NOT pass --logfile: the supervisor opens
// RedisLogPath as the parent (running as root) and inherits the fd to
// the child, which sidesteps the ownership problem of the dropped
// redis-server process trying to open a root-owned log file itself.
// redis-server's stderr is captured via that inherited fd. Same fix we
// applied to mysql.
//
// Compiled-in save policy stays in effect (3600 1 / 300 100 / 60 10000).
// AOF off — RDB is sufficient for dev work.
func buildRedisArgs() []string {
	return []string{
		"--bind", "127.0.0.1",
		"--port", strconv.Itoa(PortFor()),
		"--dir", config.RedisDataDir(),
		"--dbfilename", "dump.rdb",
		"--pidfile", "/tmp/pv-redis.pid",
		"--daemonize", "no",
		"--protected-mode", "no",
		"--appendonly", "no",
	}
}

// tcpReady returns a Ready function that probes 127.0.0.1:port.
// redis-server starts accepting connections almost immediately after
// the listener binds — 10s is generous.
func tcpReady(port int) func(context.Context) error {
	addr := fmt.Sprintf("127.0.0.1:%d", port)
	return func(ctx context.Context) error {
		d := net.Dialer{Timeout: 500 * time.Millisecond}
		c, err := d.DialContext(ctx, "tcp", addr)
		if err != nil {
			return err
		}
		c.Close()
		return nil
	}
}
```

- [ ] **Step 4: Run test, confirm pass**

```bash
go test ./internal/redis/ -v -run TestBuildSupervisorProcess
```

- [ ] **Step 5: gofmt + vet + build + commit**

```bash
gofmt -w internal/redis/
go vet ./...
go build ./...
git add internal/redis/process.go internal/redis/process_test.go
git commit -m "feat(redis): BuildSupervisorProcess with boot flags"
```

---

## Task 16: `internal/redis/database.go` — `BindLinkedProjects`

**Files:**
- Create: `/Users/clovismuneza/Apps/pv/internal/redis/database.go`
- Create: `/Users/clovismuneza/Apps/pv/internal/redis/database_test.go`

`BindLinkedProjects()` is the retroactive-bind path called at the end of `redis:install`. Walks the registry, sets `Services.Redis = true` for every Laravel-shaped project (unconditionally — no `.env` heuristic, mirroring the mailpit/rustfs pattern), writes `REDIS_*` to `.env` via `laravel.UpdateProjectEnvForRedis`. Saves the registry once at the end if anything changed.

The forward path (linking after install) is covered by Task 22 (`automation/steps/detect_services.go`).

- [ ] **Step 1: Write failing test**

Create `/Users/clovismuneza/Apps/pv/internal/redis/database_test.go`:

```go
package redis

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/registry"
)

func writeProjectEnv(t *testing.T, dir, content string) {
	t.Helper()
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, ".env"), []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
}

func TestBindLinkedProjects_LaravelOnly(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	a := filepath.Join(t.TempDir(), "a")
	b := filepath.Join(t.TempDir(), "b")
	c := filepath.Join(t.TempDir(), "c")
	writeProjectEnv(t, a, "APP_NAME=a\n")
	writeProjectEnv(t, b, "APP_NAME=b\n")
	writeProjectEnv(t, c, "APP_NAME=c\n")

	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{},
		Projects: []registry.Project{
			{Name: "a", Path: a, Type: "laravel"},
			{Name: "b", Path: b, Type: "laravel-octane"},
			{Name: "c", Path: c, Type: "static"}, // not Laravel — should not bind
		},
	}
	if err := reg.Save(); err != nil {
		t.Fatal(err)
	}

	if err := BindLinkedProjects(); err != nil {
		t.Fatalf("BindLinkedProjects: %v", err)
	}

	r2, _ := registry.Load()
	if r2.Projects[0].Services == nil || !r2.Projects[0].Services.Redis {
		t.Errorf("project a (laravel) should have Redis=true")
	}
	if r2.Projects[1].Services == nil || !r2.Projects[1].Services.Redis {
		t.Errorf("project b (laravel-octane) should have Redis=true")
	}
	if r2.Projects[2].Services != nil && r2.Projects[2].Services.Redis {
		t.Errorf("project c (static) must NOT have Redis bound")
	}

	// .env files for laravel projects should have REDIS_HOST set.
	for _, p := range []string{a, b} {
		data, err := os.ReadFile(filepath.Join(p, ".env"))
		if err != nil {
			t.Fatal(err)
		}
		if !contains(string(data), "REDIS_HOST=127.0.0.1") {
			t.Errorf("project at %s missing REDIS_HOST=127.0.0.1, .env=%s", p, string(data))
		}
	}
}

func contains(s, sub string) bool {
	for i := 0; i+len(sub) <= len(s); i++ {
		if s[i:i+len(sub)] == sub {
			return true
		}
	}
	return false
}
```

- [ ] **Step 2: Run test, confirm failure**

```bash
go test ./internal/redis/ -v -run TestBindLinkedProjects
```

Expected: `BindLinkedProjects` undefined.

- [ ] **Step 3: Implement `internal/redis/database.go`**

Note: `internal/redis/database.go` cannot import `internal/laravel` (laravel imports redis once Task 21 lands → cycle). To break the cycle the same way mysql does, expose an `EnvWriter` callback variable that `internal/laravel/env.go` wires at init time. (Mysql avoids this because `internal/commands/mysql/install.go` calls `laravel.UpdateProjectEnvForMysql` from the *cobra layer*, not from `internal/mysql/`. Redis follows the same pattern: `BindLinkedProjects` here ONLY mutates the registry; the `.env` write is delegated to a callback wired by the cobra command.)

Create `/Users/clovismuneza/Apps/pv/internal/redis/database.go`:

```go
package redis

import (
	"fmt"

	"github.com/prvious/pv/internal/registry"
)

// EnvWriter is the per-project .env writer. Wired at init time by the
// cobra layer (internal/commands/redis/install.go) to call
// laravel.UpdateProjectEnvForRedis. Kept as a callback to break the
// internal/redis ↔ internal/laravel import cycle (laravel imports redis
// for EnvVars; redis can't import laravel back).
//
// Signature parallels the existing laravel.UpdateProjectEnvFor*
// helpers: (projectPath, projectName, *ProjectServices) error.
var EnvWriter func(projectPath, projectName string, bound *registry.ProjectServices) error

// BindLinkedProjects walks the registry and binds every Laravel-shaped
// project to redis (Services.Redis = true) plus, when EnvWriter is
// wired, writes REDIS_HOST/PORT/PASSWORD to each project's .env file.
//
// Mirrors mailpit/rustfs single-version auto-bind: redis is a
// transparent dependency for Laravel apps, so we don't gate on the
// project's existing .env content (no DB_CONNECTION-style heuristic).
//
// Saves the registry once at the end if anything changed.
func BindLinkedProjects() error {
	reg, err := registry.Load()
	if err != nil {
		return fmt.Errorf("load registry: %w", err)
	}
	changed := false
	for i := range reg.Projects {
		p := &reg.Projects[i]
		if p.Type != "laravel" && p.Type != "laravel-octane" {
			continue
		}
		if p.Services == nil {
			p.Services = &registry.ProjectServices{}
		}
		if !p.Services.Redis {
			p.Services.Redis = true
			changed = true
		}
		if EnvWriter != nil {
			if err := EnvWriter(p.Path, p.Name, p.Services); err != nil {
				// Best-effort: don't fail the whole install on one
				// project's .env write. The cobra layer logs via ui.Subtle.
				fmt.Printf("redis: bind %s: %v\n", p.Name, err)
			}
		}
	}
	if changed {
		if err := reg.Save(); err != nil {
			return fmt.Errorf("save registry: %w", err)
		}
	}
	return nil
}
```

- [ ] **Step 4: Run test, confirm pass**

The test wires `EnvWriter` itself for isolation. Add a `TestMain` (or wire inline) so the test exercises the env-writer path:

Append to `internal/redis/database_test.go` above the existing tests:

```go
import (
	"strconv"

	"github.com/prvious/pv/internal/services"
)

func init() {
	EnvWriter = func(projectPath, projectName string, bound *registry.ProjectServices) error {
		envPath := filepath.Join(projectPath, ".env")
		if _, err := os.Stat(envPath); os.IsNotExist(err) {
			return nil
		}
		return services.MergeDotEnv(envPath, "", map[string]string{
			"REDIS_HOST":     "127.0.0.1",
			"REDIS_PORT":     strconv.Itoa(PortFor()),
			"REDIS_PASSWORD": "null",
		})
	}
}
```

(Real production wiring is in Task 19's `internal/commands/redis/install.go` `init()`, which calls `laravel.UpdateProjectEnvForRedis`. The test wires a local stand-in to avoid the laravel import.)

```bash
go test ./internal/redis/ -v -run TestBindLinkedProjects
```

- [ ] **Step 5: gofmt + vet + build + commit**

```bash
gofmt -w internal/redis/
go vet ./...
go build ./...
git add internal/redis/database.go internal/redis/database_test.go
git commit -m "feat(redis): BindLinkedProjects + EnvWriter callback (cycle-safe)"
```

---

## Task 17: Extend `reconcileBinaryServices` with the redis source

**Files:**
- Modify: `/Users/clovismuneza/Apps/pv/internal/server/manager.go`
- Modify: `/Users/clovismuneza/Apps/pv/internal/server/manager_test.go`

The reconciler already has three sources (single-version services, postgres, mysql). This adds a fourth for redis. The supervisor key is `"redis"` (no version suffix).

- [ ] **Step 1: Write failing test**

Append to `/Users/clovismuneza/Apps/pv/internal/server/manager_test.go`:

```go
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
```

Add `"github.com/prvious/pv/internal/redis"` to the test file's imports (alphabetically, after `postgres`).

- [ ] **Step 2: Run test, confirm failure**

```bash
go test ./internal/server/ -v -run TestReconcileBinaryServices_StartsWantedRedis
```

- [ ] **Step 3: Modify `reconcileBinaryServices`**

Apply this unified diff to `/Users/clovismuneza/Apps/pv/internal/server/manager.go`:

```diff
@@ import (
 	"github.com/prvious/pv/internal/caddy"
 	"github.com/prvious/pv/internal/config"
 	"github.com/prvious/pv/internal/mysql"
 	"github.com/prvious/pv/internal/postgres"
+	"github.com/prvious/pv/internal/redis"
 	"github.com/prvious/pv/internal/registry"
 	"github.com/prvious/pv/internal/services"
 	"github.com/prvious/pv/internal/supervisor"
 )
@@ // reconcileBinaryServices brings supervisor state in line with the wanted
-// set computed from three sources:
+// set computed from four sources:
 //  1. registry: single-version services (rustfs, mailpit) marked Kind=binary
 //     and Enabled.
 //  2. internal/postgres: multi-version, on-disk + state.json driven.
 //  3. internal/mysql:    multi-version, on-disk + state.json driven.
+//  4. internal/redis:    single-version, on-disk + state.json driven.
 //
-// The diff/start/stop loop is shared across all three sources.
+// The diff/start/stop loop is shared across all four sources.
@@
 	// Source 3 — mysql, multi-version.
 	myVersions, myErr := mysql.WantedVersions()
 	if myErr != nil {
 		fmt.Fprintf(os.Stderr, "reconcile binary: mysql.WantedVersions: %v\n", myErr)
 	}
 	for _, version := range myVersions {
 		proc, err := mysql.BuildSupervisorProcess(version)
 		if err != nil {
 			startErrors = append(startErrors, fmt.Sprintf("mysql-%s: build: %v", version, err))
 			continue
 		}
 		wanted["mysql-"+version] = proc
 	}
+
+	// Source 4 — redis, single-version, filesystem + state.json.
+	if redis.IsWanted() {
+		proc, err := redis.BuildSupervisorProcess()
+		if err != nil {
+			startErrors = append(startErrors, fmt.Sprintf("redis: build: %v", err))
+		} else {
+			wanted["redis"] = proc
+		}
+	}
 
 	// Diff: stop unneeded. If the postgres source failed, skip postgres-
 	// prefixed keys — a transient state.json read error shouldn't kill
 	// running postgres processes (the wanted set is incomplete, not empty).
 	// Same transient-error guard for mysql.
 	for _, supKey := range m.supervisor.SupervisedNames() {
 		if _, ok := wanted[supKey]; ok {
 			continue
 		}
 		if pgErr != nil && strings.HasPrefix(supKey, "postgres-") {
 			continue
 		}
 		if myErr != nil && strings.HasPrefix(supKey, "mysql-") {
 			continue
 		}
```

(No transient-error guard for redis is needed — `IsWanted()` either returns true or false; there's no error channel that would flap independently of installed-on-disk.)

Verify imports stay alphabetically ordered within the second group: `caddy`, `config`, `mysql`, `postgres`, `redis`, `registry`, `services`, `supervisor`.

- [ ] **Step 4: Run test, confirm pass**

```bash
go test ./internal/server/ -v -run TestReconcileBinaryServices_StartsWantedRedis
go test ./internal/server/ -v -run TestReconcileBinaryServices_StopsRemovedRedis
go test ./internal/server/ -v   # full server package — postgres/mysql assertions still pass
```

- [ ] **Step 5: gofmt + vet + build + commit**

```bash
gofmt -w internal/server/
go vet ./...
go build ./...
git add internal/server/manager.go internal/server/manager_test.go
git commit -m "feat(server): reconcileBinaryServices picks up redis"
```

---

## Task 18: Cobra commands skeleton — register.go + bridge

**Files:**
- Create: `/Users/clovismuneza/Apps/pv/internal/commands/redis/register.go`
- Create: `/Users/clovismuneza/Apps/pv/cmd/redis.go`

Wires the (initially empty) `redis:*` group onto `rootCmd`. Subsequent tasks attach individual commands to the `cmds` slice.

- [ ] **Step 1: Create `internal/commands/redis/register.go`**

```go
// Package redis holds cobra commands for the redis:* group. There is
// intentionally no alias namespace — `redis:` is already short.
package redis

import (
	"github.com/spf13/cobra"
)

// Register wires every redis:* command onto parent.
func Register(parent *cobra.Command) {
	cmds := []*cobra.Command{
		installCmd,
		uninstallCmd,
		updateCmd,
		startCmd,
		stopCmd,
		restartCmd,
		listCmd,
		logsCmd,
		statusCmd,
		downloadCmd, // hidden; included so it's discoverable for debugging
	}
	for _, c := range cmds {
		parent.AddCommand(c)
	}
}

// Run* — convenience wrappers for orchestrators (pv install / pv update /
// pv uninstall) and the setup wizard. Each one threads args through to
// the corresponding cobra command's RunE so behavior stays in a single
// place.
func RunInstall(args []string) error {
	return installCmd.RunE(installCmd, args)
}

func RunUpdate(args []string) error {
	return updateCmd.RunE(updateCmd, args)
}

func RunUninstall(args []string) error {
	return uninstallCmd.RunE(uninstallCmd, args)
}

// UninstallForce removes redis without a confirmation prompt. Used by
// the pv uninstall orchestrator after it has already obtained blanket
// consent from the user. Mirrors postgres.UninstallForce / mysql.UninstallForce.
func UninstallForce() error {
	prev := uninstallForce
	uninstallForce = true
	defer func() { uninstallForce = prev }()
	return uninstallCmd.RunE(uninstallCmd, nil)
}
```

- [ ] **Step 2: Create `cmd/redis.go`**

```go
package cmd

import (
	rediscmd "github.com/prvious/pv/internal/commands/redis"
	"github.com/spf13/cobra"
)

func init() {
	rootCmd.AddGroup(&cobra.Group{
		ID:    "redis",
		Title: "Redis Management:",
	})
	rediscmd.Register(rootCmd)
}
```

- [ ] **Step 3: Build will fail until Tasks 19–25 land**

```bash
go build ./...
```

Expected: undefined `installCmd`, `uninstallCmd`, etc. That's fine — the next tasks fill them in. Don't commit yet; commit at the end of Task 25 once the package compiles.

---

## Task 19: `redis:install` + `redis:download`

**Files:**
- Create: `/Users/clovismuneza/Apps/pv/internal/commands/redis/install.go`
- Create: `/Users/clovismuneza/Apps/pv/internal/commands/redis/download.go`

`install` is user-facing; `download` is hidden debug. Both call into `redis.InstallProgress`. After install, wire `redis.EnvWriter` and call `redis.BindLinkedProjects()` so projects pre-linked before the install get their `.env` updated.

- [ ] **Step 1: Implement `internal/commands/redis/download.go`**

```go
package redis

import (
	"net/http"
	"time"

	r "github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

// Hidden debug rung. Mirrors postgres' / mysql's downloadCmd: collapses
// to the same call as :install. Useful when poking at a half-installed
// state without going through the wizard / orchestrator.
var downloadCmd = &cobra.Command{
	Use:     "redis:download",
	GroupID: "redis",
	Short:   "Run the full install pipeline (debug; same as redis:install)",
	Hidden:  true,
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		client := &http.Client{Timeout: 5 * time.Minute}
		return ui.StepProgress("Downloading Redis...",
			func(progress func(written, total int64)) (string, error) {
				if err := r.InstallProgress(client, progress); err != nil {
					return "", err
				}
				return "Installed Redis", nil
			})
	},
}
```

- [ ] **Step 2: Implement `internal/commands/redis/install.go`**

```go
package redis

import (
	"fmt"

	"github.com/prvious/pv/internal/laravel"
	r "github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

func init() {
	// Wire the env-writer callback so r.BindLinkedProjects can write
	// REDIS_* into project .env files. Lives here (not in
	// internal/redis/) to avoid the redis ↔ laravel import cycle.
	r.EnvWriter = func(projectPath, projectName string, bound *registry.ProjectServices) error {
		return laravel.UpdateProjectEnvForRedis(projectPath, projectName, bound)
	}
}

var installCmd = &cobra.Command{
	Use:     "redis:install",
	GroupID: "redis",
	Short:   "Install (or re-install) Redis",
	Long:    "Downloads the Redis binary, registers it as wanted-running, and binds every linked Laravel project. No version arg — single-version service.",
	Example: `pv redis:install`,
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		// Already installed → idempotent: re-mark wanted=running, re-bind
		// linked projects (in case any were added since), and signal the
		// daemon. Same friendly contract postgres/mysql use.
		if r.IsInstalled() {
			if err := r.SetWanted(r.WantedRunning); err != nil {
				return err
			}
			if err := r.BindLinkedProjects(); err != nil {
				ui.Subtle(fmt.Sprintf("Could not retroactively bind linked projects: %v", err))
			}
			ui.Success("Redis already installed — marked as wanted running.")
			return signalDaemon()
		}

		// Run the download/extract pipeline.
		if err := downloadCmd.RunE(downloadCmd, nil); err != nil {
			return err
		}
		if err := r.BindLinkedProjects(); err != nil {
			ui.Subtle(fmt.Sprintf("Could not retroactively bind linked projects: %v", err))
		}
		ui.Success("Redis installed.")
		return signalDaemon()
	},
}

// signalDaemon nudges the running pv daemon to reconcile, or no-ops with
// a friendly note if the daemon isn't up.
func signalDaemon() error {
	if !server.IsRunning() {
		ui.Subtle("daemon not running — redis will start on next `pv start`")
		return nil
	}
	return server.SignalDaemon()
}
```

The `laravel.UpdateProjectEnvForRedis` reference will be undefined until Task 21 lands. That's expected — the package won't compile until then. Don't run the build yet.

---

## Task 20: `redis:uninstall` / `:update` / `:start` / `:stop` / `:restart`

**Files:**
- Create: `/Users/clovismuneza/Apps/pv/internal/commands/redis/uninstall.go`
- Create: `/Users/clovismuneza/Apps/pv/internal/commands/redis/update.go`
- Create: `/Users/clovismuneza/Apps/pv/internal/commands/redis/start.go`
- Create: `/Users/clovismuneza/Apps/pv/internal/commands/redis/stop.go`
- Create: `/Users/clovismuneza/Apps/pv/internal/commands/redis/restart.go`

- [ ] **Step 1: Create `internal/commands/redis/uninstall.go`**

```go
package redis

import (
	"fmt"
	"time"

	"charm.land/huh/v2"
	r "github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var uninstallForce bool

var uninstallCmd = &cobra.Command{
	Use:     "redis:uninstall",
	GroupID: "redis",
	Short:   "Stop, remove the binary, and (with --force) DELETE the data directory",
	Long: "Stops the supervised process and removes the binary tree at " +
		"~/.pv/redis/. With --force, also removes the data directory at " +
		"~/.pv/data/redis/ (deletes dump.rdb). Unbinds every linked project.",
	Example: `pv redis:uninstall --force`,
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		if !r.IsInstalled() {
			ui.Subtle("Redis is not installed.")
			return nil
		}
		if !uninstallForce {
			confirmed := false
			if err := huh.NewConfirm().
				Title("Remove Redis? With --force this also DELETES the data directory. This cannot be undone.").
				Affirmative("Yes").
				Negative("No").
				Value(&confirmed).
				Run(); err != nil {
				return err
			}
			if !confirmed {
				return fmt.Errorf("aborted")
			}
		}

		if err := r.SetWanted(r.WantedStopped); err != nil {
			return err
		}
		if server.IsRunning() {
			if err := server.SignalDaemon(); err != nil {
				return fmt.Errorf("signal daemon: %w", err)
			}
			if err := r.WaitStopped(10 * time.Second); err != nil {
				return fmt.Errorf("waiting for redis to stop: %w", err)
			}
		}

		if err := ui.Step("Uninstalling Redis...", func() (string, error) {
			if err := r.Uninstall(uninstallForce); err != nil {
				return "", err
			}
			return "Uninstalled Redis", nil
		}); err != nil {
			return err
		}

		// Unbind from projects — Uninstall already did this internally,
		// but reload + save once more here is a defensive belt-and-braces
		// in case another writer raced us between steps.
		reg, err := registry.Load()
		if err != nil {
			return err
		}
		reg.UnbindService("redis")
		if err := reg.Save(); err != nil {
			return err
		}

		ui.Success("Redis uninstalled.")
		return nil
	},
}

func init() {
	uninstallCmd.Flags().BoolVar(&uninstallForce, "force", false, "Skip the confirmation prompt and delete the data directory")
}
```

- [ ] **Step 2: Create `internal/commands/redis/update.go`**

```go
package redis

import (
	"fmt"
	"net/http"
	"time"

	r "github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var updateCmd = &cobra.Command{
	Use:     "redis:update",
	GroupID: "redis",
	Short:   "Re-download Redis (data dir untouched)",
	Example: `pv redis:update`,
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		if !r.IsInstalled() {
			return fmt.Errorf("redis is not installed")
		}

		wasRunning := false
		if st, err := r.LoadState(); err == nil && st.Wanted == r.WantedRunning {
			wasRunning = true
		}

		if err := r.SetWanted(r.WantedStopped); err != nil {
			return err
		}
		if server.IsRunning() {
			if err := server.SignalDaemon(); err != nil {
				return fmt.Errorf("signal daemon: %w", err)
			}
			if err := r.WaitStopped(10 * time.Second); err != nil {
				return fmt.Errorf("waiting for redis to stop: %w", err)
			}
		}

		client := &http.Client{Timeout: 5 * time.Minute}
		if err := ui.StepProgress("Updating Redis...",
			func(progress func(written, total int64)) (string, error) {
				if err := r.UpdateProgress(client, progress); err != nil {
					return "", err
				}
				return "Updated Redis", nil
			}); err != nil {
			return err
		}

		if wasRunning {
			if err := r.SetWanted(r.WantedRunning); err != nil {
				return err
			}
		}

		ui.Success("Redis updated.")
		if server.IsRunning() {
			return server.SignalDaemon()
		}
		return nil
	},
}
```

- [ ] **Step 3: Create `internal/commands/redis/start.go`**

```go
package redis

import (
	r "github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var startCmd = &cobra.Command{
	Use:     "redis:start",
	GroupID: "redis",
	Short:   "Mark Redis as wanted-running",
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		if !r.IsInstalled() {
			ui.Subtle("Redis is not installed (run `pv redis:install`).")
			return nil
		}
		if err := r.SetWanted(r.WantedRunning); err != nil {
			return err
		}
		ui.Success("Redis marked running.")
		if server.IsRunning() {
			return server.SignalDaemon()
		}
		ui.Subtle("daemon not running — will start on next `pv start`")
		return nil
	},
}
```

- [ ] **Step 4: Create `internal/commands/redis/stop.go`**

```go
package redis

import (
	r "github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var stopCmd = &cobra.Command{
	Use:     "redis:stop",
	GroupID: "redis",
	Short:   "Mark Redis as wanted-stopped",
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		if err := r.SetWanted(r.WantedStopped); err != nil {
			return err
		}
		ui.Success("Redis marked stopped.")
		if server.IsRunning() {
			return server.SignalDaemon()
		}
		return nil
	},
}
```

- [ ] **Step 5: Create `internal/commands/redis/restart.go`**

```go
package redis

import (
	"fmt"
	"time"

	r "github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var restartCmd = &cobra.Command{
	Use:     "redis:restart",
	GroupID: "redis",
	Short:   "Stop and start Redis",
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		// Phase 1: ask for stop, signal, wait for actual shutdown. Skipping
		// WaitStopped here would race with the supervisor's restart of the
		// next phase.
		if err := r.SetWanted(r.WantedStopped); err != nil {
			return err
		}
		if server.IsRunning() {
			if err := server.SignalDaemon(); err != nil {
				return fmt.Errorf("signal daemon: %w", err)
			}
			if err := r.WaitStopped(10 * time.Second); err != nil {
				return fmt.Errorf("waiting for redis to stop: %w", err)
			}
		}
		// Phase 2: ask for running, signal once.
		if err := r.SetWanted(r.WantedRunning); err != nil {
			return err
		}
		if server.IsRunning() {
			if err := server.SignalDaemon(); err != nil {
				return err
			}
		}
		ui.Success("Redis restarted.")
		return nil
	},
}
```

---

## Task 21: `redis:list` / `:status` / `:logs`

**Files:**
- Create: `/Users/clovismuneza/Apps/pv/internal/commands/redis/list.go`
- Create: `/Users/clovismuneza/Apps/pv/internal/commands/redis/status.go`
- Create: `/Users/clovismuneza/Apps/pv/internal/commands/redis/logs.go`

- [ ] **Step 1: Create `internal/commands/redis/list.go`**

```go
package redis

import (
	"fmt"
	"strings"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
	r "github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var listCmd = &cobra.Command{
	Use:     "redis:list",
	GroupID: "redis",
	Short:   "Show Redis status (single row)",
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		if !r.IsInstalled() {
			ui.Subtle("Redis is not installed.")
			return nil
		}

		st, _ := r.LoadState()
		vs, _ := binaries.LoadVersions()
		reg, _ := registry.Load()
		status, _ := server.ReadDaemonStatus()

		precise := "?"
		if vs != nil {
			if v := vs.Get("redis"); v != "" {
				precise = v
			}
		}

		runState := "stopped"
		if status != nil {
			if s, ok := status.Supervised["redis"]; ok && s.Running {
				runState = "running"
			}
		}
		wanted := st.Wanted
		if wanted == "" {
			wanted = "—"
		}

		projects := []string{}
		if reg != nil {
			for _, p := range reg.List() {
				if p.Services != nil && p.Services.Redis {
					projects = append(projects, p.Name)
				}
			}
		}
		projectsCol := "—"
		if len(projects) > 0 {
			projectsCol = strings.Join(projects, ",")
		}

		rows := [][]string{{
			precise,
			fmt.Sprintf("%d", r.PortFor()),
			fmt.Sprintf("%s (%s)", runState, wanted),
			config.RedisDataDir(),
			projectsCol,
		}}
		ui.Table([]string{"VERSION", "PORT", "STATUS", "DATA DIR", "LINKED PROJECTS"}, rows)
		return nil
	},
}
```

- [ ] **Step 2: Create `internal/commands/redis/status.go`**

```go
package redis

import (
	"fmt"

	r "github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var statusCmd = &cobra.Command{
	Use:     "redis:status",
	GroupID: "redis",
	Short:   "Show Redis status",
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		if !r.IsInstalled() {
			ui.Subtle("Redis is not installed.")
			return nil
		}
		status, _ := server.ReadDaemonStatus()
		if status != nil {
			if s, ok := status.Supervised["redis"]; ok && s.Running {
				ui.Success(fmt.Sprintf("redis: running on :%d (pid %d)", r.PortFor(), s.PID))
				return nil
			}
		}
		ui.Subtle("redis: stopped")
		return nil
	},
}
```

- [ ] **Step 3: Create `internal/commands/redis/logs.go`**

```go
package redis

import (
	"io"
	"os"
	"os/exec"

	"github.com/prvious/pv/internal/config"
	"github.com/spf13/cobra"
)

var logsFollow bool

var logsCmd = &cobra.Command{
	Use:     "redis:logs",
	GroupID: "redis",
	Short:   "Tail the Redis log file",
	Long:    "Reads ~/.pv/logs/redis.log. With -f / --follow, tails the file like `tail -f`.",
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		path := config.RedisLogPath()
		if logsFollow {
			c := exec.Command("tail", "-f", path)
			c.Stdout = os.Stdout
			c.Stderr = os.Stderr
			return c.Run()
		}
		f, err := os.Open(path)
		if err != nil {
			return err
		}
		defer f.Close()
		_, err = io.Copy(os.Stdout, f)
		return err
	},
}

func init() {
	logsCmd.Flags().BoolVarP(&logsFollow, "follow", "f", false, "Follow the log (tail -f)")
}
```

---

## Task 22: `laravel.UpdateProjectEnvForRedis`

**Files:**
- Modify: `/Users/clovismuneza/Apps/pv/internal/laravel/env.go`
- Modify: `/Users/clovismuneza/Apps/pv/internal/laravel/env_test.go`

Add the env-writer helper that `internal/commands/redis/install.go` wires as `r.EnvWriter`. Mirrors `UpdateProjectEnvForMysql` but with redis's no-error `EnvVars` signature.

- [ ] **Step 1: Write failing test**

Append to `/Users/clovismuneza/Apps/pv/internal/laravel/env_test.go` (create if it doesn't exist — see existing tests for the pattern):

```go
func TestUpdateProjectEnvForRedis(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	dir := t.TempDir()
	envPath := filepath.Join(dir, ".env")
	if err := os.WriteFile(envPath, []byte("APP_NAME=test\nREDIS_HOST=stale\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	bound := &registry.ProjectServices{Redis: true}
	if err := UpdateProjectEnvForRedis(dir, "test", bound); err != nil {
		t.Fatalf("UpdateProjectEnvForRedis: %v", err)
	}

	got, err := os.ReadFile(envPath)
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{
		"REDIS_HOST=127.0.0.1",
		"REDIS_PORT=6379",
		"REDIS_PASSWORD=null",
		"CACHE_STORE=redis",       // smart vars from SmartEnvVars
		"SESSION_DRIVER=redis",
		"QUEUE_CONNECTION=redis",
	} {
		if !strings.Contains(string(got), want) {
			t.Errorf(".env missing %q; got: %s", want, string(got))
		}
	}
}

func TestUpdateProjectEnvForRedis_NoEnvFile(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	dir := t.TempDir()
	bound := &registry.ProjectServices{Redis: true}
	// No .env file in dir — helper must no-op without error.
	if err := UpdateProjectEnvForRedis(dir, "test", bound); err != nil {
		t.Fatalf("UpdateProjectEnvForRedis on missing .env: %v", err)
	}
}
```

If imports are missing, add: `"os"`, `"path/filepath"`, `"strings"`, `"testing"`, `"github.com/prvious/pv/internal/registry"`.

- [ ] **Step 2: Run test, confirm failure**

```bash
go test ./internal/laravel/ -v -run TestUpdateProjectEnvForRedis
```

- [ ] **Step 3: Implement `UpdateProjectEnvForRedis` in `internal/laravel/env.go`**

Append to `/Users/clovismuneza/Apps/pv/internal/laravel/env.go` (after `UpdateProjectEnvForMysql`):

```go
// UpdateProjectEnvForRedis mirrors UpdateProjectEnvForMysql /
// UpdateProjectEnvForPostgres for the redis native-binary case.
// Redis has the simplest signature — EnvVars(projectName) returns a
// constant map (REDIS_HOST/PORT/PASSWORD), no error.
func UpdateProjectEnvForRedis(projectPath, projectName string, bound *registry.ProjectServices) error {
	envPath := filepath.Join(projectPath, ".env")
	if _, err := os.Stat(envPath); os.IsNotExist(err) {
		return nil
	}
	rVars := redis.EnvVars(projectName)
	smartVars := SmartEnvVars(bound)
	for k, v := range smartVars {
		rVars[k] = v
	}
	backupPath := envPath + ".pv-backup"
	return services.MergeDotEnv(envPath, backupPath, rVars)
}
```

Add `"github.com/prvious/pv/internal/redis"` to the import block (alphabetically, between `postgres` and `registry`).

- [ ] **Step 4: Run test, confirm pass**

```bash
go test ./internal/laravel/ -v -run TestUpdateProjectEnvForRedis
```

- [ ] **Step 5: gofmt + vet + build (the redis-cobra package should now compile)**

```bash
gofmt -w internal/laravel/ internal/commands/redis/ cmd/
go vet ./...
go build ./...
```

- [ ] **Step 6: Commit (Tasks 18–22 land together)**

```bash
git add internal/commands/redis/ cmd/redis.go internal/laravel/env.go internal/laravel/env_test.go
git commit -m "feat(redis): cobra commands (install/uninstall/update/start/stop/restart/list/status/logs) + UpdateProjectEnvForRedis"
```

---

## Task 23: Wire `UpdateProjectEnvForRedis` into the link pipeline (`DetectServicesStep`)

**Files:**
- Modify: `/Users/clovismuneza/Apps/pv/internal/laravel/steps.go`

`DetectServicesStep.Run` already iterates over bound services; add a redis branch parallel to the postgres / mysql ones. Also fix the existing `len(vars) == 0` short-circuit so it doesn't return early before the per-service env writers run.

- [ ] **Step 1: Modify `DetectServicesStep.Run` in `internal/laravel/steps.go`**

Apply this unified diff:

```diff
@@ func (s *DetectServicesStep) Run(ctx *automation.Context) (string, error) {
 	proj := ctx.Registry.Find(ctx.ProjectName)
 	if proj == nil || proj.Services == nil {
 		return "no services bound", nil
 	}
 	vars := SmartEnvVars(proj.Services)
 	envPath := filepath.Join(ctx.ProjectPath, ".env")
 	if len(vars) > 0 {
 		if err := services.MergeDotEnv(envPath, "", vars); err != nil {
 			return "", fmt.Errorf("merge service env: %w", err)
 		}
 	}
 	proj = ctx.Registry.Find(ctx.ProjectName)
 	if proj != nil && proj.Services != nil && proj.Services.Postgres != "" {
 		if err := UpdateProjectEnvForPostgres(ctx.ProjectPath, ctx.ProjectName, proj.Services.Postgres, proj.Services); err != nil {
 			ui.Subtle(fmt.Sprintf("Could not write postgres env vars: %v", err))
 		}
 	}
 	if proj != nil && proj.Services != nil && proj.Services.MySQL != "" {
 		if err := UpdateProjectEnvForMysql(ctx.ProjectPath, ctx.ProjectName, proj.Services.MySQL, proj.Services); err != nil {
 			ui.Subtle(fmt.Sprintf("Could not write mysql env vars: %v", err))
 		}
 	}
+	if proj != nil && proj.Services != nil && proj.Services.Redis {
+		if err := UpdateProjectEnvForRedis(ctx.ProjectPath, ctx.ProjectName, proj.Services); err != nil {
+			ui.Subtle(fmt.Sprintf("Could not write redis env vars: %v", err))
+		}
+	}
 	if len(vars) == 0 {
 		return "no env vars to set", nil
 	}
 	return fmt.Sprintf("set %d service env vars", len(vars)), nil
 }
```

- [ ] **Step 2: Verify build + tests**

```bash
gofmt -w internal/laravel/
go vet ./...
go build ./...
go test ./internal/laravel/ -v
```

- [ ] **Step 3: Commit**

```bash
git add internal/laravel/steps.go
git commit -m "feat(laravel): DetectServicesStep writes redis env vars when bound"
```

---

## Task 24: Auto-bind redis in `automation/steps/detect_services.go`

**Files:**
- Modify: `/Users/clovismuneza/Apps/pv/internal/automation/steps/detect_services.go`
- Modify: `/Users/clovismuneza/Apps/pv/internal/automation/steps/detect_services_test.go`

Currently the redis branch in `Run` consults the docker registry (`findServiceByName(reg, "redis")`). After we delete `internal/services/redis.go` (Task 26), that lookup will always return empty. Replace it with a check on `redis.IsInstalled()`. Unconditional auto-bind on every Laravel project — no `.env` heuristic.

- [ ] **Step 1: Write/update failing test**

Find the existing `detect_services_test.go` and add (or modify) a redis test case to assert: when `redis.IsInstalled()` returns true, every Laravel project gets `Services.Redis = true`, regardless of `.env` content.

```go
func TestDetectServices_AutoBindsRedisWhenInstalled(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	// Pre-stage redis as installed.
	if err := os.MkdirAll(config.RedisDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(config.RedisDir(), "redis-server"), []byte{}, 0o755); err != nil {
		t.Fatal(err)
	}

	dir := t.TempDir()
	envPath := filepath.Join(dir, ".env")
	// .env has NO REDIS_HOST — auto-bind must trigger anyway (mirrors mailpit/rustfs).
	if err := os.WriteFile(envPath, []byte("APP_NAME=test\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{},
		Projects: []registry.Project{
			{Name: "test", Path: dir, Type: "laravel"},
		},
	}

	ctx := &automation.Context{
		ProjectName: "test",
		ProjectPath: dir,
		ProjectType: "laravel",
		Registry:    reg,
	}

	step := &DetectServicesStep{}
	if _, err := step.Run(ctx); err != nil {
		t.Fatalf("Run: %v", err)
	}
	if reg.Projects[0].Services == nil || !reg.Projects[0].Services.Redis {
		t.Errorf("project should have Redis=true after detect when redis is installed")
	}
}
```

Add imports: `"github.com/prvious/pv/internal/config"`, `"github.com/prvious/pv/internal/automation"`, `"github.com/prvious/pv/internal/registry"`, `"path/filepath"`, `"os"`, `"testing"`.

- [ ] **Step 2: Run test, confirm failure**

```bash
go test ./internal/automation/steps/ -v -run TestDetectServices_AutoBindsRedis
```

- [ ] **Step 3: Modify `internal/automation/steps/detect_services.go`**

Apply this unified diff:

```diff
@@ import (
 	"github.com/prvious/pv/internal/automation"
 	"github.com/prvious/pv/internal/mysql"
 	"github.com/prvious/pv/internal/postgres"
+	"github.com/prvious/pv/internal/redis"
 	"github.com/prvious/pv/internal/registry"
 	"github.com/prvious/pv/internal/services"
 	"github.com/prvious/pv/internal/ui"
 )
@@ func (s *DetectServicesStep) Run(ctx *automation.Context) (string, error) {
 	envPath := filepath.Join(ctx.ProjectPath, ".env")
 	envVars, err := services.ReadDotEnv(envPath)
 	if err != nil {
 		if os.IsNotExist(err) {
 			return "no .env found", nil
 		}
 		ui.Subtle(fmt.Sprintf("Could not read %s: %v", envPath, err))
 		return "skipped (.env unreadable)", nil
 	}
 
 	var bound int
 	dbName := services.SanitizeProjectName(ctx.ProjectName)
@@
 	if envVars["DB_CONNECTION"] == "mysql" {
 		versions, err := mysql.InstalledVersions()
 		if err == nil && len(versions) > 0 {
 			version := versions[len(versions)-1]
 			bindProjectMysql(ctx.Registry, ctx.ProjectName, version)
 			bound++
 		} else {
 			ui.Subtle("mysql detected but not installed. Run: pv mysql:install")
 		}
 	}
 
+	// Redis auto-bind: unconditional on every Laravel project once redis
+	// is installed. No .env heuristic — redis-as-cache/session is the
+	// path of least surprise in Laravel; mirrors mailpit/rustfs.
+	if redis.IsInstalled() {
+		bindProjectService(ctx.Registry, ctx.ProjectName, "redis", "redis")
+		bound++
+	}
+
 	type probe struct {
 		match  bool
 		name   string
 		addCmd string
 	}
 
 	probes := []probe{
-		{envVars["REDIS_HOST"] != "", "redis", "pv service:add redis"},
 		{
 			func() bool {
 				h := envVars["MAIL_HOST"]
 				return h != "" && (strings.Contains(h, "localhost") || strings.Contains(h, "127.0.0.1"))
 			}(),
 			"mail", "pv mailpit:install",
 		},
 		{
 			func() bool {
 				e := envVars["AWS_ENDPOINT"]
 				return e != "" && (strings.Contains(e, "localhost") || strings.Contains(e, "127.0.0.1"))
 			}(),
 			"s3", "pv rustfs:install",
 		},
 	}
```

The `bindProjectService` helper (already in this file) handles `case "redis"` by setting `Services.Redis = true`, so no signature change is needed.

- [ ] **Step 4: Run test, confirm pass**

```bash
go test ./internal/automation/steps/ -v
```

- [ ] **Step 5: Commit**

```bash
gofmt -w internal/automation/steps/
go vet ./...
go build ./...
git add internal/automation/steps/detect_services.go internal/automation/steps/detect_services_test.go
git commit -m "feat(automation): auto-bind redis on Laravel link when installed"
```

---

## Task 25: Wire `pv install` / `pv update` / `pv uninstall` orchestrators

**Files:**
- Modify: `/Users/clovismuneza/Apps/pv/cmd/update.go`
- Modify: `/Users/clovismuneza/Apps/pv/cmd/uninstall.go`
- Modify: `/Users/clovismuneza/Apps/pv/cmd/setup.go`
- Modify: `/Users/clovismuneza/Apps/pv/cmd/install.go` (drop `service[redis:...]` parser leftovers)
- Modify: `/Users/clovismuneza/Apps/pv/cmd/install_test.go`
- Modify: `/Users/clovismuneza/Apps/pv/cmd/setup_test.go`

`pv install` is wizard-gated (the wizard checkbox handles redis install). `pv update` iterates installed binary services and triggers their update; redis joins that pass. `pv uninstall` calls `UninstallForce()` on installed redis.

- [ ] **Step 1: Add redis pass to `cmd/update.go`**

Apply this unified diff:

```diff
@@ import (
 	mysqlCmds "github.com/prvious/pv/internal/commands/mysql"
 	"github.com/prvious/pv/internal/commands/postgres"
 	postgresCmds "github.com/prvious/pv/internal/commands/postgres"
+	rediscmd "github.com/prvious/pv/internal/commands/redis"
 	my "github.com/prvious/pv/internal/mysql"
 	pg "github.com/prvious/pv/internal/postgres"
+	r "github.com/prvious/pv/internal/redis"
 )
@@
 		// Update each installed mysql version. Mirrors the postgres pass — fetches
 		// the rolling artifact and atomic-replaces the binary tree per version.
 		if versions, err := my.InstalledVersions(); err == nil {
 			for _, version := range versions {
 				if err := mysqlCmds.RunUpdate([]string{version}); err != nil {
 					if !errors.Is(err, ui.ErrAlreadyPrinted) {
 						ui.Fail(fmt.Sprintf("MySQL %s update failed: %v", version, err))
 					}
 					failures = append(failures, "MySQL "+version)
 				}
 			}
 		}
+
+		// Update redis (single-version). Skip if not installed — redis is
+		// opt-in via `pv redis:install`.
+		if r.IsInstalled() {
+			if err := rediscmd.RunUpdate(nil); err != nil {
+				if !errors.Is(err, ui.ErrAlreadyPrinted) {
+					ui.Fail(fmt.Sprintf("Redis update failed: %v", err))
+				}
+				failures = append(failures, "Redis")
+			}
+		}
```

(Verify import alphabetization within the second group.)

- [ ] **Step 2: Add redis pass to `cmd/uninstall.go`**

Apply this unified diff:

```diff
@@ import (
 	mysqlCmds "github.com/prvious/pv/internal/commands/mysql"
 	postgresCmds "github.com/prvious/pv/internal/commands/postgres"
+	rediscmd "github.com/prvious/pv/internal/commands/redis"
 	my "github.com/prvious/pv/internal/mysql"
 	pg "github.com/prvious/pv/internal/postgres"
+	r "github.com/prvious/pv/internal/redis"
 )
@@
 		// Mysql uninstall (per installed version). Removes data dirs, binaries,
 		// state. User has already consented to a full pv uninstall.
 		if versions, err := my.InstalledVersions(); err == nil {
 			for _, version := range versions {
 				if err := mysqlCmds.UninstallForce(version); err != nil {
 					hadFailures = true
 					if !errors.Is(err, ui.ErrAlreadyPrinted) {
 						ui.Fail(fmt.Sprintf("mysql %s uninstall failed: %v", version, err))
 					}
 				}
 			}
 		}
+
+		// Redis uninstall (single-version). Removes data dir, binary, state.
+		// User has already consented to a full pv uninstall.
+		if r.IsInstalled() {
+			if err := rediscmd.UninstallForce(); err != nil {
+				hadFailures = true
+				if !errors.Is(err, ui.ErrAlreadyPrinted) {
+					ui.Fail(fmt.Sprintf("redis uninstall failed: %v", err))
+				}
+			}
+		}
```

- [ ] **Step 3: Add wizard checkbox + install hook in `cmd/setup.go`**

Apply this unified diff:

```diff
@@ import (
 	"github.com/prvious/pv/internal/commands/mago"
 	mysqlcmd "github.com/prvious/pv/internal/commands/mysql"
+	rediscmd "github.com/prvious/pv/internal/commands/redis"
 	"github.com/prvious/pv/internal/config"
 	"github.com/prvious/pv/internal/mysql"
+	"github.com/prvious/pv/internal/redis"
 )
@@
 		// Tool options.
 		toolOpts := []selectOption{
 			{label: "Mago (PHP linter & formatter)", value: "mago", selected: isExecutable(config.BinDir() + "/mago")},
 			{label: "MySQL 8.4 (LTS, native binary)", value: "mysql-8.4", selected: mysql.IsInstalled("8.4")},
+			{label: "Redis (native binary)", value: "redis", selected: redis.IsInstalled()},
 		}
@@
 		if toolSet["mysql-8.4"] {
 			if err := mysqlcmd.RunInstall([]string{"8.4"}); err != nil {
 				if !errors.Is(err, ui.ErrAlreadyPrinted) {
 					ui.Fail(fmt.Sprintf("MySQL 8.4 install failed: %v", err))
 				}
 			}
 		}
+
+		if toolSet["redis"] {
+			if err := rediscmd.RunInstall(nil); err != nil {
+				if !errors.Is(err, ui.ErrAlreadyPrinted) {
+					ui.Fail(fmt.Sprintf("Redis install failed: %v", err))
+				}
+			}
+		}
```

- [ ] **Step 4: Drop `service[redis:...]` cases in `cmd/install_test.go`**

The current tests reference `service[redis:7]` from when redis was a docker service. Since the docker registry is being emptied (Task 26), these test cases are stale. Replace `redis` with `mail` (the remaining docker → binary service that still parses through `service[...]`) or remove the cases.

Edit `/Users/clovismuneza/Apps/pv/cmd/install_test.go`:

- Find line 106: `spec, err := parseWith("service[redis:7],service[mail]")` — change to `parseWith("service[mail]")` and adjust the assertion accordingly.
- Find line 113: `if spec.services[0].name != "redis" || spec.services[0].version != "7"` — drop or change to mail.
- Find line 122: `parseWith("php:8.3,mago,service[redis:7],service[mail]")` — change to `parseWith("php:8.3,mago,service[mail]")` and adjust assertions.
- Find line 152: `parseWith("service[redis]")` — change to `parseWith("service[mail]")` and adjust assertions.
- Find line 159: assertion on `redis` → `mail`.

(If `service[redis:...]` parsing was the only parsing test for the `:<version>` syntax, keep one such test against `mail:1.20` so the parser path stays covered.)

- [ ] **Step 5: Drop redis from `cmd/setup_test.go`**

Edit `/Users/clovismuneza/Apps/pv/cmd/setup_test.go` line 16:

```go
// Was: want := []string{"redis", "mail", "s3"}
// After Task 26 removes docker redis, the docker registry is empty.
// `mail` and `s3` remain (binary services). Adjust to:
want := []string{"mail", "s3"}
```

(Update the comment on lines 13–14 to reflect that the docker registry is now empty; binary services remain.)

- [ ] **Step 6: Verify build + tests**

```bash
gofmt -w cmd/
go vet ./...
go build ./...
go test ./cmd/ -v -run TestParseWith
go test ./cmd/ -v -run TestSetup
```

Note: `cmd/install.go` itself doesn't currently have a `mysql` orchestrator pass beyond the wizard; redis follows the same pattern (wizard-gated, no orchestrator pass needed in `cmd/install.go` proper).

- [ ] **Step 7: Commit**

```bash
git add cmd/update.go cmd/uninstall.go cmd/setup.go cmd/install_test.go cmd/setup_test.go
git commit -m "feat(cmd): wire redis into update/uninstall orchestrators + setup wizard"
```

---

## Task 26: Delete docker `services.Redis`

**Files:**
- Delete: `/Users/clovismuneza/Apps/pv/internal/services/redis.go`
- Delete: `/Users/clovismuneza/Apps/pv/internal/services/redis_test.go`
- Modify: `/Users/clovismuneza/Apps/pv/internal/services/service.go`
- Modify: `/Users/clovismuneza/Apps/pv/internal/services/lookup_test.go`
- Modify: `/Users/clovismuneza/Apps/pv/internal/services/service_test.go`

The docker registry becomes empty. `services.Lookup("redis")` must now return its existing "unknown service" error (no special-casing per spec Q4/C). Some tests need their assertions adjusted to expect an empty docker map.

- [ ] **Step 1: Delete the docker redis files**

```bash
git rm internal/services/redis.go internal/services/redis_test.go
```

- [ ] **Step 2: Drop `"redis"` from the docker registry map in `internal/services/service.go`**

Apply this unified diff to `/Users/clovismuneza/Apps/pv/internal/services/service.go`:

```diff
-var registry = map[string]Service{
-	"redis": &Redis{},
-}
+// Docker registry — currently empty. Postgres and MySQL migrated to
+// native binaries (PR #75 / PR #80); Redis migrated in this PR. The
+// registry stays as a map so callers (Lookup, Available) continue to
+// compile and operate over the empty set.
+var registry = map[string]Service{}
```

- [ ] **Step 3: Update `internal/services/lookup_test.go`**

The current test asserts `LookupAny("redis")` succeeds. After this change it must error (or return the unknown-service path). Adjust the test to match the new contract — `LookupAny("redis")` returns an "unknown service" error.

Read the existing test (around line 25) and modify the assertion. Concretely, replace:

```go
kind, binSvc, docSvc, err := LookupAny("redis")
if err != nil {
    t.Fatalf("LookupAny(\"redis\") error = %v", err)
}
// ... assertion that returns kind=docker, docSvc != nil
```

with:

```go
_, _, _, err := LookupAny("redis")
if err == nil {
    t.Error("LookupAny(\"redis\") should error after docker redis removal")
}
```

(If the test had multiple assertions tied to the docker shape, drop those branches and keep the new error assertion.)

- [ ] **Step 4: Update `internal/services/service_test.go`**

Adjust assertions that reference docker redis:

- Line ~10: `for _, name := range []string{"redis"}` — drop the loop entirely (no docker services to iterate). Or replace with a comment: `// docker registry is empty; nothing to iterate.`
- Line ~34: `{"redis", "", "redis"}` and `{"redis", "latest", "redis"}` — drop these test rows (the surrounding test was likely a `ServiceKey` parametric — keep the mysql/postgres rows that exercise the same code path).
- Line ~48: comment "1 Docker service (redis) + 2 binary services (s3, mail)" — update to "0 Docker services + 2 binary services (s3, mail)".
- Line ~63: `{"redis", "redis", "latest"}` — drop the row.

- [ ] **Step 5: Verify build + tests**

```bash
gofmt -w internal/services/
go vet ./...
go build ./...
go test ./internal/services/ -v
```

- [ ] **Step 6: Commit**

```bash
git add internal/services/
git commit -m "refactor(services): remove docker Redis (replaced by internal/redis)"
```

---

## Task 27: E2E test — redis lifecycle

**Files:**
- Create: `/Users/clovismuneza/Apps/pv/scripts/e2e/redis-binary.sh`
- Modify: `/Users/clovismuneza/Apps/pv/.github/workflows/e2e.yml`

Mirrors `scripts/e2e/mysql-binary.sh` but for the simpler single-version redis flow.

- [ ] **Step 1: Create `scripts/e2e/redis-binary.sh`**

```bash
#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

echo "==> Phase: Redis native-binary lifecycle"

# Start pv in foreground so the supervisor reconciles redis state.
sudo -E pv start >/tmp/pv-redis-e2e.log 2>&1 &
START_PID=$!
sleep 8

cleanup() {
  sudo -E pv unlink e2e-redis-env >/dev/null 2>&1 || true
  sudo -E pv redis:uninstall --force >/dev/null 2>&1 || true
  sudo -E pv stop >/dev/null 2>&1 || true
  rm -rf "${ENVTEST_DIR:-}" 2>/dev/null || true
}
trap cleanup EXIT

# Pre-link a Laravel project so the redis auto-bind path is exercised.
ENVTEST_DIR=$(mktemp -d)
echo '{"require":{"php":"^8.2","laravel/framework":"^11.0"}}' > "$ENVTEST_DIR/composer.json"
mkdir -p "$ENVTEST_DIR/public"
echo '<?php echo "test";' > "$ENVTEST_DIR/public/index.php"
echo "APP_NAME=test" > "$ENVTEST_DIR/.env"
sudo -E pv link "$ENVTEST_DIR" --name e2e-redis-env >/dev/null 2>&1 || { echo "FAIL: pv link"; exit 1; }

echo "==> redis:install"
sudo -E pv redis:install || { echo "FAIL: redis:install"; exit 1; }

echo "==> Verify binary tree exists"
test -x "$HOME/.pv/redis/redis-server" || { echo "FAIL: redis-server binary missing"; exit 1; }
test -x "$HOME/.pv/redis/redis-cli" || { echo "FAIL: redis-cli binary missing"; exit 1; }
echo "OK: redis binary tree present"

echo "==> Wait for port 6379 to accept connections"
wait_for_tcp 127.0.0.1 6379 30 || { echo "FAIL: 6379 not reachable"; exit 1; }
echo "OK: 6379 reachable"

echo "==> Verify daemon-status.json lists redis"
grep -q '"redis"' "$HOME/.pv/daemon-status.json" || { echo "FAIL: redis missing from daemon-status.json"; exit 1; }
echo "OK: daemon-status.json advertises redis"

echo "==> redis-cli PING"
PING=$("$HOME/.pv/redis/redis-cli" -h 127.0.0.1 -p 6379 PING | tr -d '[:space:]')
[ "$PING" = "PONG" ] || { echo "FAIL: PING returned '$PING', want 'PONG'"; exit 1; }
echo "OK: PING returned PONG"

echo "==> redis-cli SET/GET roundtrip"
"$HOME/.pv/redis/redis-cli" -h 127.0.0.1 -p 6379 SET pv_e2e_key "hello-world" >/dev/null
GOT=$("$HOME/.pv/redis/redis-cli" -h 127.0.0.1 -p 6379 GET pv_e2e_key)
[ "$GOT" = "hello-world" ] || { echo "FAIL: GET returned '$GOT', want 'hello-world'"; exit 1; }
echo "OK: SET/GET roundtrip"

echo "==> Verify pre-linked project got REDIS_HOST=127.0.0.1 (auto-bind retroactive)"
grep -q "REDIS_HOST=127.0.0.1" "$ENVTEST_DIR/.env" || {
    echo "FAIL: linked project .env should have REDIS_HOST=127.0.0.1";
    echo "  actual .env contents:";
    cat "$ENVTEST_DIR/.env";
    exit 1;
}
grep -q "REDIS_PORT=6379" "$ENVTEST_DIR/.env" || { echo "FAIL: missing REDIS_PORT=6379"; exit 1; }
grep -q "REDIS_PASSWORD=null" "$ENVTEST_DIR/.env" || { echo "FAIL: missing REDIS_PASSWORD=null"; exit 1; }
echo "OK: linked project .env has REDIS_*"

echo "==> redis:list shows the row"
LIST=$(sudo -E pv redis:list 2>&1)
echo "$LIST" | strip_ansi | grep -q "6379" || { echo "FAIL: list missing port 6379"; echo "$LIST"; exit 1; }
echo "OK: redis:list shows the row"

echo "==> redis:stop"
sudo -E pv redis:stop
for i in $(seq 1 10); do
    if ! nc -z 127.0.0.1 6379 2>/dev/null; then break; fi
    sleep 1
done
if nc -z 127.0.0.1 6379 2>/dev/null; then echo "FAIL: 6379 still answering after stop"; exit 1; fi
echo "OK: redis stopped"

echo "==> redis:start"
sudo -E pv redis:start
wait_for_tcp 127.0.0.1 6379 30 || { echo "FAIL: 6379 not reachable after start"; exit 1; }
echo "OK: redis back online"

echo "==> redis:uninstall --force"
sudo -E pv redis:uninstall --force
test ! -d "$HOME/.pv/redis" || { echo "FAIL: redis binary tree not removed"; exit 1; }
test ! -d "$HOME/.pv/data/redis" || { echo "FAIL: redis data dir not removed"; exit 1; }
echo "OK: redis fully removed"

echo "==> daemon-status.json no longer lists redis"
sleep 2
grep -q '"redis"' "$HOME/.pv/daemon-status.json" && { echo "FAIL: redis still in daemon-status after uninstall"; exit 1; } || true
echo "OK: redis cleared from daemon-status"

echo "==> pv stop"
sudo -E pv stop || true
trap - EXIT

echo "OK: Redis native-binary lifecycle passed"
```

- [ ] **Step 2: Make the script executable**

```bash
chmod +x scripts/e2e/redis-binary.sh
```

- [ ] **Step 3: Wire into `.github/workflows/e2e.yml`**

Apply this unified diff:

```diff
       # ── Phase 23: MySQL native-binary lifecycle ────────────────────
       - name: E2E — MySQL native-binary lifecycle
         timeout-minutes: 5
         run: scripts/e2e/mysql-binary.sh

+      # ── Phase 24: Redis native-binary lifecycle ────────────────────
+      - name: E2E — Redis native-binary lifecycle
+        timeout-minutes: 3
+        run: scripts/e2e/redis-binary.sh
+
-      # ── Phase 24: Uninstall ───────────────────────────────────────
+      # ── Phase 25: Uninstall ───────────────────────────────────────
       # TODO: frankenphp untrust hangs in CI (internal sudo prompt, no terminal)
       # - name: Test pv uninstall
       #   timeout-minutes: 1
       #   run: scripts/e2e/uninstall.sh

-      # ── Phase 25: Failure Diagnostics & Cleanup ────────────────────
+      # ── Phase 26: Failure Diagnostics & Cleanup ────────────────────
```

- [ ] **Step 4: Commit**

```bash
git add scripts/e2e/redis-binary.sh .github/workflows/e2e.yml
git commit -m "test(e2e): redis-binary lifecycle phase"
```

---

## Task 28: Extend `scripts/e2e/diagnostics.sh` with redis blocks

**Files:**
- Modify: `/Users/clovismuneza/Apps/pv/scripts/e2e/diagnostics.sh`

Append redis-specific dump blocks parallel to the existing mysql / postgres blocks. The existing `state.json` dump (in the postgres / mysql sections) already covers redis since `state.json` is a single shared file; we don't duplicate the cat.

- [ ] **Step 1: Append to `scripts/e2e/diagnostics.sh`**

Apply this unified diff (append after the mysql blocks around line 112):

```diff
@@
 echo "==> /tmp pv-mysql sockets/pids"
 ls -la /tmp/pv-mysql-* 2>/dev/null || echo "(no /tmp/pv-mysql files)"
+
+echo "==> redis log"
+if [ -f ~/.pv/logs/redis.log ]; then
+  echo "  -- ~/.pv/logs/redis.log --"
+  tail -200 ~/.pv/logs/redis.log
+else
+  echo "(no redis.log)"
+fi
+
+echo "==> redis data dir"
+ls -la ~/.pv/data/redis/ 2>/dev/null || echo "(no redis data dir)"
+
+echo "==> redis binary tree"
+ls -la ~/.pv/redis/ 2>/dev/null || echo "(no redis binary dir)"
+
+echo "==> /tmp pv-redis files"
+ls -la /tmp/pv-redis* 2>/dev/null || echo "(no /tmp/pv-redis files)"
+
+echo "==> redis e2e log"
+if [ -f /tmp/pv-redis-e2e.log ]; then
+  echo "  -- /tmp/pv-redis-e2e.log --"
+  tail -200 /tmp/pv-redis-e2e.log
+fi
```

If the existing diagnostics script has a section that loops `/tmp/pv-mysql-e2e.log /tmp/pv-postgres-e2e.log /tmp/pv-mail-e2e.log /tmp/pv-s3-e2e.log` together (around line 94), add `/tmp/pv-redis-e2e.log` to that list too — the standalone block above is fine if it doesn't.

- [ ] **Step 2: Commit**

```bash
git add scripts/e2e/diagnostics.sh
git commit -m "test(e2e): diagnostics dumps for redis"
```

---

## Task 29: End-to-end manual verification on macOS arm64

Final sanity pass. Manual checklist; no commit.

- [ ] **Step 1: Build pv from this branch**

```bash
go build -o /tmp/pv-redis-test .
```

- [ ] **Step 2: Sandbox into a temp HOME**

```bash
export HOME=$(mktemp -d)
mkdir -p "$HOME/.pv"
```

- [ ] **Step 3: Install redis**

```bash
/tmp/pv-redis-test redis:install
/tmp/pv-redis-test redis:list
```

Expected: `redis:list` shows one row with port 6379, status running.

- [ ] **Step 4: Start the daemon and verify supervision**

```bash
/tmp/pv-redis-test start &
sleep 5
cat "$HOME/.pv/daemon-status.json" | grep redis
```

Expected: `redis` is running with a non-zero PID.

- [ ] **Step 5: Connect via the bundled redis-cli**

```bash
"$HOME/.pv/redis/redis-cli" -h 127.0.0.1 -p 6379 PING
"$HOME/.pv/redis/redis-cli" -h 127.0.0.1 -p 6379 SET k v
"$HOME/.pv/redis/redis-cli" -h 127.0.0.1 -p 6379 GET k
```

Expected: `PONG`, `OK`, `v`.

- [ ] **Step 6: Daemon restart preserves state**

```bash
/tmp/pv-redis-test stop
/tmp/pv-redis-test start &
sleep 5
"$HOME/.pv/redis/redis-cli" -h 127.0.0.1 -p 6379 PING
```

Expected: `PONG` (state.json says wanted=running, supervisor brings it back).

- [ ] **Step 7: Link a Laravel project**

```bash
PROJ=$(mktemp -d)
echo '{"require":{"laravel/framework":"^11.0"}}' > "$PROJ/composer.json"
mkdir -p "$PROJ/public"
echo '<?php echo "test";' > "$PROJ/public/index.php"
echo "APP_NAME=test" > "$PROJ/.env"

/tmp/pv-redis-test link "$PROJ" --name verify-redis
grep -E "^REDIS_" "$PROJ/.env"
```

Expected: `.env` now contains `REDIS_HOST=127.0.0.1`, `REDIS_PORT=6379`, `REDIS_PASSWORD=null`.

- [ ] **Step 8: Persistence — restart redis and verify keys survive**

```bash
"$HOME/.pv/redis/redis-cli" -h 127.0.0.1 -p 6379 SET persist_key persist_value
"$HOME/.pv/redis/redis-cli" -h 127.0.0.1 -p 6379 SAVE   # force RDB snapshot
/tmp/pv-redis-test redis:restart
sleep 3
GOT=$("$HOME/.pv/redis/redis-cli" -h 127.0.0.1 -p 6379 GET persist_key)
echo "Persisted value: $GOT"
```

Expected: `Persisted value: persist_value`. (RDB at `~/.pv/data/redis/dump.rdb` is reloaded on restart.)

- [ ] **Step 9: Uninstall and verify cleanup**

```bash
/tmp/pv-redis-test redis:uninstall --force
ls "$HOME/.pv/redis/" 2>/dev/null
ls "$HOME/.pv/data/redis/" 2>/dev/null
cat "$HOME/.pv/data/state.json"
```

Expected: directories under `~/.pv/redis/` and `~/.pv/data/redis/` are gone; `state.json` no longer has a `"redis"` slice.

- [ ] **Step 10: All clean — done**

No commit; this is the manual verification gate before merge.

---

## Self-Review

**Spec coverage** (cross-checked against `docs/superpowers/specs/2026-05-09-redis-native-binary-design.md`):

| Spec section | Plan task(s) |
|---|---|
| Locked decision: bind 127.0.0.1, --protected-mode no, no auth | Task 15 (BuildSupervisorProcess flags) |
| Locked decision: single version, upstream-tracked | Task 1 (verify), Task 3 (URL has no version arg), Task 4 (constant port), Task 6 (flat state) |
| Locked decision: RDB persistence, dump.rdb in ~/.pv/data/redis/ | Task 2 (RedisDataDir), Task 15 (--dir / --dbfilename / --appendonly no) |
| Locked decision: port 6379 | Task 4 |
| Locked decision: docker redis removed | Task 26 |
| Locked decision: explicit install | Task 19 (cobra command), Task 25 (wizard checkbox) |
| Locked decision: internal/redis/ mirroring postgres/mysql | Tasks 4–16 |
| Locked decision: redis:* only, no aliases | Task 18 (Register has no aliases) |
| Locked decision: pv link auto-bind unconditional once installed | Task 24 (detect_services unconditional bind) |
| Locked decision: ProjectServices.Redis bool reused | Tasks 13, 16, 24 (no new field added) |
| Locked decision: setup wizard checkbox | Task 25 |
| Locked decision: service:* unchanged | Task 26 (only drops `"redis"` from registry; commands package untouched) |
| Architecture: package layout (internal/redis/, internal/commands/redis/, cmd/redis.go) | Tasks 2–22 |
| Architecture: reconciler 4-source wanted set | Task 17 |
| Filesystem: ~/.pv/redis/, ~/.pv/data/redis/, ~/.pv/logs/redis.log | Task 2 (path helpers) |
| Boot flags (no redis.conf), no --logfile | Task 15 |
| State file redis slice (flat, single-record) | Task 6 |
| Install flow (download → extract → state) | Task 12 |
| Uninstall flow (force vs non-force, UnbindService("redis")) | Task 13 |
| Update flow (datadir untouched, prior wanted preserved) | Task 14 |
| Auto-bind on install (BindLinkedProjects) | Task 16 (impl) + Task 19 (called from install command) |
| Auto-bind on link (DetectServicesStep redis branch) | Task 24 |
| EnvVars(projectName) — no error, ignores projectName | Task 11 |
| UpdateProjectEnvForRedis | Task 22 |
| Removal of docker redis (delete files, drop registry entry, fix tests) | Task 26 |
| E2E phase | Task 27 |
| Diagnostics extension | Task 28 |
| Manual verification | Task 29 |
| Migration / rollout: orchestrator wiring (pv update / pv uninstall / setup) | Task 25 |

**Placeholder scan:** none. Every task references concrete files, function names, and exports defined in earlier tasks. Task 1 is research-only with a documented halt-condition (artifact missing → dispatch + halt) — that's an explicit prerequisite, not a TBD.

**Type / symbol consistency check** (against tasks in order):

- `config.RedisDir() / RedisDataDir() / RedisLogPath()` — Task 2; used in Tasks 5 (path joins), 12 (install paths), 13 (uninstall removal), 14 (update staging), 15 (process flags), 16 (db test), 17 (reconciler test), 21 (list/logs), 27 (e2e shell), 28 (diagnostics).
- `binaries.RedisURL() (string, error)` — Task 3; used in Task 12 (resolveRedisURL fallback), Task 14 (resolveRedisURL fallback).
- `redis.PortFor() int` — Task 4; used in Tasks 10 (waitstopped addr), 11 (envvars), 15 (process flags), 21 (list/status output).
- `redis.IsInstalled() bool` — Task 5; used in Tasks 7 (wanted intersection), 12 (idempotency), 13 (pre-uninstall check), 14 (pre-update check), 15 (process refusal), 19 (install idempotent), 20 (start/uninstall guards), 21 (list short-circuit), 22 (laravel update path? no — laravel doesn't gate on it; the bind is the gate), 24 (detect_services), 25 (orchestrators).
- `redis.ServerBinary() string` / `redis.CLIBinary() string` — Task 5; used in Task 8 (probe), Task 15 (process binary).
- `redis.LoadState() (State, error)` / `SaveState` / `SetWanted(wanted)` / `RemoveState` / `WantedRunning` / `WantedStopped` — Task 6; used in Tasks 7 (wanted), 12 (install final), 13 (uninstall), 14 (update prev/restore), 19/20 (start/stop/restart), 21 (list status column).
- `redis.IsWanted() bool` — Task 7; used in Task 17 (reconciler).
- `redis.ProbeVersion() (string, error)` — Task 8; used in Tasks 12 (record), 14 (re-record).
- `redis.WaitStopped(timeout)` — Task 10; used in Tasks 13 (uninstall), 14 (update), 20 (uninstall/update/restart cobra).
- `redis.EnvVars(projectName) map[string]string` — Task 11; used in Task 22 (laravel UpdateProjectEnvForRedis), Task 16 test fake.
- `redis.Install(client) / InstallProgress(client, progress)` — Task 12; used in Task 19 (download cobra → install pipeline).
- `redis.Uninstall(force bool) error` — Task 13; used in Task 20 (uninstall cobra).
- `redis.Update(client) / UpdateProgress(client, progress)` — Task 14; used in Task 20 (update cobra), Task 25 (orchestrator).
- `redis.BuildSupervisorProcess() (supervisor.Process, error)` — Task 15; used in Task 17 (reconciler).
- `redis.BindLinkedProjects() error` — Task 16; used in Task 19 (install cobra).
- `redis.EnvWriter` callback — Task 16; wired in Task 19's `init()` to call `laravel.UpdateProjectEnvForRedis`.
- `laravel.UpdateProjectEnvForRedis(projectPath, projectName, *ProjectServices) error` — Task 22; consumed by Task 19 (init), Task 23 (DetectServicesStep).
- `bindProjectService(reg, projectName, "redis", "redis")` — pre-existing helper in `internal/automation/steps/detect_services.go`; consumed by Task 24.
- `rediscmd.RunInstall / RunUpdate / RunUninstall / UninstallForce` — Task 18 (register.go); used in Task 25 (cmd/update.go, cmd/uninstall.go, cmd/setup.go).

All references resolve. No symbols are introduced and then orphaned; no symbols are referenced before being defined when reading the plan in order.

**Scope:** All 29 tasks contribute to the same coherent change — replacing docker redis with native redis across the registry, link pipeline, setup wizard, orchestrators, and CI. The set is complete (every spec "Architecture", "Data flows", and "Removal of docker redis" subsection has at least one task) and minimal (no task introduces machinery the spec doesn't require). Not splittable without breaking the build mid-way: Task 22's laravel helper is referenced from Task 19's cobra `init()`, so Task 19 cannot land before Task 22 (the plan commits them together at the end of Task 22). Task 26's docker-redis removal must follow Task 24's auto-bind switch (which stops consulting `findServiceByName(reg, "redis")`).

Zero gaps.
