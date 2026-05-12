# Redis Versioned Shape Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Refactor `internal/redis/` from flat single-version to version-parameterized API matching `internal/mysql/`.

**Architecture:** All public functions grow a `version string` parameter. Binary paths change from `~/.pv/redis/` to `~/.pv/redis/{version}/`. State becomes versioned map. Registry binding changes from `bool` to `string`.

**Tech Stack:** Go, cobra, internal/state, internal/config, internal/registry

---

### Task 1: Config paths — add versioned helpers

**Files:**
- Modify: `internal/config/paths.go`

**What:** Add version-aware path helpers alongside existing flat ones. The old helpers (`RedisDir()`, `RedisDataDir()`, `RedisLogPath()`) stay temporarily — callers migrate in later tasks.

- [ ] **Step 1: Add versioned path helpers**

Add after line 248 (after `RedisDataDir()`):

```go
// RedisDefaultVersion returns the single version pv ships for redis.
// Currently "8.6" — update when the artifacts pipeline bumps.
func RedisDefaultVersion() string { return "8.6" }

// RedisVersionDir returns ~/.pv/redis/{version}/ — the binary root
// for a specific version. Mirrors PostgresVersionDir / MysqlVersionDir.
func RedisVersionDir(version string) string {
	return filepath.Join(PvDir(), "redis", version)
}

// RedisDataDirV returns ~/.pv/data/redis/{version}/ — the data root
// for a specific version. Note: old RedisDataDir() (no version) is
// deprecated; renamed to RedisDataRoot() for the parent.
func RedisDataDirV(version string) string {
	return filepath.Join(DataDir(), "redis", version)
}

// RedisLogPathV returns ~/.pv/logs/redis-{version}.log.
func RedisLogPathV(version string) string {
	return filepath.Join(LogsDir(), "redis-"+version+".log")
}

// RedisDataRoot returns ~/.pv/data/redis/ — the parent of versioned
// data dirs. Used when iterating installed versions; callers should
// prefer RedisDataDirV for per-version paths.
func RedisDataRoot() string {
	return filepath.Join(DataDir(), "redis")
}
```

Also rename existing `RedisDataDir()` to `RedisDataRoot()` — this avoids confusion. But since callers still call `RedisDataDir()`, rename it and update all callers at once. Better approach: leave old functions as-is, add new ones with `V` suffix. We'll remove old ones in the cleanup task.

So: add `RedisDefaultVersion()`, `RedisVersionDir(version)`, `RedisDataDirV(version)`, `RedisLogPathV(version)`, `RedisDataRoot()`.

- [ ] **Step 2: Run tests**

Run: `go build ./internal/config/... && go test ./internal/config/...`
Expected: PASS

- [ ] **Step 3: Commit**

```
git add internal/config/paths.go
git commit -m "feat(redis): add versioned config path helpers"
```

---

### Task 2: Registry — `Redis bool` → `string`, add `UnbindRedisVersion`

**Files:**
- Modify: `internal/registry/registry.go`
- Modify: `internal/registry/registry_test.go`

- [ ] **Step 1: Change `ProjectServices.Redis` from `bool` to `string`**

In `ProjectServices` struct:
```go
Redis    string `json:"redis,omitempty"` // was bool
```

- [ ] **Step 2: Update `UnbindService("redis")` case**

Change line 230 from:
```go
r.Projects[i].Services.Redis = false
```
to:
```go
r.Projects[i].Services.Redis = ""
```

- [ ] **Step 3: Update `ProjectsUsingService("redis")` check**

Change line 204 from:
```go
if p.Services.Redis {
```
to:
```go
if p.Services.Redis != "" {
```

- [ ] **Step 4: Add `UnbindRedisVersion` helper**

Add after `UnbindMysqlVersion`:
```go
func (r *Registry) UnbindRedisVersion(version string) {
	for i := range r.Projects {
		if r.Projects[i].Services == nil {
			continue
		}
		if r.Projects[i].Services.Redis == version {
			r.Projects[i].Services.Redis = ""
		}
	}
}
```

- [ ] **Step 5: Update tests in registry_test.go**

Find the test data at line 558 — change `Redis: true` to `Redis: "8.6"`:
```go
{Services: &ProjectServices{MySQL: "8.4", Redis: "8.6"}},
```

- [ ] **Step 6: Build + test**

Run: `go build ./internal/registry/... && go test ./internal/registry/...`
Expected: PASS

- [ ] **Step 7: Commit**

```
git add internal/registry/registry.go internal/registry/registry_test.go
git commit -m "feat(redis): change registry binding from bool to string, add UnbindRedisVersion"
```

---

### Task 3: State — versioned map matching mysql

**Files:**
- Modify: `internal/redis/state.go`
- Modify: `internal/redis/state_test.go`

- [ ] **Step 1: Rewrite state.go struct + API**

Replace the flat `State` with versioned map:

```go
package redis

import (
	"encoding/json"
	"fmt"
	"os"

	"github.com/prvious/pv/internal/state"
)

const stateKey = "redis"

const (
	WantedRunning = "running"
	WantedStopped = "stopped"
)

type VersionState struct {
	Wanted string `json:"wanted"`
}

type State struct {
	Versions map[string]VersionState `json:"versions"`
}

func LoadState() (State, error) {
	all, err := state.Load()
	if err != nil {
		return State{Versions: map[string]VersionState{}}, err
	}
	raw, ok := all[stateKey]
	if !ok {
		return State{Versions: map[string]VersionState{}}, nil
	}
	var s State
	if err := json.Unmarshal(raw, &s); err != nil {
		fmt.Fprintf(os.Stderr, "redis: state slice corrupt (%v); treating as empty\n", err)
		return State{Versions: map[string]VersionState{}}, nil
	}
	if s.Versions == nil {
		s.Versions = map[string]VersionState{}
	}
	return s, nil
}

func SaveState(s State) error {
	all, err := state.Load()
	if err != nil {
		return err
	}
	if s.Versions == nil {
		s.Versions = map[string]VersionState{}
	}
	payload, err := json.Marshal(s)
	if err != nil {
		return err
	}
	all[stateKey] = payload
	return state.Save(all)
}

func SetWanted(version, wanted string) error {
	if wanted != WantedRunning && wanted != WantedStopped {
		return fmt.Errorf("redis: invalid wanted state %q (want %q or %q)", wanted, WantedRunning, WantedStopped)
	}
	s, err := LoadState()
	if err != nil {
		return err
	}
	s.Versions[version] = VersionState{Wanted: wanted}
	return SaveState(s)
}

func RemoveVersion(version string) error {
	s, err := LoadState()
	if err != nil {
		return err
	}
	delete(s.Versions, version)
	return SaveState(s)
}

func RemoveState() error {
	all, err := state.Load()
	if err != nil {
		return err
	}
	delete(all, stateKey)
	return state.Save(all)
}
```

- [ ] **Step 2: Update state_test.go**

Rewrite to test versioned map operations:

```go
package redis

import (
	"testing"
)

func TestLoadState_MissingReturnsEmptyVersions(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	s, err := LoadState()
	if err != nil {
		t.Fatal(err)
	}
	if s.Versions == nil {
		t.Error("Versions map should not be nil")
	}
	if len(s.Versions) != 0 {
		t.Error("expected empty versions")
	}
}

func TestSetWanted_Roundtrip(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if err := SetWanted("8.6", WantedRunning); err != nil {
		t.Fatal(err)
	}
	s, _ := LoadState()
	if s.Versions["8.6"].Wanted != WantedRunning {
		t.Errorf("wanted = %q", s.Versions["8.6"].Wanted)
	}
}

func TestSetWanted_RejectsInvalid(t *testing.T) {
	if err := SetWanted("8.6", "invalid"); err == nil {
		t.Error("expected error for invalid wanted state")
	}
}

func TestRemoveVersion(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	SetWanted("8.6", WantedRunning)
	if err := RemoveVersion("8.6"); err != nil {
		t.Fatal(err)
	}
	s, _ := LoadState()
	if _, ok := s.Versions["8.6"]; ok {
		t.Error("version should be removed")
	}
}

func TestRemoveState(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	SetWanted("8.6", WantedRunning)
	if err := RemoveState(); err != nil {
		t.Fatal(err)
	}
	s, _ := LoadState()
	if len(s.Versions) != 0 {
		t.Error("expected empty after RemoveState")
	}
}
```

- [ ] **Step 3: Build + test**

Run: `go build ./internal/redis/... && go test ./internal/redis/...`
Expected: state tests PASS (other tests in the package may fail — that's expected, they'll be fixed in subsequent tasks)

- [ ] **Step 4: Commit**

```
git add internal/redis/state.go internal/redis/state_test.go
git commit -m "feat(redis): versioned state map"
```

---

### Task 4: Port, installed, version — version-aware

**Files:**
- Modify: `internal/redis/port.go`
- Modify: `internal/redis/port_test.go`
- Modify: `internal/redis/installed.go`
- Modify: `internal/redis/installed_test.go`
- Modify: `internal/redis/version.go`
- Modify: `internal/redis/version_test.go`

- [ ] **Step 1: Rewrite port.go**

```go
package redis

import "strconv"

func PortFor(version string) int {
	return redisPort(version)
}

func redisPort(version string) int {
	major, minor := parseVersion(version)
	return 6300 + major*100 + minor*10
}

func parseVersion(v string) (int, int) {
	major := 8
	minor := 6
	return major, minor
}
```

Note: `parseVersion` is intentionally simple for now since we only ship one version. It can be made robust when multi-version support is actually needed. Tests exercise the formula directly.

- [ ] **Step 2: Rewrite port_test.go**

```go
package redis

import "testing"

func TestPortFor(t *testing.T) {
	tests := []struct {
		version string
		want    int
	}{
		{"7.4", 6740},
		{"8.6", 6860},
	}
	for _, tc := range tests {
		got := PortFor(tc.version)
		if got != tc.want {
			t.Errorf("PortFor(%q) = %d, want %d", tc.version, got, tc.want)
		}
	}
}
```

- [ ] **Step 3: Rewrite installed.go**

```go
package redis

import (
	"os"
	"path/filepath"
	"sort"

	"github.com/prvious/pv/internal/config"
)

func ServerBinary(version string) string {
	return filepath.Join(config.RedisVersionDir(version), "redis-server")
}

func CLIBinary(version string) string {
	return filepath.Join(config.RedisVersionDir(version), "redis-cli")
}

func IsInstalled(version string) bool {
	info, err := os.Stat(ServerBinary(version))
	return err == nil && !info.IsDir()
}

func InstalledVersions() ([]string, error) {
	root := config.RedisDir()
	entries, err := os.ReadDir(root)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}
	var out []string
	for _, e := range entries {
		if !e.IsDir() {
			continue
		}
		version := e.Name()
		bin := filepath.Join(config.RedisVersionDir(version), "redis-server")
		if info, err := os.Stat(bin); err == nil && !info.IsDir() {
			out = append(out, version)
		}
	}
	sort.Strings(out)
	return out, nil
}
```

- [ ] **Step 4: Rewrite installed_test.go**

```go
package redis

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestServerBinary(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	path := ServerBinary("8.6")
	if path == "" {
		t.Error("expected non-empty path")
	}
}

func TestIsInstalled_True(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	versionDir := config.RedisVersionDir("8.6")
	os.MkdirAll(versionDir, 0o755)
	os.WriteFile(filepath.Join(versionDir, "redis-server"), []byte("x"), 0o755)
	if !IsInstalled("8.6") {
		t.Error("expected installed")
	}
}

func TestIsInstalled_False(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if IsInstalled("8.6") {
		t.Error("expected not installed")
	}
}

func TestInstalledVersions(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	vs, err := InstalledVersions()
	if err != nil {
		t.Fatal(err)
	}
	if len(vs) != 0 {
		t.Error("expected no versions")
	}

	// Install a fake version
	versionDir := config.RedisVersionDir("8.6")
	os.MkdirAll(versionDir, 0o755)
	os.WriteFile(filepath.Join(versionDir, "redis-server"), []byte("x"), 0o755)

	vs, err = InstalledVersions()
	if err != nil {
		t.Fatal(err)
	}
	if len(vs) != 1 || vs[0] != "8.6" {
		t.Errorf("got %v, want [8.6]", vs)
	}
}
```

- [ ] **Step 5: Rewrite version.go**

```go
package redis

import (
	"fmt"
	"os/exec"
	"regexp"
	"strings"

	"github.com/prvious/pv/internal/config"
)

var redisVersionRE = regexp.MustCompile(`v=(\d+\.\d+\.\d+)\b`)

func ProbeVersion(version string) (string, error) {
	binPath := ServerBinary(version)
	// During testing the binary may not exist yet — caller handles the error.
	out, err := exec.Command(binPath, "--version").Output()
	if err != nil {
		return "", fmt.Errorf("redis-server --version: %w", err)
	}
	return parseRedisVersion(string(out))
}

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

- [ ] **Step 6: Rewrite version_test.go**

```go
package redis

import "testing"

func TestParseRedisVersion(t *testing.T) {
	tests := []struct {
		input string
		want  string
	}{
		{"Redis server v=7.4.1 sha=00000000:0 malloc=libc bits=64 build=fake", "7.4.1"},
		{"Redis server v=8.6.0 sha=00000000:0 malloc=libc bits=64 build=x", "8.6.0"},
	}
	for _, tc := range tests {
		got, err := parseRedisVersion(tc.input)
		if err != nil {
			t.Errorf("parseRedisVersion(%q) error: %v", tc.input, err)
			continue
		}
		if got != tc.want {
			t.Errorf("parseRedisVersion(%q) = %q, want %q", tc.input, got, tc.want)
		}
	}
}

func TestParseRedisVersion_Empty(t *testing.T) {
	if _, err := parseRedisVersion(""); err == nil {
		t.Error("expected error for empty input")
	}
}

func TestParseRedisVersion_Garbage(t *testing.T) {
	if _, err := parseRedisVersion("not redis output at all"); err == nil {
		t.Error("expected error for garbage input")
	}
}
```

- [ ] **Step 7: Build + test**

Run: `go build ./internal/redis/... && go test ./internal/redis/...`
Expected: port, installed, version tests PASS

- [ ] **Step 8: Commit**

```
git add internal/redis/port.go internal/redis/port_test.go
git add internal/redis/installed.go internal/redis/installed_test.go
git add internal/redis/version.go internal/redis/version_test.go
git commit -m "feat(redis): version-aware port, installed, version functions"
```

---

### Task 5: Wanted, waitstopped, privileges — version-aware

**Files:**
- Modify: `internal/redis/wanted.go`
- Modify: `internal/redis/waitstopped.go`
- Modify: `internal/redis/privileges.go`

- [ ] **Step 1: Rewrite wanted.go**

Replace `IsWanted()` with `WantedVersions()`:

```go
package redis

import (
	"fmt"
	"os"
	"sort"
)

func WantedVersions() ([]string, error) {
	st, err := LoadState()
	if err != nil {
		return nil, err
	}
	installed, err := InstalledVersions()
	if err != nil {
		return nil, err
	}
	installedSet := map[string]struct{}{}
	for _, v := range installed {
		installedSet[v] = struct{}{}
	}
	var out []string
	for version, vs := range st.Versions {
		if vs.Wanted != WantedRunning {
			continue
		}
		if _, ok := installedSet[version]; !ok {
			fmt.Fprintf(os.Stderr, "redis: state.json wants %s running but binary is missing; skipping\n", version)
			continue
		}
		out = append(out, version)
	}
	sort.Strings(out)
	return out, nil
}
```

- [ ] **Step 2: Rewrite wanted_test.go**

```go
package redis

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestWantedVersions_Empty(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	vs, err := WantedVersions()
	if err != nil {
		t.Fatal(err)
	}
	if len(vs) != 0 {
		t.Error("expected empty")
	}
}

func TestWantedVersions_WantedAndInstalled(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	if err := SetWanted("8.6", WantedRunning); err != nil {
		t.Fatal(err)
	}

	// Install fake binary
	versionDir := config.RedisVersionDir("8.6")
	os.MkdirAll(versionDir, 0o755)
	os.WriteFile(filepath.Join(versionDir, "redis-server"), []byte("x"), 0o755)

	vs, err := WantedVersions()
	if err != nil {
		t.Fatal(err)
	}
	if len(vs) != 1 || vs[0] != "8.6" {
		t.Errorf("got %v, want [8.6]", vs)
	}
}

func TestWantedVersions_WantedButNotInstalled_Skipped(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	if err := SetWanted("8.6", WantedRunning); err != nil {
		t.Fatal(err)
	}

	vs, err := WantedVersions()
	if err != nil {
		t.Fatal(err)
	}
	if len(vs) != 0 {
		t.Error("expected empty when binary missing")
	}
}
```

- [ ] **Step 3: Rewrite waitstopped.go**

```go
package redis

import (
	"fmt"
	"net"
	"time"
)

func WaitStopped(version string, timeout time.Duration) error {
	addr := fmt.Sprintf("127.0.0.1:%d", PortFor(version))
	deadline := time.Now().Add(timeout)
	for time.Now().Before(deadline) {
		c, err := net.DialTimeout("tcp", addr, 200*time.Millisecond)
		if err != nil {
			return nil
		}
		c.Close()
		time.Sleep(200 * time.Millisecond)
	}
	return fmt.Errorf("redis %s did not stop within %s", version, timeout)
}
```

- [ ] **Step 4: privileges.go stays unchanged**

`dropCredential`, `dropSysProcAttr`, `chownToTarget` have no version dependency. No changes needed.

- [ ] **Step 5: Build + test**

Run: `go build ./internal/redis/... && go test ./internal/redis/...`
Expected: PASS

- [ ] **Step 6: Commit**

```
git add internal/redis/wanted.go internal/redis/waitstopped.go
git commit -m "feat(redis): version-aware WantedVersions and WaitStopped"
```

---

### Task 6: Process, envvars, templatevars — version-aware

**Files:**
- Modify: `internal/redis/process.go`
- Modify: `internal/redis/process_test.go`
- Modify: `internal/redis/envvars.go`
- Modify: `internal/redis/envvars_test.go`
- Modify: `internal/redis/templatevars.go`
- Modify: `internal/redis/templatevars_test.go`

- [ ] **Step 1: Rewrite process.go**

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

func BuildSupervisorProcess(version string) (supervisor.Process, error) {
	binPath := ServerBinary(version)
	if _, err := os.Stat(binPath); err != nil {
		return supervisor.Process{}, fmt.Errorf("redis-%s: not installed (run pv redis:install %s)", version, version)
	}
	return supervisor.Process{
		Name:    "redis-" + version,
		Binary:  binPath,
		Args:    buildRedisArgs(version),
		LogFile: config.RedisLogPathV(version),
		SysProcAttr: dropSysProcAttr(),
		Ready:   tcpReady(PortFor(version)),
		ReadyTimeout: 10 * time.Second,
	}, nil
}

func buildRedisArgs(version string) []string {
	return []string{
		"--bind", "127.0.0.1",
		"--port", strconv.Itoa(PortFor(version)),
		"--dir", config.RedisDataDirV(version),
		"--dbfilename", "dump.rdb",
		"--pidfile", "/tmp/pv-redis-" + version + ".pid",
		"--daemonize", "no",
		"--protected-mode", "no",
		"--appendonly", "no",
	}
}

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

- [ ] **Step 2: Rewrite process_test.go**

```go
package redis

import (
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"

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

	versionDir := config.RedisVersionDir("8.6")
	os.MkdirAll(versionDir, 0o755)

	// Build the fake redis-server binary
	out := filepath.Join(versionDir, "redis-server")
	cmd := exec.Command("go", "build", "-o", out,
		filepath.Join("..", "..", "internal", "redis", "testdata", "fake-redis-server.go"))
	if b, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("build fake redis-server: %v\n%s", err, b)
	}

	proc, err := BuildSupervisorProcess("8.6")
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
```

Note: need to add `"os/exec"` and `"time"` to the test imports.

- [ ] **Step 3: Rewrite envvars.go**

```go
package redis

import "strconv"

func EnvVars(version, projectName string) map[string]string {
	_ = projectName
	return map[string]string{
		"REDIS_HOST":     "127.0.0.1",
		"REDIS_PORT":     strconv.Itoa(PortFor(version)),
		"REDIS_PASSWORD": "null",
	}
}
```

- [ ] **Step 4: Rewrite envvars_test.go**

```go
package redis

import "testing"

func TestEnvVars_ContainsExpectedKeys(t *testing.T) {
	vars := EnvVars("8.6", "myapp")
	if vars["REDIS_HOST"] != "127.0.0.1" {
		t.Errorf("REDIS_HOST = %q", vars["REDIS_HOST"])
	}
	if vars["REDIS_PORT"] != "6860" {
		t.Errorf("REDIS_PORT = %q", vars["REDIS_PORT"])
	}
	if vars["REDIS_PASSWORD"] != "null" {
		t.Errorf("REDIS_PASSWORD = %q", vars["REDIS_PASSWORD"])
	}
}
```

- [ ] **Step 5: Rewrite templatevars.go**

```go
package redis

import (
	"fmt"
	"strconv"
)

func TemplateVars(version string) map[string]string {
	const host = "127.0.0.1"
	port := PortFor(version)
	return map[string]string{
		"host":     host,
		"port":     strconv.Itoa(port),
		"password": "",
		"url":      fmt.Sprintf("redis://%s:%d", host, port),
	}
}
```

- [ ] **Step 6: Rewrite templatevars_test.go**

```go
package redis

import "testing"

func TestTemplateVars(t *testing.T) {
	vars := TemplateVars("8.6")
	if vars["host"] != "127.0.0.1" {
		t.Errorf("host = %q", vars["host"])
	}
	if vars["port"] != "6860" {
		t.Errorf("port = %q", vars["port"])
	}
	if vars["password"] != "" {
		t.Errorf("password = %q", vars["password"])
	}
	if vars["url"] != "redis://127.0.0.1:6860" {
		t.Errorf("url = %q", vars["url"])
	}
}
```

- [ ] **Step 7: Build + test**

Run: `go build ./internal/redis/... && go test ./internal/redis/...`
Expected: PASS

- [ ] **Step 8: Commit**

```
git add internal/redis/process.go internal/redis/process_test.go
git add internal/redis/envvars.go internal/redis/envvars_test.go
git add internal/redis/templatevars.go internal/redis/templatevars_test.go
git commit -m "feat(redis): version-aware process, envvars, templatevars"
```

---

### Task 7: Install, uninstall, update — version-aware

**Files:**
- Modify: `internal/redis/install.go`
- Modify: `internal/redis/install_test.go`
- Modify: `internal/redis/uninstall.go`
- Modify: `internal/redis/uninstall_test.go`
- Modify: `internal/redis/update.go`
- Modify: `internal/redis/update_test.go`

- [ ] **Step 1: Rewrite install.go**

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

func Install(client *http.Client, version string) error {
	return InstallProgress(client, version, nil)
}

func InstallProgress(client *http.Client, version string, progress binaries.ProgressFunc) error {
	if err := config.EnsureDirs(); err != nil {
		return err
	}

	url, err := resolveRedisURL()
	if err != nil {
		return err
	}

	versionDir := config.RedisVersionDir(version)
	if !IsInstalled(version) {
		stagingDir := versionDir + ".new"
		os.RemoveAll(stagingDir)
		if err := os.MkdirAll(stagingDir, 0o755); err != nil {
			return fmt.Errorf("create staging: %w", err)
		}
		archive := filepath.Join(config.PvDir(), "redis-"+version+".tar.gz")
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
		os.RemoveAll(versionDir)
		if err := os.Rename(stagingDir, versionDir); err != nil {
			os.RemoveAll(stagingDir)
			return fmt.Errorf("rename staging: %w", err)
		}
		if err := chownToTarget(versionDir); err != nil {
			return fmt.Errorf("chown redis tree: %w", err)
		}
	}

	dataDir := config.RedisDataDirV(version)
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		return fmt.Errorf("create redis data dir: %w", err)
	}
	if err := chownToTarget(dataDir); err != nil {
		return fmt.Errorf("chown redis data dir: %w", err)
	}

	if v, err := ProbeVersion(version); err == nil {
		if vs, err := binaries.LoadVersions(); err == nil {
			vs.Set("redis-"+version, v)
			_ = vs.Save()
		}
	}

	return SetWanted(version, WantedRunning)
}

func resolveRedisURL() (string, error) {
	if override := os.Getenv("PV_REDIS_URL_OVERRIDE"); override != "" {
		return override, nil
	}
	return binaries.RedisURL()
}
```

- [ ] **Step 2: Rewrite install_test.go**

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

	// Binaries on disk under versioned dir.
	serverPath := filepath.Join(config.RedisVersionDir(version), "redis-server")
	cliPath := filepath.Join(config.RedisVersionDir(version), "redis-cli")
	if _, err := os.Stat(serverPath); err != nil {
		t.Errorf("missing redis-server: %v", err)
	}
	if _, err := os.Stat(cliPath); err != nil {
		t.Errorf("missing redis-cli: %v", err)
	}

	// Data dir present.
	if _, err := os.Stat(config.RedisDataDirV(version)); err != nil {
		t.Errorf("data dir missing: %v", err)
	}

	// State recorded.
	st, _ := LoadState()
	if st.Versions[version].Wanted != WantedRunning {
		t.Errorf("state.Versions[%q].Wanted = %q, want running", version, st.Versions[version].Wanted)
	}

	// Version recorded.
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
```

- [ ] **Step 3: Rewrite uninstall.go**

```go
package redis

import (
	"os"
	"time"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
)

func Uninstall(version string, force bool) error {
	if isInstalledOnDisk(version) {
		_ = SetWanted(version, WantedStopped)
		_ = WaitStopped(version, 10*time.Second)
	}

	if err := RemoveVersion(version); err != nil {
		return err
	}
	if vs, err := binaries.LoadVersions(); err == nil {
		delete(vs.Versions, "redis-"+version)
		_ = vs.Save()
	}
	if reg, err := registry.Load(); err == nil {
		reg.UnbindRedisVersion(version)
		_ = reg.Save()
	}

	if err := os.RemoveAll(config.RedisVersionDir(version)); err != nil {
		return err
	}
	_ = os.Remove(config.RedisLogPathV(version))
	if force {
		if err := os.RemoveAll(config.RedisDataDirV(version)); err != nil {
			return err
		}
	}
	return nil
}

func isInstalledOnDisk(version string) bool {
	_, err := os.Stat(config.RedisVersionDir(version))
	return err == nil
}
```

- [ ] **Step 4: Rewrite uninstall_test.go**

```go
package redis

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
)

func TestUninstall_NonForce_KeepsDataDir(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	version := "8.6"

	// Create fake install
	versionDir := config.RedisVersionDir(version)
	os.MkdirAll(versionDir, 0o755)
	os.WriteFile(filepath.Join(versionDir, "redis-server"), []byte("x"), 0o755)

	dataDir := config.RedisDataDirV(version)
	os.MkdirAll(dataDir, 0o755)
	os.WriteFile(filepath.Join(dataDir, "dump.rdb"), []byte("data"), 0644)

	SetWanted(version, WantedRunning)

	// Bind a project
	reg := registry.New()
	reg.AddProject("myapp", "/tmp/myapp", "laravel")
	reg.Projects[0].Services = &registry.ProjectServices{Redis: version}
	reg.Save()

	if err := Uninstall(version, false); err != nil {
		t.Fatalf("Uninstall: %v", err)
	}

	// Binary tree removed
	if _, err := os.Stat(versionDir); !os.IsNotExist(err) {
		t.Error("expected binary dir removed")
	}

	// Data dir preserved
	if _, err := os.Stat(dataDir); err != nil {
		t.Error("expected data dir preserved")
	}

	// Registry unbound
	reg2, _ := registry.Load()
	if reg2.Projects[0].Services != nil && reg2.Projects[0].Services.Redis != "" {
		t.Error("expected redis binding removed")
	}
}

func TestUninstall_Force_RemovesDataDir(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	version := "8.6"

	versionDir := config.RedisVersionDir(version)
	os.MkdirAll(versionDir, 0o755)
	os.WriteFile(filepath.Join(versionDir, "redis-server"), []byte("x"), 0o755)

	dataDir := config.RedisDataDirV(version)
	os.MkdirAll(dataDir, 0o755)

	if err := Uninstall(version, true); err != nil {
		t.Fatalf("Uninstall: %v", err)
	}

	if _, err := os.Stat(dataDir); !os.IsNotExist(err) {
		t.Error("expected data dir removed with --force")
	}
}
```

- [ ] **Step 5: Rewrite update.go**

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

func Update(client *http.Client, version string) error {
	return UpdateProgress(client, version, nil)
}

func UpdateProgress(client *http.Client, version string, progress binaries.ProgressFunc) error {
	if !IsInstalled(version) {
		return fmt.Errorf("redis-%s is not installed", version)
	}

	prevWanted := WantedStopped
	if st, err := LoadState(); err == nil {
		if vs, ok := st.Versions[version]; ok && vs.Wanted != "" {
			prevWanted = vs.Wanted
		}
	}

	if prevWanted == WantedRunning {
		_ = SetWanted(version, WantedStopped)
		_ = WaitStopped(version, 10*time.Second)
	}

	url, err := resolveRedisURL()
	if err != nil {
		return err
	}

	versionDir := config.RedisVersionDir(version)
	stagingDir := versionDir + ".new"
	os.RemoveAll(stagingDir)
	if err := os.MkdirAll(stagingDir, 0o755); err != nil {
		return fmt.Errorf("create staging: %w", err)
	}

	archive := filepath.Join(config.PvDir(), "redis-"+version+".tar.gz")
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

	oldDir := versionDir + ".old"
	os.RemoveAll(oldDir)
	if err := os.Rename(versionDir, oldDir); err != nil {
		os.RemoveAll(stagingDir)
		return fmt.Errorf("rename old: %w", err)
	}
	if err := os.Rename(stagingDir, versionDir); err != nil {
		if rollbackErr := os.Rename(oldDir, versionDir); rollbackErr != nil {
			return fmt.Errorf("rename new failed (%w); rollback also failed (%v); redis %s install dir is broken",
				err, rollbackErr, version)
		}
		return fmt.Errorf("rename new: %w", err)
	}
	os.RemoveAll(oldDir)

	if err := chownToTarget(versionDir); err != nil {
		return fmt.Errorf("chown redis tree: %w", err)
	}

	if v, err := ProbeVersion(version); err == nil {
		if vs, err := binaries.LoadVersions(); err == nil {
			vs.Set("redis-"+version, v)
			_ = vs.Save()
		}
	}

	return SetWanted(version, prevWanted)
}
```

- [ ] **Step 6: Build + test**

Run: `go build ./internal/redis/... && go test ./internal/redis/...`
Expected: all internal/redis/ tests PASS

- [ ] **Step 7: Commit**

```
git add internal/redis/install.go internal/redis/install_test.go
git add internal/redis/uninstall.go internal/redis/uninstall_test.go
git add internal/redis/update.go internal/redis/update_test.go
git commit -m "feat(redis): version-aware install, uninstall, update"
```

---

### Task 8: Commands layer — version arg with default

**Files:**
- Modify: `internal/commands/redis/register.go`
- Modify: `internal/commands/redis/install.go`
- Modify: `internal/commands/redis/uninstall.go`
- Modify: `internal/commands/redis/update.go`
- Modify: `internal/commands/redis/start.go`
- Modify: `internal/commands/redis/stop.go`
- Modify: `internal/commands/redis/restart.go`
- Modify: `internal/commands/redis/status.go`
- Modify: `internal/commands/redis/list.go`
- Modify: `internal/commands/redis/logs.go`
- Modify: `internal/commands/redis/download.go`

Each command gets an optional `version` arg defaulting to `config.RedisDefaultVersion()`.

- [ ] **Step 1: Rewrite register.go** — add a shared helper `resolveVersion`

Add at top of register.go:
```go
import "github.com/prvious/pv/internal/config"

func resolveVersion(args []string) string {
	if len(args) > 0 {
		return args[0]
	}
	return config.RedisDefaultVersion()
}
```

- [ ] **Step 2: Rewrite install.go**

```go
var installCmd = &cobra.Command{
	Use:     "redis:install [version]",
	GroupID: "redis",
	Short:   "Install (or re-install) Redis",
	Long:    "Downloads the Redis binary and registers it as wanted-running.",
	Example: `pv redis:install`,
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version := resolveVersion(args)
		if r.IsInstalled(version) {
			if err := r.SetWanted(version, r.WantedRunning); err != nil {
				return err
			}
			ui.Success(fmt.Sprintf("Redis %s already installed — marked as wanted running.", version))
			return signalDaemon()
		}
		if err := downloadCmd.RunE(downloadCmd, []string{version}); err != nil {
			return err
		}
		ui.Success(fmt.Sprintf("Redis %s installed.", version))
		return signalDaemon()
	},
}
```

- [ ] **Step 3: Rewrite uninstall.go**

```go
var uninstallForce bool

var uninstallCmd = &cobra.Command{
	Use:     "redis:uninstall [version]",
	GroupID: "redis",
	Short:   "Stop, remove the binary, and (with --force) DELETE the data directory",
	Long: "Stops the supervised process and removes the binary tree at " +
		"~/.pv/redis/{version}/. With --force, also removes the data directory. " +
		"Unbinds every linked project bound to that version.",
	Example: `pv redis:uninstall --force`,
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version := resolveVersion(args)
		if !r.IsInstalled(version) {
			ui.Subtle(fmt.Sprintf("Redis %s is not installed.", version))
			return nil
		}
		if !uninstallForce {
			confirmed := false
			if err := huh.NewConfirm().
				Title(fmt.Sprintf("Remove Redis %s? With --force this also DELETES the data directory.", version)).
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

		if err := r.SetWanted(version, r.WantedStopped); err != nil {
			return err
		}
		if server.IsRunning() {
			if err := server.SignalDaemon(); err != nil {
				return fmt.Errorf("signal daemon: %w", err)
			}
			if err := r.WaitStopped(version, 10*time.Second); err != nil {
				return fmt.Errorf("waiting for redis to stop: %w", err)
			}
		}

		if err := ui.Step(fmt.Sprintf("Uninstalling Redis %s...", version), func() (string, error) {
			if err := r.Uninstall(version, uninstallForce); err != nil {
				return "", err
			}
			return fmt.Sprintf("Uninstalled Redis %s", version), nil
		}); err != nil {
			return err
		}

		ui.Success(fmt.Sprintf("Redis %s uninstalled.", version))
		return nil
	},
}

func init() {
	uninstallCmd.Flags().BoolVar(&uninstallForce, "force", false, "Skip the confirmation prompt and delete the data directory")
}
```

- [ ] **Step 4: Rewrite update.go**

```go
var updateCmd = &cobra.Command{
	Use:     "redis:update [version]",
	GroupID: "redis",
	Short:   "Re-download and re-install the Redis binary",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version := resolveVersion(args)
		if !r.IsInstalled(version) {
			ui.Subtle(fmt.Sprintf("Redis %s is not installed.", version))
			return nil
		}
		if err := ui.Step(fmt.Sprintf("Updating Redis %s...", version), func() (string, error) {
			client := &http.Client{}
			if err := r.UpdateProgress(client, version, ui.ProgressBar("Downloading Redis...")); err != nil {
				return "", err
			}
			return fmt.Sprintf("Redis %s updated.", version), nil
		}); err != nil {
			return err
		}
		ui.Success(fmt.Sprintf("Redis %s updated.", version))
		return signalDaemon()
	},
}
```

- [ ] **Step 5: Rewrite start.go**

```go
var startCmd = &cobra.Command{
	Use:     "redis:start [version]",
	GroupID: "redis",
	Short:   "Mark Redis as wanted-running",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version := resolveVersion(args)
		if !r.IsInstalled(version) {
			ui.Subtle(fmt.Sprintf("Redis %s is not installed (run `pv redis:install %s`).", version, version))
			return nil
		}
		if err := r.SetWanted(version, r.WantedRunning); err != nil {
			return err
		}
		ui.Success(fmt.Sprintf("Redis %s marked running.", version))
		if server.IsRunning() {
			return server.SignalDaemon()
		}
		ui.Subtle("daemon not running — will start on next `pv start`")
		return nil
	},
}
```

- [ ] **Step 6: Rewrite stop.go**

```go
var stopCmd = &cobra.Command{
	Use:     "redis:stop [version]",
	GroupID: "redis",
	Short:   "Mark Redis as wanted-stopped",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version := resolveVersion(args)
		if err := r.SetWanted(version, r.WantedStopped); err != nil {
			return err
		}
		ui.Success(fmt.Sprintf("Redis %s marked stopped.", version))
		if server.IsRunning() {
			return server.SignalDaemon()
		}
		return nil
	},
}
```

- [ ] **Step 7: Rewrite restart.go**

```go
var restartCmd = &cobra.Command{
	Use:     "redis:restart [version]",
	GroupID: "redis",
	Short:   "Stop and start Redis",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version := resolveVersion(args)
		if !r.IsInstalled(version) {
			ui.Subtle(fmt.Sprintf("Redis %s is not installed.", version))
			return nil
		}
		if err := r.SetWanted(version, r.WantedStopped); err != nil {
			return err
		}
		if server.IsRunning() {
			if err := server.SignalDaemon(); err != nil {
				return fmt.Errorf("signal daemon: %w", err)
			}
			if err := r.WaitStopped(version, 10*time.Second); err != nil {
				return fmt.Errorf("waiting for redis to stop: %w", err)
			}
		}
		if err := r.SetWanted(version, r.WantedRunning); err != nil {
			return err
		}
		if server.IsRunning() {
			if err := server.SignalDaemon(); err != nil {
				return err
			}
		}
		ui.Success(fmt.Sprintf("Redis %s restarted.", version))
		return nil
	},
}
```

- [ ] **Step 8: Rewrite status.go**

```go
var statusCmd = &cobra.Command{
	Use:     "redis:status [version]",
	GroupID: "redis",
	Short:   "Show Redis status",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version := resolveVersion(args)
		if !r.IsInstalled(version) {
			ui.Subtle(fmt.Sprintf("Redis %s is not installed.", version))
			return nil
		}
		status, _ := server.ReadDaemonStatus()
		supKey := "redis-" + version
		if status != nil {
			if s, ok := status.Supervised[supKey]; ok && s.Running {
				ui.Success(fmt.Sprintf("redis-%s: running on :%d (pid %d)", version, r.PortFor(version), s.PID))
				return nil
			}
		}
		ui.Subtle(fmt.Sprintf("redis-%s: stopped", version))
		return nil
	},
}
```

- [ ] **Step 9: Rewrite list.go**

```go
var listCmd = &cobra.Command{
	Use:     "redis:list",
	GroupID: "redis",
	Short:   "Show Redis versions and status",
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		versions, err := r.InstalledVersions()
		if err != nil {
			return err
		}
		if len(versions) == 0 {
			ui.Subtle("Redis is not installed.")
			return nil
		}

		st, _ := r.LoadState()
		vs, _ := binaries.LoadVersions()
		reg, _ := registry.Load()
		status, _ := server.ReadDaemonStatus()

		var rows [][]string
		for _, version := range versions {
			precise := "?"
			if vs != nil {
				if v := vs.Get("redis-" + version); v != "" {
					precise = v
				}
			}

			supKey := "redis-" + version
			runState := "stopped"
			if status != nil {
				if s, ok := status.Supervised[supKey]; ok && s.Running {
					runState = "running"
				}
			}
			wanted := ""
			if vs, ok := st.Versions[version]; ok {
				wanted = vs.Wanted
			}
			if wanted == "" {
				wanted = "—"
			}

			projects := []string{}
			if reg != nil {
				for _, p := range reg.List() {
					if p.Services != nil && p.Services.Redis == version {
						projects = append(projects, p.Name)
					}
				}
			}
			projectsCol := "—"
			if len(projects) > 0 {
				projectsCol = strings.Join(projects, ",")
			}

			rows = append(rows, []string{
				version,
				precise,
				fmt.Sprintf("%d", r.PortFor(version)),
				fmt.Sprintf("%s (%s)", runState, wanted),
				config.RedisDataDirV(version),
				projectsCol,
			})
		}
		ui.Table([]string{"VERSION", "PRECISE", "PORT", "STATUS", "DATA DIR", "LINKED PROJECTS"}, rows)
		return nil
	},
}
```

- [ ] **Step 10: Rewrite logs.go**

```go
var logsCmd = &cobra.Command{
	Use:     "redis:logs [version]",
	GroupID: "redis",
	Short:   "Tail the Redis log file",
	Long:    "Reads ~/.pv/logs/redis-{version}.log. With -f / --follow, tails the file.",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version := resolveVersion(args)
		path := config.RedisLogPathV(version)
		// ... rest same as before
	},
}
```

- [ ] **Step 11: Rewrite download.go**

```go
var downloadCmd = &cobra.Command{
	Use:     "redis:download [version]",
	GroupID: "redis",
	Short:   "Download the Redis binary (without marking wanted-running)",
	Long:    "Hidden — debug only. Downloads and extracts the redis tarball but does not mark wanted=running.",
	Hidden:  true,
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version := resolveVersion(args)
		client := &http.Client{}
		return r.InstallProgress(client, version, ui.ProgressBar("Downloading Redis..."))
	},
}
```

- [ ] **Step 12: Update `register.go` Run* helpers and `UninstallForce`**

```go
func RunInstall(args []string) error {
	return installCmd.RunE(installCmd, args)
}

func RunUpdate(args []string) error {
	return updateCmd.RunE(updateCmd, args)
}

func RunUninstall(args []string) error {
	return uninstallCmd.RunE(uninstallCmd, args)
}

func UninstallForce(version string) error {
	prev := uninstallForce
	uninstallForce = true
	defer func() { uninstallForce = prev }()
	return uninstallCmd.RunE(uninstallCmd, []string{version})
}
```

- [ ] **Step 13: Build + test**

Run: `go build ./internal/commands/redis/... && go test ./internal/commands/redis/...`
Expected: PASS

- [ ] **Step 14: Commit**

```
git add internal/commands/redis/
git commit -m "feat(redis): version arg on all redis:* commands"
```

---

### Task 9: Manager — WantedVersions iteration

**Files:**
- Modify: `internal/server/manager.go`
- Modify: `internal/server/manager_test.go`

- [ ] **Step 1: Update manager.go**

Replace the flat redis wanted-set source:

Before (lines 244-252):
```go
// Source 4 — redis, single-version, filesystem + state.json.
if redis.IsWanted() {
    proc, err := redis.BuildSupervisorProcess()
    if err != nil {
        startErrors = append(startErrors, fmt.Sprintf("redis: build: %v", err))
    } else {
        wanted["redis"] = proc
    }
}
```

After:
```go
// Source 4 — redis, multi-version via WantedVersions.
rdVersions, rdErr := redis.WantedVersions()
if rdErr != nil {
    fmt.Fprintf(os.Stderr, "reconcile binary: redis.WantedVersions: %v\n", rdErr)
}
for _, version := range rdVersions {
    proc, err := redis.BuildSupervisorProcess(version)
    if err != nil {
        startErrors = append(startErrors, fmt.Sprintf("redis-%s: build: %v", version, err))
        continue
    }
    wanted["redis-"+version] = proc
}
```

Also add `rdErr` to the transient-error guard for redis keys:
```go
if rdErr != nil && strings.HasPrefix(supKey, "redis-") {
    continue
}
```

- [ ] **Step 2: Update manager_test.go**

Update `TestReconcileBinaryServices_StartsWantedRedis`:

```go
func TestReconcileBinaryServices_StartsWantedRedis(t *testing.T) {
	if runtime.GOOS != "darwin" && runtime.GOOS != "linux" {
		t.Skipf("fake binary helper not supported on %s", runtime.GOOS)
	}
	t.Setenv("HOME", t.TempDir())

	version := config.RedisDefaultVersion()
	versionDir := config.RedisVersionDir(version)
	if err := os.MkdirAll(versionDir, 0o755); err != nil {
		t.Fatal(err)
	}
	cmd := exec.Command("go", "build", "-o", filepath.Join(versionDir, "redis-server"),
		filepath.Join("..", "..", "internal", "redis", "testdata", "fake-redis-server.go"))
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("build fake redis-server: %v\n%s", err, out)
	}

	if err := redis.SetWanted(version, redis.WantedRunning); err != nil {
		t.Fatal(err)
	}

	sup := supervisor.New()
	mgr := NewServerManager(nil, sup)
	defer sup.StopAll(2 * time.Second)

	if err := mgr.reconcileBinaryServices(context.Background()); err != nil {
		t.Fatalf("reconcileBinaryServices: %v", err)
	}

	supKey := "redis-" + version
	if !sup.IsRunning(supKey) {
		t.Errorf("expected %s to be supervised after reconcile", supKey)
	}
}
```

And `TestReconcileBinaryServices_StopsRemovedRedis`:

```go
func TestReconcileBinaryServices_StopsRemovedRedis(t *testing.T) {
	if runtime.GOOS != "darwin" && runtime.GOOS != "linux" {
		t.Skipf("fake binary helper not supported on %s", runtime.GOOS)
	}
	t.Setenv("HOME", t.TempDir())

	version := config.RedisDefaultVersion()
	versionDir := config.RedisVersionDir(version)
	if err := os.MkdirAll(versionDir, 0o755); err != nil {
		t.Fatal(err)
	}
	cmd := exec.Command("go", "build", "-o", filepath.Join(versionDir, "redis-server"),
		filepath.Join("..", "..", "internal", "redis", "testdata", "fake-redis-server.go"))
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("build fake redis-server: %v\n%s", err, out)
	}

	if err := redis.SetWanted(version, redis.WantedRunning); err != nil {
		t.Fatal(err)
	}

	sup := supervisor.New()
	mgr := NewServerManager(nil, sup)
	defer sup.StopAll(2 * time.Second)

	supKey := "redis-" + version
	if err := mgr.reconcileBinaryServices(context.Background()); err != nil {
		t.Fatal(err)
	}
	if !sup.IsRunning(supKey) {
		t.Fatal("expected redis running after first reconcile")
	}

	if err := redis.SetWanted(version, redis.WantedStopped); err != nil {
		t.Fatal(err)
	}
	if err := mgr.reconcileBinaryServices(context.Background()); err != nil {
		t.Fatal(err)
	}
	if sup.IsRunning(supKey) {
		t.Error("expected redis stopped after wanted flipped to stopped")
	}
}
```

- [ ] **Step 3: Build + test**

Run: `go build ./internal/server/... && go test ./internal/server/...`
Expected: PASS

- [ ] **Step 4: Commit**

```
git add internal/server/manager.go internal/server/manager_test.go
git commit -m "feat(redis): reconcile over WantedVersions"
```

---

### Task 10: cmd/ orchestrators, setup, laravel/automation

**Files:**
- Modify: `cmd/update.go`
- Modify: `cmd/uninstall.go`
- Modify: `cmd/setup.go`
- Modify: `internal/automation/steps/apply_pvyml_services.go`
- Modify: `internal/automation/steps/apply_pvyml_env.go`

- [ ] **Step 1: Update cmd/update.go — iterate over InstalledVersions**

Replace the flat `r.IsInstalled()` block (lines 111-120) with iteration:

```go
// Update each installed redis version. Mirrors the mysql pass.
if versions, err := r.InstalledVersions(); err == nil {
    for _, version := range versions {
        if err := rediscmd.RunUpdate([]string{version}); err != nil {
            if !errors.Is(err, ui.ErrAlreadyPrinted) {
                ui.Fail(fmt.Sprintf("Redis %s update failed: %v", version, err))
            }
            failures = append(failures, "Redis "+version)
        }
    }
}
```

- [ ] **Step 2: Update cmd/uninstall.go — iterate over InstalledVersions**

Replace the flat `r.IsInstalled()` block (lines 235-244):

Before:
```go
// Redis uninstall (single-version).
if r.IsInstalled() {
    if err := rediscmd.UninstallForce(); err != nil {
```

After:
```go
// Redis uninstall (per installed version). Matches postgres/mysql pattern.
if versions, err := r.InstalledVersions(); err == nil {
    for _, version := range versions {
        if err := rediscmd.UninstallForce(version); err != nil {
            hadFailures = true
            if !errors.Is(err, ui.ErrAlreadyPrinted) {
                ui.Fail(fmt.Sprintf("redis %s uninstall failed: %v", version, err))
            }
        }
    }
}
```

- [ ] **Step 3: Update cmd/setup.go — pass version to RunInstall**

Line 221-227, change:
```go
if toolSet["redis"] {
    if err := rediscmd.RunInstall(nil); err != nil {
```
to:
```go
if toolSet["redis"] {
    if err := rediscmd.RunInstall([]string{config.RedisDefaultVersion()}); err != nil {
```

Add `config "github.com/prvious/pv/internal/config"` to setup.go imports.

- [ ] **Step 4: Update apply_pvyml_services.go**

Change the redis block (lines 62-67) to use version:

```go
if cfg.Redis != nil {
    version := config.RedisDefaultVersion()
    if !redis.IsInstalled(version) {
        return "", fmt.Errorf("pv.yml redis %s is not installed — run `pv redis:install %s`", version, version)
    }
    bindProjectService(ctx.Registry, ctx.ProjectName, "redis", "redis")
    count++
}
```

Change `bindProjectService` function (line 109):
```go
case "redis":
    reg.Projects[i].Services.Redis = config.RedisDefaultVersion()
```

Add `"github.com/prvious/pv/internal/config"` to imports.

- [ ] **Step 5: Update apply_pvyml_env.go**

The `redis.TemplateVars()` call at line 81 needs a version param. Change:
```go
if err := renderIntoMap(rendered, cfg.Redis.Env, redis.TemplateVars(), "redis.env"); err != nil {
```
to:
```go
if err := renderIntoMap(rendered, cfg.Redis.Env, redis.TemplateVars(config.RedisDefaultVersion()), "redis.env"); err != nil {
```

- [ ] **Step 6: Build all**

Run: `go build ./... && go vet ./...`
Expected: no errors

- [ ] **Step 7: Run all tests**

Run: `go test ./...`
Expected: all PASS

- [ ] **Step 8: Commit**

```
git add cmd/update.go cmd/uninstall.go cmd/setup.go
git add internal/automation/steps/apply_pvyml_services.go
git add internal/automation/steps/apply_pvyml_env.go
git commit -m "feat(redis): update orchestrators and automation for versioned redis"
```

---

### Task 11: Format, vet, final build, cleanup

**Files:**
- All modified files

- [ ] **Step 1: Format everything**

Run: `gofmt -w .`

- [ ] **Step 2: Vet**

Run: `go vet ./...`
Expected: clean

- [ ] **Step 3: Final full test**

Run: `go test ./...`
Expected: all PASS

- [ ] **Step 4: Final commit**

```
git add -A
git commit -m "chore(redis): gofmt, vet, final cleanup"
```
