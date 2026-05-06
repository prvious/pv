# PostgreSQL Native Binaries Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace docker-backed postgres with native binaries supervised by the pv daemon. Multi-version coexistence (PG 17 + 18). Top-level `postgres:*` command group with `pg:*` aliases. Trust auth on 127.0.0.1.

**Architecture:** New `internal/postgres/` package mirroring `internal/phpenv/` owns version-aware lifecycle (download, initdb, conf templating, supervisor.Process construction). New `internal/state/` package owns per-service runtime state at `~/.pv/data/state.json`. Existing `reconcileBinaryServices` gains a second wanted-set source from `postgres.WantedMajors()` — diff/start/stop loop unchanged. Docker `services.Postgres` is deleted entirely.

**Tech Stack:** Go, existing `binaries` package (download, extract, version state), existing `supervisor` + `ServerManager`, existing `tools` registry pattern (for command surface). Cobra for CLI.

**Spec:** `docs/superpowers/specs/2026-05-05-postgres-native-binaries-design.md`

**Dependency:** `docs/superpowers/plans/2026-04-29-postgres-artifacts.md` must be merged (artifacts pipeline ships `postgres-mac-arm64-{17,18}.tar.gz` from the rolling `artifacts` release at `prvious/pv`). Verified present at the time of writing.

---

## File Structure

| Path | Action | Responsibility |
|------|--------|---------------|
| `internal/state/state.go` | Create | Generic per-service state file at `~/.pv/data/state.json`, top-level keyed by service. Load/Save with file lock; `map[string]json.RawMessage` so each service owns its own subschema. |
| `internal/state/state_test.go` | Create | Round-trip tests, missing-file tolerance, corrupt-file recovery. |
| `internal/config/paths.go` | Modify | Add `StatePath()` helper (`~/.pv/data/state.json`) and `PostgresDir()` / `PostgresVersionDir(major)` helpers. |
| `internal/config/paths_test.go` | Modify | Add tests for new helpers. |
| `internal/binaries/postgres.go` | Create | `DownloadURL` + `ChecksumURL` for `postgres-mac-arm64-<major>.tar.gz`. |
| `internal/binaries/postgres_test.go` | Create | URL construction tests. |
| `internal/postgres/port.go` | Create | `PortFor(major) = 54000 + major`. |
| `internal/postgres/port_test.go` | Create | Port arithmetic tests including invalid major fallback. |
| `internal/postgres/installed.go` | Create | `InstalledMajors()` scans `~/.pv/postgres/<n>/`. |
| `internal/postgres/installed_test.go` | Create | Filesystem scan tests. |
| `internal/postgres/state.go` | Create | Wraps `internal/state` for the `postgres` key — get/set wanted state per major. |
| `internal/postgres/state_test.go` | Create | State read/write round-trip via the generic state package. |
| `internal/postgres/wanted.go` | Create | `WantedMajors()` returns the intersection of installed-on-disk AND `wanted=running`. |
| `internal/postgres/wanted_test.go` | Create | Intersection rules + missing-binaries-with-stale-state warning. |
| `internal/postgres/version.go` | Create | `ProbeVersion(major)` runs `pg_config --version` and returns "17.5" etc. |
| `internal/postgres/version_test.go` | Create | Probe via a synthetic `pg_config` shim. |
| `internal/postgres/conf.go` | Create | `WriteOverrides(major)` appends pv-managed block to `postgresql.conf`; `RewriteHBA(major)` writes the trust-only `pg_hba.conf`. |
| `internal/postgres/conf_test.go` | Create | Idempotency (running twice doesn't grow the conf), correct port arithmetic per major. |
| `internal/postgres/initdb.go` | Create | `RunInitdb(major)` invokes the bundled `initdb`; idempotent via PG_VERSION presence; cleans partial dirs on failure. |
| `internal/postgres/initdb_test.go` | Create | Idempotency test (second run is a no-op). E2E-flavored test gated on the binaries being installed. |
| `internal/postgres/install.go` | Create | `Install(client, major)` orchestrates: download → extract → chmod → initdb → conf → version-record → state-update. |
| `internal/postgres/install_test.go` | Create | Mock-server install test (download path); idempotent re-install. |
| `internal/postgres/uninstall.go` | Create | `Uninstall(major)` removes data dir + binaries + log + state entry + version entry. |
| `internal/postgres/uninstall_test.go` | Create | Removes everything; missing major is a no-op. |
| `internal/postgres/update.go` | Create | `Update(client, major)` stops, redownloads (atomic via `.new` + rename), re-emits conf, marks running. |
| `internal/postgres/update_test.go` | Create | Atomic-rename behavior; data dir untouched. |
| `internal/postgres/envvars.go` | Create | `EnvVars(projectName, major)` returns `DB_*` map. |
| `internal/postgres/envvars_test.go` | Create | Golden test for the map; correct port per major. |
| `internal/postgres/process.go` | Create | `BuildSupervisorProcess(major)` returns a `supervisor.Process`. |
| `internal/postgres/process_test.go` | Create | Refuses uninitialized data dir; correct binary path + log file. |
| `internal/server/manager.go` | Modify | `reconcileBinaryServices` gains a second wanted-set source from `postgres.WantedMajors()`. |
| `internal/server/manager_test.go` | Modify | Reconcile picks up postgres majors; stops removed ones. |
| `internal/commands/postgres/register.go` | Create | `Register(parent)` wires every command twice: `postgres:*` + `pg:*`. Exports `RunInstall(args)` etc. for orchestrator use. |
| `internal/commands/postgres/install.go` | Create | `postgres:install [major]` cobra command. |
| `internal/commands/postgres/uninstall.go` | Create | `postgres:uninstall <major>` cobra command. |
| `internal/commands/postgres/update.go` | Create | `postgres:update <major>` cobra command. |
| `internal/commands/postgres/start.go` | Create | `postgres:start [major]` cobra command. |
| `internal/commands/postgres/stop.go` | Create | `postgres:stop [major]` cobra command. |
| `internal/commands/postgres/restart.go` | Create | `postgres:restart [major]` cobra command. |
| `internal/commands/postgres/list.go` | Create | `postgres:list` cobra command. |
| `internal/commands/postgres/logs.go` | Create | `postgres:logs [major] [-f]` cobra command. |
| `internal/commands/postgres/status.go` | Create | `postgres:status [major]` cobra command. |
| `internal/commands/postgres/download.go` | Create | `postgres:download <major>` (hidden) cobra command. |
| `internal/commands/postgres/dispatch.go` | Create | Disambiguation helper: resolves `[major]` arg via `InstalledMajors()`. |
| `internal/commands/postgres/dispatch_test.go` | Create | Unit tests for the disambiguation helper. |
| `cmd/postgres.go` | Create | Bridge: `init() { postgres.Register(rootCmd) }` + adds the `postgres` group. |
| `internal/registry/registry.go` | Modify | Add `UnbindPostgresMajor(major)` helper. |
| `internal/registry/registry_test.go` | Modify | Test the helper. |
| `internal/laravel/env.go` | Modify | New `UpdateProjectEnvForPostgres` helper that calls `postgres.EnvVars(...)`. |
| `internal/laravel/env_test.go` | Modify | Test the helper. |
| `internal/laravel/steps.go` | Modify | `CreateDatabaseStep` for postgres uses bundled `psql` via absolute path; mysql path unchanged. |
| `internal/automation/steps/detect_services.go` | Modify | Replace `findServiceByName(reg, "postgres")` with a call to `postgres.InstalledMajors()`. |
| `internal/automation/steps/detect_services_test.go` | Modify | Update postgres-binding test fixtures. |
| `internal/services/postgres.go` | Delete | Old docker `Postgres` struct. |
| `internal/services/postgres_test.go` | Delete | Tests for deleted struct. |
| `internal/services/service.go` | Modify | Drop `"postgres": &Postgres{}` from docker `registry` map. |
| `internal/services/lookup_test.go` | Modify | Drop postgres-specific cases. |
| `internal/commands/service/add.go` | Modify | Drop postgres from example text. |
| `internal/commands/service/hooks_test.go` | Modify | Migrate `Services.Postgres: "17"` fixtures to mysql or remove. |
| `internal/commands/setup/setup.go` | Modify | Drop postgres from the docker-services multi-select. |
| `scripts/e2e/postgres-binary.sh` | Create | E2E lifecycle test (install, list, status, link a project, uninstall). |
| `scripts/e2e/helpers.sh` | Modify (if needed) | Add a helper for waiting on a TCP port. |
| `.github/workflows/e2e.yml` | Modify | Add postgres-binary phase. |

---

## Task 1: Verify postgres tarball layout & contents

Research-only. Confirm assumptions before any code changes.

- [ ] **Step 1: Inspect the artifacts release**

```bash
curl -s https://api.github.com/repos/prvious/pv/releases/tags/artifacts \
  | jq -r '.assets[].name' | grep '^postgres-'
```

Expected output:
```
postgres-mac-arm64-17.tar.gz
postgres-mac-arm64-18.tar.gz
```

If either is missing, stop and verify the artifacts pipeline ran. Do NOT proceed.

- [ ] **Step 2: Download and extract one tarball**

```bash
cd /tmp
rm -rf pg-extract && mkdir pg-extract
curl -fsSL -o pg17.tar.gz "https://github.com/prvious/pv/releases/download/artifacts/postgres-mac-arm64-17.tar.gz"
tar -xzf pg17.tar.gz -C pg-extract
ls pg-extract
```

Expected: `bin lib share include` at the root (no nesting). If layout differs, amend the spec and update `internal/postgres/install.go` accordingly.

- [ ] **Step 3: Verify key binaries are present and runnable**

```bash
/tmp/pg-extract/bin/postgres --version
/tmp/pg-extract/bin/initdb --version
/tmp/pg-extract/bin/pg_isready --version
/tmp/pg-extract/bin/pg_config --version
/tmp/pg-extract/bin/psql --version
```

Each should print a version string. Record the output of `pg_config --version` — the format is `PostgreSQL X.Y` and we'll parse it in Task 11.

- [ ] **Step 4: Verify install_names are clean**

```bash
otool -L /tmp/pg-extract/bin/postgres | grep -E '/(opt/homebrew|Users/runner)' && echo "LEAK" || echo "CLEAN"
```

Expected: `CLEAN`. If `LEAK`, the artifacts pipeline regressed — stop.

- [ ] **Step 5: Smoke-test initdb + start + stop**

```bash
DATA=/tmp/pg-extract-data
rm -rf "$DATA"
/tmp/pg-extract/bin/initdb -D "$DATA" -U postgres --auth=trust --encoding=UTF8 --locale=C
echo 'port = 54199' >> "$DATA/postgresql.conf"
echo "listen_addresses = '127.0.0.1'" >> "$DATA/postgresql.conf"
echo "unix_socket_directories = '/tmp'" >> "$DATA/postgresql.conf"
/tmp/pg-extract/bin/postgres -D "$DATA" >/tmp/pg.log 2>&1 &
PG_PID=$!
sleep 2
/tmp/pg-extract/bin/pg_isready -h 127.0.0.1 -p 54199
/tmp/pg-extract/bin/psql -h 127.0.0.1 -p 54199 -U postgres -tAc "SELECT version();"
kill $PG_PID
wait $PG_PID 2>/dev/null
rm -rf "$DATA" /tmp/pg-extract /tmp/pg17.tar.gz /tmp/pg.log
```

Expected: `pg_isready` reports "accepting connections", `psql` returns a version string. If anything fails, stop and amend the spec / Task 11 / Task 13 (initdb args) accordingly.

---

## Task 2: `internal/state/` — generic per-service state file

**Files:**
- Create: `internal/state/state.go`
- Create: `internal/state/state_test.go`

Generic per-service state file at `~/.pv/data/state.json`. Top-level keyed by service name; each service's value is opaque JSON owned by that service's package.

- [ ] **Step 1: Write the failing test**

Create `internal/state/state_test.go`:

```go
package state

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func TestLoad_MissingFile_ReturnsEmpty(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	s, err := Load()
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	if got := len(s); got != 0 {
		t.Errorf("expected empty state, got %d entries", got)
	}
}

func TestSaveLoad_RoundTrip(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	s := State{}
	s["postgres"] = json.RawMessage(`{"majors":{"17":{"wanted":"running"}}}`)
	if err := Save(s); err != nil {
		t.Fatalf("Save: %v", err)
	}
	got, err := Load()
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	raw, ok := got["postgres"]
	if !ok {
		t.Fatal("expected postgres key after round-trip")
	}
	if string(raw) != `{"majors":{"17":{"wanted":"running"}}}` {
		t.Errorf("unexpected payload: %s", raw)
	}
}

func TestLoad_CorruptFile_ReturnsEmptyWithWarning(t *testing.T) {
	tmp := t.TempDir()
	t.Setenv("HOME", tmp)
	dataDir := filepath.Join(tmp, ".pv", "data")
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(dataDir, "state.json"), []byte("not json"), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}
	s, err := Load()
	if err != nil {
		t.Fatalf("Load should tolerate corruption, got: %v", err)
	}
	if len(s) != 0 {
		t.Errorf("expected empty state on corruption, got %d", len(s))
	}
}
```

- [ ] **Step 2: Run the test to confirm it fails**

```bash
go test ./internal/state/ -v
```

Expected: build error — package doesn't exist yet.

- [ ] **Step 3: Implement `internal/state/state.go`**

```go
// Package state owns the per-service runtime-state file at
// ~/.pv/data/state.json. The file is top-level keyed by service name; each
// service's value is opaque JSON owned by that service's package, so two
// services cannot accidentally collide on the same key.
package state

import (
	"encoding/json"
	"fmt"
	"os"

	"github.com/prvious/pv/internal/config"
)

// State maps service-name → opaque JSON payload.
type State map[string]json.RawMessage

// Load reads ~/.pv/data/state.json. A missing file returns an empty State
// (no error). A corrupt file logs a warning to stderr and returns empty.
func Load() (State, error) {
	path := config.StatePath()
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return State{}, nil
		}
		return nil, fmt.Errorf("state: read %s: %w", path, err)
	}
	var s State
	if err := json.Unmarshal(data, &s); err != nil {
		fmt.Fprintf(os.Stderr, "state: %s is corrupt (%v); treating as empty\n", path, err)
		return State{}, nil
	}
	if s == nil {
		s = State{}
	}
	return s, nil
}

// Save writes s to ~/.pv/data/state.json atomically (temp file + rename).
func Save(s State) error {
	if err := config.EnsureDirs(); err != nil {
		return err
	}
	data, err := json.MarshalIndent(s, "", "  ")
	if err != nil {
		return err
	}
	path := config.StatePath()
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, data, 0o644); err != nil {
		return fmt.Errorf("state: write tmp: %w", err)
	}
	if err := os.Rename(tmp, path); err != nil {
		os.Remove(tmp)
		return fmt.Errorf("state: rename: %w", err)
	}
	return nil
}
```

(`config.StatePath()` is added in Task 3.)

- [ ] **Step 4: Add the path helper to `internal/config/paths.go`**

In `internal/config/paths.go`, add after `VersionsPath`:

```go
func StatePath() string {
	return filepath.Join(DataDir(), "state.json")
}
```

- [ ] **Step 5: Run tests; expect pass**

```bash
go test ./internal/state/ -v
```

Expected: all three tests pass.

- [ ] **Step 6: gofmt + vet + build**

```bash
gofmt -w internal/state/ internal/config/paths.go
go vet ./...
go build ./...
```

- [ ] **Step 7: Commit**

```bash
git add internal/state/ internal/config/paths.go
git commit -m "feat(state): add per-service state file at ~/.pv/data/state.json"
```

---

## Task 3: Postgres path helpers

**Files:**
- Modify: `internal/config/paths.go`
- Modify: `internal/config/paths_test.go`

Centralize the postgres-binary paths so callers don't duplicate `filepath.Join`s.

- [ ] **Step 1: Write failing tests**

Append to `internal/config/paths_test.go`:

```go
func TestPostgresDir(t *testing.T) {
	t.Setenv("HOME", "/home/test")
	got := PostgresDir()
	want := "/home/test/.pv/postgres"
	if got != want {
		t.Errorf("PostgresDir = %q, want %q", got, want)
	}
}

func TestPostgresVersionDir(t *testing.T) {
	t.Setenv("HOME", "/home/test")
	got := PostgresVersionDir("17")
	want := "/home/test/.pv/postgres/17"
	if got != want {
		t.Errorf("PostgresVersionDir = %q, want %q", got, want)
	}
}

func TestPostgresBinDir(t *testing.T) {
	t.Setenv("HOME", "/home/test")
	got := PostgresBinDir("17")
	want := "/home/test/.pv/postgres/17/bin"
	if got != want {
		t.Errorf("PostgresBinDir = %q, want %q", got, want)
	}
}

func TestPostgresLogPath(t *testing.T) {
	t.Setenv("HOME", "/home/test")
	got := PostgresLogPath("17")
	want := "/home/test/.pv/logs/postgres-17.log"
	if got != want {
		t.Errorf("PostgresLogPath = %q, want %q", got, want)
	}
}
```

- [ ] **Step 2: Run tests, confirm failure**

```bash
go test ./internal/config/ -v -run TestPostgres
```

Expected: build error (functions undefined).

- [ ] **Step 3: Implement helpers**

Append to `internal/config/paths.go`:

```go
// PostgresDir is the root for native postgres binary trees:
// ~/.pv/postgres/<major>/{bin,lib,share,include}.
func PostgresDir() string {
	return filepath.Join(PvDir(), "postgres")
}

// PostgresVersionDir is the per-major root inside PostgresDir.
func PostgresVersionDir(major string) string {
	return filepath.Join(PostgresDir(), major)
}

// PostgresBinDir holds postgres + initdb + psql etc. for a major.
func PostgresBinDir(major string) string {
	return filepath.Join(PostgresVersionDir(major), "bin")
}

// PostgresLogPath returns the supervisor log file for a postgres major.
func PostgresLogPath(major string) string {
	return filepath.Join(LogsDir(), "postgres-"+major+".log")
}
```

- [ ] **Step 4: Run tests, confirm pass**

```bash
go test ./internal/config/ -v -run TestPostgres
```

- [ ] **Step 5: gofmt + vet + build**

```bash
gofmt -w internal/config/paths.go internal/config/paths_test.go
go vet ./...
go build ./...
```

- [ ] **Step 6: Commit**

```bash
git add internal/config/paths.go internal/config/paths_test.go
git commit -m "feat(config): add postgres path helpers"
```

---

## Task 4: `binaries.Postgres` descriptor

**Files:**
- Create: `internal/binaries/postgres.go`
- Create: `internal/binaries/postgres_test.go`
- Modify: `internal/binaries/manager.go`

Add a `Binary` descriptor + URL builder for postgres. Different from rustfs/mailpit because the URL is per-major (we don't fetch the latest patch — we always pull the rolling artifact).

- [ ] **Step 1: Write failing test**

Create `internal/binaries/postgres_test.go`:

```go
package binaries

import (
	"runtime"
	"testing"
)

func TestPostgresURL(t *testing.T) {
	if runtime.GOOS != "darwin" || runtime.GOARCH != "arm64" {
		t.Skip("postgres binaries only published for darwin/arm64 in v1")
	}
	got, err := PostgresURL("17")
	if err != nil {
		t.Fatalf("PostgresURL: %v", err)
	}
	want := "https://github.com/prvious/pv/releases/download/artifacts/postgres-mac-arm64-17.tar.gz"
	if got != want {
		t.Errorf("PostgresURL(17) = %q, want %q", got, want)
	}
}

func TestPostgresURL_UnsupportedPlatform(t *testing.T) {
	if runtime.GOOS == "darwin" && runtime.GOARCH == "arm64" {
		t.Skip("on supported platform; this test only runs elsewhere")
	}
	if _, err := PostgresURL("17"); err == nil {
		t.Error("PostgresURL should error on unsupported platform")
	}
}
```

- [ ] **Step 2: Run test, confirm failure**

```bash
go test ./internal/binaries/ -v -run TestPostgresURL
```

Expected: `undefined: PostgresURL`.

- [ ] **Step 3: Implement `internal/binaries/postgres.go`**

```go
package binaries

import (
	"fmt"
	"runtime"
)

// Postgres descriptor. Versioned by major; URL is per-major because the
// artifacts release is rolling (always carries latest patch of a major).
var Postgres = Binary{
	Name:         "postgres",
	DisplayName:  "PostgreSQL",
	NeedsExtract: true,
}

var postgresPlatformNames = map[string]map[string]string{
	"darwin": {
		"arm64": "mac-arm64",
	},
}

// PostgresURL returns the artifacts-release URL for the given major.
// Today only darwin/arm64 is published; other platforms error.
func PostgresURL(major string) (string, error) {
	archMap, ok := postgresPlatformNames[runtime.GOOS]
	if !ok {
		return "", fmt.Errorf("unsupported OS for PostgreSQL: %s", runtime.GOOS)
	}
	platform, ok := archMap[runtime.GOARCH]
	if !ok {
		return "", fmt.Errorf("unsupported architecture for PostgreSQL: %s/%s", runtime.GOOS, runtime.GOARCH)
	}
	return fmt.Sprintf("https://github.com/prvious/pv/releases/download/artifacts/postgres-%s-%s.tar.gz", platform, major), nil
}
```

- [ ] **Step 4: Run test, confirm pass**

```bash
go test ./internal/binaries/ -v -run TestPostgresURL
```

- [ ] **Step 5: gofmt + vet + build**

```bash
gofmt -w internal/binaries/
go vet ./...
go build ./...
```

- [ ] **Step 6: Commit**

```bash
git add internal/binaries/postgres.go internal/binaries/postgres_test.go
git commit -m "feat(binaries): add Postgres descriptor + URL builder"
```

---

## Task 5: `internal/postgres/port.go`

**Files:**
- Create: `internal/postgres/port.go`
- Create: `internal/postgres/port_test.go`

Port = 54000 + major. Same scheme docker postgres used; keep continuity.

- [ ] **Step 1: Write failing test**

Create `internal/postgres/port_test.go`:

```go
package postgres

import "testing"

func TestPortFor(t *testing.T) {
	tests := []struct {
		major string
		want  int
	}{
		{"17", 54017},
		{"18", 54018},
		{"99", 54099},
	}
	for _, tt := range tests {
		got, err := PortFor(tt.major)
		if err != nil {
			t.Errorf("PortFor(%q): %v", tt.major, err)
			continue
		}
		if got != tt.want {
			t.Errorf("PortFor(%q) = %d, want %d", tt.major, got, tt.want)
		}
	}
}

func TestPortFor_Invalid(t *testing.T) {
	if _, err := PortFor(""); err == nil {
		t.Error("PortFor empty should error")
	}
	if _, err := PortFor("18-alpine"); err == nil {
		t.Error("PortFor non-numeric should error")
	}
}
```

- [ ] **Step 2: Run test, confirm failure**

```bash
go test ./internal/postgres/ -v -run TestPortFor
```

- [ ] **Step 3: Implement `internal/postgres/port.go`**

```go
// Package postgres owns the lifecycle of native PostgreSQL majors managed
// by pv. Mirrors internal/phpenv/ — version-aware install, supervised
// processes, on-disk state at ~/.pv/postgres/<major>/.
package postgres

import (
	"fmt"
	"strconv"
)

// PortFor returns the TCP port a postgres major should bind to.
// Scheme: 54000 + major. Major must be a numeric string ("17", "18", …).
func PortFor(major string) (int, error) {
	n, err := strconv.Atoi(major)
	if err != nil {
		return 0, fmt.Errorf("postgres: invalid major %q: %w", major, err)
	}
	if n <= 0 {
		return 0, fmt.Errorf("postgres: invalid major %q (non-positive)", major)
	}
	return 54000 + n, nil
}
```

- [ ] **Step 4: Run test, confirm pass**

```bash
go test ./internal/postgres/ -v -run TestPortFor
```

- [ ] **Step 5: gofmt + vet + build**

```bash
gofmt -w internal/postgres/
go vet ./...
go build ./...
```

- [ ] **Step 6: Commit**

```bash
git add internal/postgres/port.go internal/postgres/port_test.go
git commit -m "feat(postgres): add PortFor helper"
```

---

## Task 6: `internal/postgres/installed.go`

**Files:**
- Create: `internal/postgres/installed.go`
- Create: `internal/postgres/installed_test.go`

Scan `~/.pv/postgres/<n>/` for installed majors. A major counts as installed if `bin/postgres` exists (the file, not just the dir).

- [ ] **Step 1: Write failing test**

Create `internal/postgres/installed_test.go`:

```go
package postgres

import (
	"os"
	"path/filepath"
	"sort"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestInstalledMajors_Empty(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	got, err := InstalledMajors()
	if err != nil {
		t.Fatalf("InstalledMajors: %v", err)
	}
	if len(got) != 0 {
		t.Errorf("expected empty, got %v", got)
	}
}

func TestInstalledMajors_FindsBinaries(t *testing.T) {
	tmp := t.TempDir()
	t.Setenv("HOME", tmp)
	for _, major := range []string{"17", "18"} {
		bin := filepath.Join(config.PostgresBinDir(major))
		if err := os.MkdirAll(bin, 0o755); err != nil {
			t.Fatalf("mkdir: %v", err)
		}
		if err := os.WriteFile(filepath.Join(bin, "postgres"), []byte("#!/bin/sh\n"), 0o755); err != nil {
			t.Fatalf("write: %v", err)
		}
	}
	got, err := InstalledMajors()
	if err != nil {
		t.Fatalf("InstalledMajors: %v", err)
	}
	sort.Strings(got)
	want := []string{"17", "18"}
	if len(got) != 2 || got[0] != want[0] || got[1] != want[1] {
		t.Errorf("InstalledMajors = %v, want %v", got, want)
	}
}

func TestInstalledMajors_DirWithoutBinary_NotInstalled(t *testing.T) {
	tmp := t.TempDir()
	t.Setenv("HOME", tmp)
	if err := os.MkdirAll(config.PostgresVersionDir("17"), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	got, err := InstalledMajors()
	if err != nil {
		t.Fatalf("InstalledMajors: %v", err)
	}
	if len(got) != 0 {
		t.Errorf("dir without bin/postgres should not count: got %v", got)
	}
}
```

- [ ] **Step 2: Run test, confirm failure**

```bash
go test ./internal/postgres/ -v -run TestInstalledMajors
```

- [ ] **Step 3: Implement `internal/postgres/installed.go`**

```go
package postgres

import (
	"os"
	"path/filepath"
	"sort"

	"github.com/prvious/pv/internal/config"
)

// InstalledMajors returns the sorted list of postgres majors that have a
// runnable bin/postgres on disk. A directory under ~/.pv/postgres/ with no
// bin/postgres is treated as not-installed (incomplete extraction, etc.).
func InstalledMajors() ([]string, error) {
	root := config.PostgresDir()
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
		major := e.Name()
		bin := filepath.Join(config.PostgresBinDir(major), "postgres")
		if info, err := os.Stat(bin); err == nil && !info.IsDir() {
			out = append(out, major)
		}
	}
	sort.Strings(out)
	return out, nil
}

// IsInstalled is a convenience wrapper for callers that want a yes/no.
func IsInstalled(major string) bool {
	bin := filepath.Join(config.PostgresBinDir(major), "postgres")
	info, err := os.Stat(bin)
	return err == nil && !info.IsDir()
}
```

- [ ] **Step 4: Run test, confirm pass**

```bash
go test ./internal/postgres/ -v -run TestInstalledMajors
```

- [ ] **Step 5: Commit**

```bash
gofmt -w internal/postgres/
go vet ./...
go build ./...
git add internal/postgres/installed.go internal/postgres/installed_test.go
git commit -m "feat(postgres): add InstalledMajors / IsInstalled"
```

---

## Task 7: `internal/postgres/state.go` — postgres-keyed wrapper around `internal/state`

**Files:**
- Create: `internal/postgres/state.go`
- Create: `internal/postgres/state_test.go`

Postgres's slice of the global state.json. Schema:
```json
{ "majors": { "17": { "wanted": "running" } } }
```

- [ ] **Step 1: Write failing test**

Create `internal/postgres/state_test.go`:

```go
package postgres

import "testing"

func TestState_DefaultEmpty(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	st, err := LoadState()
	if err != nil {
		t.Fatalf("LoadState: %v", err)
	}
	if len(st.Majors) != 0 {
		t.Errorf("expected empty, got %d", len(st.Majors))
	}
}

func TestState_SetAndPersist(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if err := SetWanted("17", "running"); err != nil {
		t.Fatalf("SetWanted: %v", err)
	}
	st, err := LoadState()
	if err != nil {
		t.Fatalf("LoadState: %v", err)
	}
	if got := st.Majors["17"].Wanted; got != "running" {
		t.Errorf("Wanted = %q, want running", got)
	}
}

func TestState_RemoveMajor(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	_ = SetWanted("17", "running")
	_ = SetWanted("18", "stopped")
	if err := RemoveMajor("17"); err != nil {
		t.Fatalf("RemoveMajor: %v", err)
	}
	st, err := LoadState()
	if err != nil {
		t.Fatalf("LoadState: %v", err)
	}
	if _, ok := st.Majors["17"]; ok {
		t.Error("17 should be removed")
	}
	if _, ok := st.Majors["18"]; !ok {
		t.Error("18 should still be present")
	}
}
```

- [ ] **Step 2: Run test, confirm failure**

```bash
go test ./internal/postgres/ -v -run TestState
```

- [ ] **Step 3: Implement `internal/postgres/state.go`**

```go
package postgres

import (
	"encoding/json"

	"github.com/prvious/pv/internal/state"
)

const stateKey = "postgres"

// MajorState is the per-major sub-record of postgres state.
type MajorState struct {
	Wanted string `json:"wanted"` // "running" | "stopped"
}

// State is the postgres slice of ~/.pv/data/state.json.
type State struct {
	Majors map[string]MajorState `json:"majors"`
}

// LoadState reads the postgres slice. Missing or empty → zero-value state.
func LoadState() (State, error) {
	all, err := state.Load()
	if err != nil {
		return State{Majors: map[string]MajorState{}}, err
	}
	raw, ok := all[stateKey]
	if !ok {
		return State{Majors: map[string]MajorState{}}, nil
	}
	var s State
	if err := json.Unmarshal(raw, &s); err != nil {
		return State{Majors: map[string]MajorState{}}, nil
	}
	if s.Majors == nil {
		s.Majors = map[string]MajorState{}
	}
	return s, nil
}

// SaveState writes the postgres slice, preserving other services' slices.
func SaveState(s State) error {
	all, err := state.Load()
	if err != nil {
		return err
	}
	if s.Majors == nil {
		s.Majors = map[string]MajorState{}
	}
	payload, err := json.Marshal(s)
	if err != nil {
		return err
	}
	all[stateKey] = payload
	return state.Save(all)
}

// SetWanted updates the wanted-state for one major and persists.
func SetWanted(major, wanted string) error {
	s, err := LoadState()
	if err != nil {
		return err
	}
	s.Majors[major] = MajorState{Wanted: wanted}
	return SaveState(s)
}

// RemoveMajor drops a major's entry from state and persists.
func RemoveMajor(major string) error {
	s, err := LoadState()
	if err != nil {
		return err
	}
	delete(s.Majors, major)
	return SaveState(s)
}
```

- [ ] **Step 4: Run test, confirm pass**

```bash
go test ./internal/postgres/ -v -run TestState
```

- [ ] **Step 5: Commit**

```bash
gofmt -w internal/postgres/
go vet ./...
go build ./...
git add internal/postgres/state.go internal/postgres/state_test.go
git commit -m "feat(postgres): add per-major state wrapper around internal/state"
```

---

## Task 8: `internal/postgres/wanted.go`

**Files:**
- Create: `internal/postgres/wanted.go`
- Create: `internal/postgres/wanted_test.go`

`WantedMajors()` = state-says-running ∩ installed-on-disk. Stale entries (state says running but binaries gone) get a one-line stderr warning and are filtered out.

- [ ] **Step 1: Write failing test**

Create `internal/postgres/wanted_test.go`:

```go
package postgres

import (
	"os"
	"path/filepath"
	"sort"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func installFakeMajor(t *testing.T, major string) {
	t.Helper()
	bin := config.PostgresBinDir(major)
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(bin, "postgres"), []byte{}, 0o755); err != nil {
		t.Fatalf("write: %v", err)
	}
}

func TestWantedMajors_Intersection(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	installFakeMajor(t, "17")
	installFakeMajor(t, "18")
	_ = SetWanted("17", "running")
	_ = SetWanted("18", "stopped")
	got, err := WantedMajors()
	if err != nil {
		t.Fatalf("WantedMajors: %v", err)
	}
	if len(got) != 1 || got[0] != "17" {
		t.Errorf("WantedMajors = %v, want [17]", got)
	}
}

func TestWantedMajors_StaleStateFiltered(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	// state says running but never installed
	_ = SetWanted("17", "running")
	got, err := WantedMajors()
	if err != nil {
		t.Fatalf("WantedMajors: %v", err)
	}
	if len(got) != 0 {
		t.Errorf("stale state should be filtered, got %v", got)
	}
}

func TestWantedMajors_SortedOutput(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	for _, m := range []string{"18", "17"} {
		installFakeMajor(t, m)
		_ = SetWanted(m, "running")
	}
	got, err := WantedMajors()
	if err != nil {
		t.Fatalf("WantedMajors: %v", err)
	}
	sorted := make([]string, len(got))
	copy(sorted, got)
	sort.Strings(sorted)
	for i := range got {
		if got[i] != sorted[i] {
			t.Errorf("output not sorted: %v", got)
			break
		}
	}
}
```

- [ ] **Step 2: Run test, confirm failure**

```bash
go test ./internal/postgres/ -v -run TestWantedMajors
```

- [ ] **Step 3: Implement `internal/postgres/wanted.go`**

```go
package postgres

import (
	"fmt"
	"os"
	"sort"
)

// WantedMajors returns the majors that should currently be supervised:
// majors marked wanted="running" in state.json AND installed on disk.
// Stale entries (state says running but binaries are missing) emit a
// stderr warning and are filtered out.
func WantedMajors() ([]string, error) {
	st, err := LoadState()
	if err != nil {
		return nil, err
	}
	installed, err := InstalledMajors()
	if err != nil {
		return nil, err
	}
	installedSet := map[string]struct{}{}
	for _, m := range installed {
		installedSet[m] = struct{}{}
	}
	var out []string
	for major, ms := range st.Majors {
		if ms.Wanted != "running" {
			continue
		}
		if _, ok := installedSet[major]; !ok {
			fmt.Fprintf(os.Stderr, "postgres: state.json wants %s running but binaries are missing; skipping\n", major)
			continue
		}
		out = append(out, major)
	}
	sort.Strings(out)
	return out, nil
}
```

- [ ] **Step 4: Run test, confirm pass**

```bash
go test ./internal/postgres/ -v -run TestWantedMajors
```

- [ ] **Step 5: Commit**

```bash
gofmt -w internal/postgres/
go vet ./...
go build ./...
git add internal/postgres/wanted.go internal/postgres/wanted_test.go
git commit -m "feat(postgres): add WantedMajors (state ∩ installed)"
```

---

## Task 9: `internal/postgres/version.go` — `pg_config --version` probe

**Files:**
- Create: `internal/postgres/version.go`
- Create: `internal/postgres/version_test.go`
- Create: `internal/postgres/testdata/fake-pg_config.go`

`ProbeVersion(major)` runs `<dir>/bin/pg_config --version`, parses `PostgreSQL 17.5` → `17.5`. The test uses a synthetic `pg_config` binary built with `go build` from a Go source under `testdata/` (per CLAUDE.md: no python/bash for test fakes — Go only).

- [ ] **Step 1: Write the test fake (Go program)**

Create `internal/postgres/testdata/fake-pg_config.go`:

```go
//go:build ignore

// Synthetic pg_config used by version_test.go.
// Compiled into the test temp dir at test time.
package main

import (
	"fmt"
	"os"
)

func main() {
	if len(os.Args) >= 2 && os.Args[1] == "--version" {
		fmt.Println("PostgreSQL 17.5")
		return
	}
	fmt.Fprintln(os.Stderr, "fake pg_config: unexpected args")
	os.Exit(2)
}
```

- [ ] **Step 2: Write failing test**

Create `internal/postgres/version_test.go`:

```go
package postgres

import (
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"testing"

	"github.com/prvious/pv/internal/config"
)

// buildFakePgConfig compiles the testdata fake-pg_config.go into binDir/pg_config.
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
```

- [ ] **Step 3: Run test, confirm failure**

```bash
go test ./internal/postgres/ -v -run TestProbeVersion
```

- [ ] **Step 4: Implement `internal/postgres/version.go`**

```go
package postgres

import (
	"fmt"
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/prvious/pv/internal/config"
)

// ProbeVersion runs `<bin>/pg_config --version` and returns the version
// component (e.g. "17.5" from "PostgreSQL 17.5"). The major argument
// selects the install root; the answer may be a patch within that major.
func ProbeVersion(major string) (string, error) {
	binPath := filepath.Join(config.PostgresBinDir(major), "pg_config")
	out, err := exec.Command(binPath, "--version").Output()
	if err != nil {
		return "", fmt.Errorf("pg_config --version: %w", err)
	}
	s := strings.TrimSpace(string(out))
	const prefix = "PostgreSQL "
	if !strings.HasPrefix(s, prefix) {
		return "", fmt.Errorf("unexpected pg_config output: %q", s)
	}
	return strings.TrimPrefix(s, prefix), nil
}
```

- [ ] **Step 5: Run test, confirm pass**

```bash
go test ./internal/postgres/ -v -run TestProbeVersion
```

- [ ] **Step 6: Commit**

```bash
gofmt -w internal/postgres/
go vet ./...
go build ./...
git add internal/postgres/version.go internal/postgres/version_test.go internal/postgres/testdata/fake-pg_config.go
git commit -m "feat(postgres): add ProbeVersion via pg_config"
```

---

## Task 10: `internal/postgres/conf.go` — postgresql.conf overrides + pg_hba.conf rewrite

**Files:**
- Create: `internal/postgres/conf.go`
- Create: `internal/postgres/conf_test.go`

`WriteOverrides(major)` appends a pv-managed block to the data dir's `postgresql.conf`. Idempotent: if the marker line is already present, the block is replaced rather than appended a second time. `RewriteHBA(major)` overwrites `pg_hba.conf` entirely (initdb's defaults are too narrow).

- [ ] **Step 1: Write failing test**

Create `internal/postgres/conf_test.go`:

```go
package postgres

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func setupFakeDataDir(t *testing.T, major string) string {
	t.Helper()
	dir := config.ServiceDataDir("postgres", major)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(dir, "postgresql.conf"), []byte("# initdb default\n"), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}
	return dir
}

func TestWriteOverrides(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	setupFakeDataDir(t, "17")
	if err := WriteOverrides("17"); err != nil {
		t.Fatalf("WriteOverrides: %v", err)
	}
	got, err := os.ReadFile(filepath.Join(config.ServiceDataDir("postgres", "17"), "postgresql.conf"))
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{
		"port = 54017",
		"listen_addresses = '127.0.0.1'",
		"unix_socket_directories = '/tmp/pv-postgres-17'",
		"fsync = on",
		"logging_collector = off",
	} {
		if !strings.Contains(string(got), want) {
			t.Errorf("missing %q in postgresql.conf:\n%s", want, got)
		}
	}
}

func TestWriteOverrides_Idempotent(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	setupFakeDataDir(t, "17")
	for i := 0; i < 3; i++ {
		if err := WriteOverrides("17"); err != nil {
			t.Fatalf("WriteOverrides #%d: %v", i, err)
		}
	}
	got, err := os.ReadFile(filepath.Join(config.ServiceDataDir("postgres", "17"), "postgresql.conf"))
	if err != nil {
		t.Fatal(err)
	}
	if c := strings.Count(string(got), "# pv-managed begin"); c != 1 {
		t.Errorf("expected 1 pv-managed block, got %d", c)
	}
}

func TestRewriteHBA(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	dir := config.ServiceDataDir("postgres", "17")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := RewriteHBA("17"); err != nil {
		t.Fatalf("RewriteHBA: %v", err)
	}
	got, err := os.ReadFile(filepath.Join(dir, "pg_hba.conf"))
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{
		"local   all             all                                     trust",
		"host    all             all             127.0.0.1/32            trust",
		"host    all             all             ::1/128                 trust",
	} {
		if !strings.Contains(string(got), want) {
			t.Errorf("missing %q in pg_hba.conf:\n%s", want, got)
		}
	}
}
```

- [ ] **Step 2: Run test, confirm failure**

```bash
go test ./internal/postgres/ -v -run "TestWriteOverrides|TestRewriteHBA"
```

- [ ] **Step 3: Implement `internal/postgres/conf.go`**

```go
package postgres

import (
	"fmt"
	"os"
	"path/filepath"
	"regexp"

	"github.com/prvious/pv/internal/config"
)

const (
	overridesBeginMarker = "# pv-managed begin"
	overridesEndMarker   = "# pv-managed end"
)

// pvManagedBlock matches our managed block (begin to end, inclusive of any
// content/newlines between). Used to strip the previous block before
// re-appending so multiple WriteOverrides calls don't pile up.
var pvManagedBlock = regexp.MustCompile(`(?ms)\n?# pv-managed begin.*?# pv-managed end\n`)

// WriteOverrides appends pv's postgresql.conf overrides for a major,
// replacing any previously-written pv block. Safe to call repeatedly.
func WriteOverrides(major string) error {
	port, err := PortFor(major)
	if err != nil {
		return err
	}
	confPath := filepath.Join(config.ServiceDataDir("postgres", major), "postgresql.conf")
	current, err := os.ReadFile(confPath)
	if err != nil {
		return fmt.Errorf("read postgresql.conf: %w", err)
	}
	stripped := pvManagedBlock.ReplaceAll(current, []byte("\n"))
	block := fmt.Sprintf(`
%s
# Managed by pv — do not hand-edit.
listen_addresses = '127.0.0.1'
port = %d
unix_socket_directories = '/tmp/pv-postgres-%s'
fsync = on
synchronous_commit = on
logging_collector = off
log_destination = 'stderr'
shared_buffers = 128MB
max_connections = 100
%s
`, overridesBeginMarker, port, major, overridesEndMarker)
	out := append(stripped, []byte(block)...)
	return os.WriteFile(confPath, out, 0o644)
}

// RewriteHBA writes the trust-only pg_hba.conf for a major.
// Loopback only — no external network exposure.
func RewriteHBA(major string) error {
	hbaPath := filepath.Join(config.ServiceDataDir("postgres", major), "pg_hba.conf")
	body := []byte(`# Managed by pv — do not hand-edit.
# TYPE  DATABASE        USER            ADDRESS                 METHOD
local   all             all                                     trust
host    all             all             127.0.0.1/32            trust
host    all             all             ::1/128                 trust
`)
	return os.WriteFile(hbaPath, body, 0o600)
}
```

- [ ] **Step 4: Run test, confirm pass**

```bash
go test ./internal/postgres/ -v -run "TestWriteOverrides|TestRewriteHBA"
```

- [ ] **Step 5: Commit**

```bash
gofmt -w internal/postgres/
go vet ./...
go build ./...
git add internal/postgres/conf.go internal/postgres/conf_test.go
git commit -m "feat(postgres): conf overrides + pg_hba rewrite"
```

---

## Task 11: `internal/postgres/initdb.go`

**Files:**
- Create: `internal/postgres/initdb.go`
- Create: `internal/postgres/initdb_test.go`

`RunInitdb(major)` invokes the bundled `initdb` against `~/.pv/services/postgres/<major>/data`. Idempotent: presence of `PG_VERSION` short-circuits. Cleans the partial data dir on failure so retry is clean.

The unit test stubs `initdb` with a Go fake (mirrors the pg_config approach in Task 9). The real e2e of initdb is exercised by Task 1's pre-flight + Task 36's e2e script.

- [ ] **Step 1: Add a fake initdb under testdata**

Create `internal/postgres/testdata/fake-initdb.go`:

```go
//go:build ignore

// Synthetic initdb used by initdb_test.go. Creates a PG_VERSION file at
// the path passed as `-D <dir>`, then exits 0.
package main

import (
	"fmt"
	"os"
	"path/filepath"
)

func main() {
	var dir string
	for i, a := range os.Args {
		if a == "-D" && i+1 < len(os.Args) {
			dir = os.Args[i+1]
		}
	}
	if dir == "" {
		fmt.Fprintln(os.Stderr, "fake-initdb: -D required")
		os.Exit(2)
	}
	if err := os.MkdirAll(dir, 0o755); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	if err := os.WriteFile(filepath.Join(dir, "PG_VERSION"), []byte("17\n"), 0o644); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	if err := os.WriteFile(filepath.Join(dir, "postgresql.conf"), []byte("# fake initdb\n"), 0o644); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
```

- [ ] **Step 2: Write failing test**

Create `internal/postgres/initdb_test.go`:

```go
package postgres

import (
	"os"
	"os/exec"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func buildFakeInitdb(t *testing.T, major string) {
	t.Helper()
	bin := config.PostgresBinDir(major)
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	cmd := exec.Command("go", "build", "-o", filepath.Join(bin, "initdb"), filepath.Join("testdata", "fake-initdb.go"))
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("go build fake-initdb: %v\n%s", err, out)
	}
}

func TestRunInitdb_FreshDataDir(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	buildFakeInitdb(t, "17")
	if err := RunInitdb("17"); err != nil {
		t.Fatalf("RunInitdb: %v", err)
	}
	pgVer := filepath.Join(config.ServiceDataDir("postgres", "17"), "PG_VERSION")
	if _, err := os.Stat(pgVer); err != nil {
		t.Errorf("PG_VERSION not created: %v", err)
	}
}

func TestRunInitdb_AlreadyInitialized_NoOp(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	dir := config.ServiceDataDir("postgres", "17")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(dir, "PG_VERSION"), []byte("17"), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}
	// fake initdb is NOT installed; if RunInitdb tried to invoke it, it'd fail.
	if err := RunInitdb("17"); err != nil {
		t.Errorf("RunInitdb on initialized dir should be a no-op, got: %v", err)
	}
}
```

- [ ] **Step 3: Run test, confirm failure**

```bash
go test ./internal/postgres/ -v -run TestRunInitdb
```

- [ ] **Step 4: Implement `internal/postgres/initdb.go`**

```go
package postgres

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"

	"github.com/prvious/pv/internal/config"
)

// RunInitdb runs the bundled initdb against the per-major data dir.
// Idempotent: if PG_VERSION is already present, returns nil immediately.
// On failure, removes the partially-created data dir so retry is clean.
func RunInitdb(major string) error {
	dataDir := config.ServiceDataDir("postgres", major)
	pgVersion := filepath.Join(dataDir, "PG_VERSION")
	if _, err := os.Stat(pgVersion); err == nil {
		return nil
	}
	if err := os.MkdirAll(filepath.Dir(dataDir), 0o755); err != nil {
		return fmt.Errorf("create services dir: %w", err)
	}

	binPath := filepath.Join(config.PostgresBinDir(major), "initdb")
	cmd := exec.Command(binPath,
		"-D", dataDir,
		"-U", "postgres",
		"--auth=trust",
		"--encoding=UTF8",
		"--locale=C",
	)
	out, err := cmd.CombinedOutput()
	if err != nil {
		os.RemoveAll(dataDir)
		return fmt.Errorf("initdb failed: %w\n%s", err, out)
	}
	return nil
}
```

- [ ] **Step 5: Run test, confirm pass**

```bash
go test ./internal/postgres/ -v -run TestRunInitdb
```

- [ ] **Step 6: Commit**

```bash
gofmt -w internal/postgres/
go vet ./...
go build ./...
git add internal/postgres/initdb.go internal/postgres/initdb_test.go internal/postgres/testdata/fake-initdb.go
git commit -m "feat(postgres): RunInitdb (idempotent, cleans partial data dir on fail)"
```

---

## Task 12: `internal/postgres/install.go` — orchestrator

**Files:**
- Create: `internal/postgres/install.go`
- Create: `internal/postgres/install_test.go`

End-to-end install: download → extract → chmod → initdb → conf overrides → version-record → state-mark-running.

`ExtractTarGz` in the binaries package today is single-binary (extracts one named entry). Postgres needs a *whole-tree* extract. We add a sibling helper `ExtractTarGzAll` in this task — see Step 0 below.

- [ ] **Step 0: Add `binaries.ExtractTarGzAll` (full-tree extractor)**

Modify `internal/binaries/download.go`. After the existing `ExtractTarGz` function, add:

```go
// ExtractTarGzAll extracts the entire archive into destDir, preserving
// directory structure and file modes. Refuses to extract entries that
// escape destDir (defense against path-traversal in archive entry names).
func ExtractTarGzAll(archivePath, destDir string) error {
	f, err := os.Open(archivePath)
	if err != nil {
		return err
	}
	defer f.Close()

	gz, err := gzip.NewReader(f)
	if err != nil {
		return fmt.Errorf("gzip open failed: %w", err)
	}
	defer gz.Close()

	if err := os.MkdirAll(destDir, 0o755); err != nil {
		return err
	}
	absDest, err := filepath.Abs(destDir)
	if err != nil {
		return err
	}

	tr := tar.NewReader(gz)
	for {
		hdr, err := tr.Next()
		if err == io.EOF {
			break
		}
		if err != nil {
			return fmt.Errorf("tar read failed: %w", err)
		}

		target := filepath.Join(destDir, hdr.Name)
		absTarget, err := filepath.Abs(target)
		if err != nil {
			return err
		}
		if !strings.HasPrefix(absTarget, absDest+string(os.PathSeparator)) && absTarget != absDest {
			return fmt.Errorf("tar entry escapes dest: %s", hdr.Name)
		}

		switch hdr.Typeflag {
		case tar.TypeDir:
			if err := os.MkdirAll(target, os.FileMode(hdr.Mode)&0o777); err != nil {
				return err
			}
		case tar.TypeReg:
			if err := os.MkdirAll(filepath.Dir(target), 0o755); err != nil {
				return err
			}
			out, err := os.OpenFile(target, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, os.FileMode(hdr.Mode)&0o777)
			if err != nil {
				return err
			}
			if _, err := io.Copy(out, tr); err != nil {
				out.Close()
				return err
			}
			if err := out.Close(); err != nil {
				return err
			}
		case tar.TypeSymlink:
			os.Remove(target)
			if err := os.Symlink(hdr.Linkname, target); err != nil {
				return err
			}
		}
	}
	return nil
}
```

Verify it compiles:

```bash
go build ./internal/binaries/...
```

- [ ] **Step 1: Write failing install test (mock HTTP server)**

Create `internal/postgres/install_test.go`:

```go
package postgres

import (
	"archive/tar"
	"bytes"
	"compress/gzip"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
)

// makeFakeTarball returns a minimal postgres-like tarball: bin/postgres,
// bin/initdb (a stub that creates PG_VERSION), bin/pg_config (echoes a
// version), share/postgresql/postgresql.conf.sample.
func makeFakeTarball(t *testing.T) []byte {
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
	add("bin/postgres", 0o755, "#!/bin/sh\nsleep 60\n")
	add("bin/initdb", 0o755, "#!/bin/sh\nfor a in \"$@\"; do prev=$x; x=$a; if [ \"$prev\" = \"-D\" ]; then mkdir -p \"$x\" && echo 17 > \"$x/PG_VERSION\" && echo \"# stub\" > \"$x/postgresql.conf\"; fi; done\n")
	add("bin/pg_config", 0o755, "#!/bin/sh\necho \"PostgreSQL 17.5\"\n")
	add("share/postgresql/postgresql.conf.sample", 0o644, "# sample\n")
	tw.Close()
	gz.Close()
	return buf.Bytes()
}

func TestInstall_HappyPath(t *testing.T) {
	tarball := makeFakeTarball(t)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/gzip")
		w.Write(tarball)
	}))
	defer srv.Close()

	t.Setenv("HOME", t.TempDir())
	t.Setenv("PV_POSTGRES_URL_OVERRIDE", srv.URL)

	if err := Install(http.DefaultClient, "17"); err != nil {
		t.Fatalf("Install: %v", err)
	}

	// Binaries on disk.
	for _, want := range []string{"bin/postgres", "bin/initdb", "bin/pg_config"} {
		p := filepath.Join(config.PostgresVersionDir("17"), want)
		if _, err := os.Stat(p); err != nil {
			t.Errorf("missing %s: %v", want, err)
		}
	}

	// Data dir initialized.
	if _, err := os.Stat(filepath.Join(config.ServiceDataDir("postgres", "17"), "PG_VERSION")); err != nil {
		t.Errorf("PG_VERSION not created: %v", err)
	}

	// State recorded.
	st, _ := LoadState()
	if st.Majors["17"].Wanted != "running" {
		t.Errorf("state.wanted = %q, want running", st.Majors["17"].Wanted)
	}
}

func TestInstall_AlreadyInstalled_Idempotent(t *testing.T) {
	tarball := makeFakeTarball(t)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write(tarball)
	}))
	defer srv.Close()

	t.Setenv("HOME", t.TempDir())
	t.Setenv("PV_POSTGRES_URL_OVERRIDE", srv.URL)

	if err := Install(http.DefaultClient, "17"); err != nil {
		t.Fatalf("first Install: %v", err)
	}
	if err := Install(http.DefaultClient, "17"); err != nil {
		t.Fatalf("second Install (idempotent): %v", err)
	}
}
```

- [ ] **Step 2: Run test, confirm failure**

```bash
go test ./internal/postgres/ -v -run TestInstall
```

- [ ] **Step 3: Implement `internal/postgres/install.go`**

```go
package postgres

import (
	"fmt"
	"net/http"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
)

// Install downloads, extracts, initdb's, and registers a postgres major
// as "wanted=running". Idempotent: re-running on an already-installed
// major just re-emits conf overrides and re-marks state.
func Install(client *http.Client, major string) error {
	return InstallProgress(client, major, nil)
}

// InstallProgress is Install with a progress callback for the download phase.
func InstallProgress(client *http.Client, major string, progress binaries.ProgressFunc) error {
	if err := config.EnsureDirs(); err != nil {
		return err
	}

	url, err := resolvePostgresURL(major)
	if err != nil {
		return err
	}

	versionDir := config.PostgresVersionDir(major)
	if !IsInstalled(major) {
		stagingDir := versionDir + ".new"
		os.RemoveAll(stagingDir)
		if err := os.MkdirAll(stagingDir, 0o755); err != nil {
			return fmt.Errorf("create staging: %w", err)
		}
		archive := filepath.Join(config.PostgresDir(), "postgres-"+major+".tar.gz")
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
	}

	if err := RunInitdb(major); err != nil {
		return err
	}
	if err := WriteOverrides(major); err != nil {
		return err
	}
	if err := RewriteHBA(major); err != nil {
		return err
	}

	if v, err := ProbeVersion(major); err == nil {
		vs, err := binaries.LoadVersions()
		if err == nil {
			vs.Set("postgres-"+major, v)
			_ = vs.Save()
		}
	}

	return SetWanted(major, "running")
}

// resolvePostgresURL allows tests to redirect the download via env var.
// Production: returns the artifacts-release URL from binaries.PostgresURL.
func resolvePostgresURL(major string) (string, error) {
	if override := os.Getenv("PV_POSTGRES_URL_OVERRIDE"); override != "" {
		return override, nil
	}
	return binaries.PostgresURL(major)
}
```

- [ ] **Step 4: Run test, confirm pass**

```bash
go test ./internal/postgres/ -v -run TestInstall
```

- [ ] **Step 5: Commit**

```bash
gofmt -w internal/postgres/ internal/binaries/
go vet ./...
go build ./...
git add internal/postgres/install.go internal/postgres/install_test.go internal/binaries/download.go
git commit -m "feat(postgres): Install orchestrator (download + initdb + conf + state)"
```

---

## Task 13: `internal/postgres/uninstall.go`

**Files:**
- Create: `internal/postgres/uninstall.go`
- Create: `internal/postgres/uninstall_test.go`

`Uninstall(major)` removes the data dir, the binary tree, the log file, the state entry, the version-tracking entry. Caller is responsible for stopping the process first via state + daemon signal — `Uninstall` itself does not signal the daemon.

- [ ] **Step 1: Write failing test**

Create `internal/postgres/uninstall_test.go`:

```go
package postgres

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
)

func setupFakeInstall(t *testing.T, major string) {
	t.Helper()
	bin := config.PostgresBinDir(major)
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(bin, "postgres"), []byte{}, 0o755); err != nil {
		t.Fatal(err)
	}
	dataDir := config.ServiceDataDir("postgres", major)
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		t.Fatal(err)
	}
	os.WriteFile(filepath.Join(dataDir, "PG_VERSION"), []byte("17"), 0o644)
	logDir := config.LogsDir()
	os.MkdirAll(logDir, 0o755)
	os.WriteFile(config.PostgresLogPath(major), []byte("log"), 0o644)
	_ = SetWanted(major, "running")
	vs, _ := binaries.LoadVersions()
	vs.Set("postgres-"+major, "17.5")
	_ = vs.Save()
}

func TestUninstall_RemovesEverything(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	setupFakeInstall(t, "17")
	if err := Uninstall("17"); err != nil {
		t.Fatalf("Uninstall: %v", err)
	}
	if _, err := os.Stat(config.PostgresVersionDir("17")); !os.IsNotExist(err) {
		t.Errorf("version dir not removed: %v", err)
	}
	if _, err := os.Stat(config.ServiceDataDir("postgres", "17")); !os.IsNotExist(err) {
		t.Errorf("data dir not removed: %v", err)
	}
	if _, err := os.Stat(config.PostgresLogPath("17")); !os.IsNotExist(err) {
		t.Errorf("log not removed: %v", err)
	}
	st, _ := LoadState()
	if _, ok := st.Majors["17"]; ok {
		t.Error("state entry not removed")
	}
	vs, _ := binaries.LoadVersions()
	if got := vs.Get("postgres-17"); got != "" {
		t.Errorf("version entry not removed: %q", got)
	}
}

func TestUninstall_Missing_NoOp(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if err := Uninstall("17"); err != nil {
		t.Errorf("Uninstall on missing major should be a no-op, got: %v", err)
	}
}
```

- [ ] **Step 2: Run test, confirm failure**

```bash
go test ./internal/postgres/ -v -run TestUninstall
```

- [ ] **Step 3: Implement `internal/postgres/uninstall.go`**

```go
package postgres

import (
	"os"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
)

// Uninstall removes all on-disk state for a major: data dir, binary tree,
// log file, state entry, version-tracking entry. Missing major is a no-op.
// Caller must stop the supervised process before calling.
func Uninstall(major string) error {
	if err := os.RemoveAll(config.ServiceDataDir("postgres", major)); err != nil {
		return err
	}
	if err := os.RemoveAll(config.PostgresVersionDir(major)); err != nil {
		return err
	}
	_ = os.Remove(config.PostgresLogPath(major))
	if err := RemoveMajor(major); err != nil {
		return err
	}
	if vs, err := binaries.LoadVersions(); err == nil {
		delete(vs.Versions, "postgres-"+major)
		_ = vs.Save()
	}
	return nil
}
```

- [ ] **Step 4: Run test, confirm pass**

```bash
go test ./internal/postgres/ -v -run TestUninstall
```

- [ ] **Step 5: Commit**

```bash
gofmt -w internal/postgres/
go vet ./...
go build ./...
git add internal/postgres/uninstall.go internal/postgres/uninstall_test.go
git commit -m "feat(postgres): Uninstall (rm data + bin + log + state + version)"
```

---

## Task 14: `internal/postgres/update.go`

**Files:**
- Create: `internal/postgres/update.go`
- Create: `internal/postgres/update_test.go`

`Update(client, major)` redownloads the tarball over the install dir, atomically. Data dir untouched (PG_VERSION already exists, so initdb skips). Re-emits conf overrides + pg_hba in case pv defaults changed. Marks `wanted=running`. Caller is responsible for first transitioning the supervisor to stopped.

- [ ] **Step 1: Write failing test**

Create `internal/postgres/update_test.go`:

```go
package postgres

import (
	"archive/tar"
	"bytes"
	"compress/gzip"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestUpdate_LeavesDataDirIntact(t *testing.T) {
	// Pre-populate a "v1" install with a marker file.
	t.Setenv("HOME", t.TempDir())
	dataDir := config.ServiceDataDir("postgres", "17")
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dataDir, "PG_VERSION"), []byte("17"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dataDir, "postgresql.conf"), []byte("# pre-existing\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dataDir, "MARKER"), []byte("DO_NOT_TOUCH"), 0o644); err != nil {
		t.Fatal(err)
	}
	bin := config.PostgresBinDir("17")
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatal(err)
	}
	os.WriteFile(filepath.Join(bin, "postgres"), []byte("v1"), 0o755)

	// Serve a "v2" tarball.
	var buf bytes.Buffer
	gz := gzip.NewWriter(&buf)
	tw := tar.NewWriter(gz)
	hdr := &tar.Header{Name: "bin/postgres", Mode: 0o755, Size: 2, Typeflag: tar.TypeReg}
	tw.WriteHeader(hdr)
	tw.Write([]byte("v2"))
	hdr2 := &tar.Header{Name: "bin/pg_config", Mode: 0o755, Size: 38, Typeflag: tar.TypeReg}
	tw.WriteHeader(hdr2)
	tw.Write([]byte("#!/bin/sh\necho \"PostgreSQL 17.6\"\n"))
	tw.Close()
	gz.Close()

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write(buf.Bytes())
	}))
	defer srv.Close()
	t.Setenv("PV_POSTGRES_URL_OVERRIDE", srv.URL)

	if err := Update(http.DefaultClient, "17"); err != nil {
		t.Fatalf("Update: %v", err)
	}

	// Marker file in data dir should still exist.
	if _, err := os.Stat(filepath.Join(dataDir, "MARKER")); err != nil {
		t.Errorf("data dir clobbered: %v", err)
	}
	// Binary should be the new version.
	got, _ := os.ReadFile(filepath.Join(bin, "postgres"))
	if string(got) != "v2" {
		t.Errorf("binary not updated: got %q", got)
	}
	// state should be wanted=running after update.
	st, _ := LoadState()
	if st.Majors["17"].Wanted != "running" {
		t.Errorf("post-update state.wanted = %q, want running", st.Majors["17"].Wanted)
	}
}
```

- [ ] **Step 2: Run test, confirm failure**

```bash
go test ./internal/postgres/ -v -run TestUpdate
```

- [ ] **Step 3: Implement `internal/postgres/update.go`**

```go
package postgres

import (
	"fmt"
	"net/http"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
)

// Update redownloads the postgres tarball for a major and re-applies
// conf overrides. Data dir is untouched. Marks wanted=running on success.
// Caller must have stopped the supervised process before calling.
func Update(client *http.Client, major string) error {
	return UpdateProgress(client, major, nil)
}

// UpdateProgress is Update with a download progress callback.
func UpdateProgress(client *http.Client, major string, progress binaries.ProgressFunc) error {
	if !IsInstalled(major) {
		return fmt.Errorf("postgres %s is not installed", major)
	}

	url, err := resolvePostgresURL(major)
	if err != nil {
		return err
	}

	versionDir := config.PostgresVersionDir(major)
	stagingDir := versionDir + ".new"
	os.RemoveAll(stagingDir)
	if err := os.MkdirAll(stagingDir, 0o755); err != nil {
		return fmt.Errorf("create staging: %w", err)
	}

	archive := filepath.Join(config.PostgresDir(), "postgres-"+major+".tar.gz")
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

	// Atomic swap.
	oldDir := versionDir + ".old"
	os.RemoveAll(oldDir)
	if err := os.Rename(versionDir, oldDir); err != nil {
		os.RemoveAll(stagingDir)
		return fmt.Errorf("rename old: %w", err)
	}
	if err := os.Rename(stagingDir, versionDir); err != nil {
		os.Rename(oldDir, versionDir) // best-effort restore
		return fmt.Errorf("rename new: %w", err)
	}
	os.RemoveAll(oldDir)

	if err := WriteOverrides(major); err != nil {
		return err
	}
	if err := RewriteHBA(major); err != nil {
		return err
	}

	if v, err := ProbeVersion(major); err == nil {
		if vs, err := binaries.LoadVersions(); err == nil {
			vs.Set("postgres-"+major, v)
			_ = vs.Save()
		}
	}

	return SetWanted(major, "running")
}
```

- [ ] **Step 4: Run test, confirm pass**

```bash
go test ./internal/postgres/ -v -run TestUpdate
```

- [ ] **Step 5: Commit**

```bash
gofmt -w internal/postgres/
go vet ./...
go build ./...
git add internal/postgres/update.go internal/postgres/update_test.go
git commit -m "feat(postgres): Update (atomic swap, data dir untouched)"
```

---

## Task 15: `internal/postgres/envvars.go`

**Files:**
- Create: `internal/postgres/envvars.go`
- Create: `internal/postgres/envvars_test.go`

Free function `EnvVars(projectName, major)` returns the `DB_*` map. Mirrors what the deleted docker `Postgres.EnvVars` produced, plus port computed via `PortFor(major)`.

- [ ] **Step 1: Write failing test**

Create `internal/postgres/envvars_test.go`:

```go
package postgres

import "testing"

func TestEnvVars_Golden(t *testing.T) {
	got, err := EnvVars("my_app", "17")
	if err != nil {
		t.Fatalf("EnvVars: %v", err)
	}
	want := map[string]string{
		"DB_CONNECTION": "pgsql",
		"DB_HOST":       "127.0.0.1",
		"DB_PORT":       "54017",
		"DB_DATABASE":   "my_app",
		"DB_USERNAME":   "postgres",
		"DB_PASSWORD":   "postgres",
	}
	for k, v := range want {
		if got[k] != v {
			t.Errorf("%s = %q, want %q", k, got[k], v)
		}
	}
}

func TestEnvVars_Pg18Port(t *testing.T) {
	got, err := EnvVars("my_app", "18")
	if err != nil {
		t.Fatalf("EnvVars: %v", err)
	}
	if got["DB_PORT"] != "54018" {
		t.Errorf("DB_PORT = %q, want 54018", got["DB_PORT"])
	}
}
```

- [ ] **Step 2: Run test, confirm failure**

```bash
go test ./internal/postgres/ -v -run TestEnvVars
```

- [ ] **Step 3: Implement `internal/postgres/envvars.go`**

```go
package postgres

import "fmt"

// EnvVars returns the DB_* map injected into a linked project's .env.
// projectName is sanitized by the caller (services.SanitizeProjectName).
func EnvVars(projectName, major string) (map[string]string, error) {
	port, err := PortFor(major)
	if err != nil {
		return nil, err
	}
	return map[string]string{
		"DB_CONNECTION": "pgsql",
		"DB_HOST":       "127.0.0.1",
		"DB_PORT":       fmt.Sprintf("%d", port),
		"DB_DATABASE":   projectName,
		"DB_USERNAME":   "postgres",
		"DB_PASSWORD":   "postgres",
	}, nil
}
```

- [ ] **Step 4: Run test, confirm pass**

```bash
go test ./internal/postgres/ -v -run TestEnvVars
```

- [ ] **Step 5: Commit**

```bash
gofmt -w internal/postgres/
go vet ./...
go build ./...
git add internal/postgres/envvars.go internal/postgres/envvars_test.go
git commit -m "feat(postgres): EnvVars helper for project .env injection"
```

---

## Task 16: `internal/postgres/process.go` — `BuildSupervisorProcess`

**Files:**
- Create: `internal/postgres/process.go`
- Create: `internal/postgres/process_test.go`

Returns a `supervisor.Process` for a major. Refuses to build for an uninitialized data dir. Port is implicit via `postgresql.conf`, not on the command line.

- [ ] **Step 1: Write failing test**

Create `internal/postgres/process_test.go`:

```go
package postgres

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestBuildSupervisorProcess_NotInitialized_Errors(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if _, err := BuildSupervisorProcess("17"); err == nil {
		t.Error("expected error when data dir not initialized")
	}
}

func TestBuildSupervisorProcess_HappyPath(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	dataDir := config.ServiceDataDir("postgres", "17")
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		t.Fatal(err)
	}
	os.WriteFile(filepath.Join(dataDir, "PG_VERSION"), []byte("17"), 0o644)

	p, err := BuildSupervisorProcess("17")
	if err != nil {
		t.Fatalf("BuildSupervisorProcess: %v", err)
	}
	if p.Name != "postgres-17" {
		t.Errorf("Name = %q, want postgres-17", p.Name)
	}
	if !strings.HasSuffix(p.Binary, "/postgres/17/bin/postgres") {
		t.Errorf("Binary = %q, expected to end with /postgres/17/bin/postgres", p.Binary)
	}
	wantArgs := []string{"-D", dataDir}
	if len(p.Args) != 2 || p.Args[0] != wantArgs[0] || p.Args[1] != wantArgs[1] {
		t.Errorf("Args = %v, want %v", p.Args, wantArgs)
	}
	if !strings.HasSuffix(p.LogFile, "/logs/postgres-17.log") {
		t.Errorf("LogFile = %q", p.LogFile)
	}
	if p.Ready == nil {
		t.Error("Ready func is nil")
	}
}
```

- [ ] **Step 2: Run test, confirm failure**

```bash
go test ./internal/postgres/ -v -run TestBuildSupervisorProcess
```

- [ ] **Step 3: Implement `internal/postgres/process.go`**

```go
package postgres

import (
	"context"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"time"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/supervisor"
)

// BuildSupervisorProcess returns a supervisor.Process for a postgres major.
// Refuses to build for a data dir without PG_VERSION (i.e., not yet
// initialized). Port comes from postgresql.conf — not the command line —
// so there's a single source of truth.
func BuildSupervisorProcess(major string) (supervisor.Process, error) {
	dataDir := config.ServiceDataDir("postgres", major)
	if _, err := os.Stat(filepath.Join(dataDir, "PG_VERSION")); err != nil {
		return supervisor.Process{}, fmt.Errorf("postgres %s: data dir not initialized (run pv postgres:install %s)", major, major)
	}
	port, err := PortFor(major)
	if err != nil {
		return supervisor.Process{}, err
	}
	binary := filepath.Join(config.PostgresBinDir(major), "postgres")
	return supervisor.Process{
		Name:         "postgres-" + major,
		Binary:       binary,
		Args:         []string{"-D", dataDir},
		LogFile:      config.PostgresLogPath(major),
		Ready:        tcpReady(port),
		ReadyTimeout: 30 * time.Second,
	}, nil
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

- [ ] **Step 4: Run test, confirm pass**

```bash
go test ./internal/postgres/ -v -run TestBuildSupervisorProcess
```

- [ ] **Step 5: Commit**

```bash
gofmt -w internal/postgres/
go vet ./...
go build ./...
git add internal/postgres/process.go internal/postgres/process_test.go
git commit -m "feat(postgres): BuildSupervisorProcess for daemon supervision"
```

---

## Task 17: Extend `reconcileBinaryServices` with the postgres source

**Files:**
- Modify: `internal/server/manager.go`
- Modify: `internal/server/manager_test.go`

Existing `reconcileBinaryServices` reads from `services.AllBinary()` ∩ `reg.Services[Kind=binary, Enabled]`. Add a second source: `postgres.WantedMajors()` → `postgres.BuildSupervisorProcess(major)`. The diff/start/stop loop is unified.

- [ ] **Step 1: Write failing test**

Append to `internal/server/manager_test.go` (file may need to be created if it doesn't exist; check first):

```bash
ls internal/server/manager_test.go 2>/dev/null && echo EXISTS || echo CREATE
```

If `CREATE`, scaffold a minimal test file with package and imports first. Then add:

```go
func TestReconcileBinaryServices_StartsWantedPostgres(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	// Pre-stage an installed major + state-marked-running so reconciler
	// will want to start it.
	bin := config.PostgresBinDir("17")
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatal(err)
	}
	// A binary that doesn't exit immediately, so the supervisor can mark it running.
	// Use a Go-built fake (not a python/bash one-liner — see CLAUDE.md).
	cmd := exec.Command("go", "build", "-o", filepath.Join(bin, "postgres"),
		filepath.Join("..", "..", "internal", "postgres", "testdata", "fake-postgres-server.go"))
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("build fake postgres: %v\n%s", err, out)
	}
	dataDir := config.ServiceDataDir("postgres", "17")
	os.MkdirAll(dataDir, 0o755)
	os.WriteFile(filepath.Join(dataDir, "PG_VERSION"), []byte("17"), 0o644)
	if err := postgres.WriteOverrides("17"); err != nil {
		t.Fatal(err)
	}
	if err := postgres.SetWanted("17", "running"); err != nil {
		t.Fatal(err)
	}

	sup := supervisor.New()
	mgr := NewServerManager(nil, sup)

	// Reconcile (we only exercise the binary-services phase here).
	if err := mgr.reconcileBinaryServices(context.Background()); err != nil {
		t.Fatalf("reconcileBinaryServices: %v", err)
	}

	if !sup.IsRunning("postgres-17") {
		t.Error("expected postgres-17 to be supervised after reconcile")
	}
	_ = sup.StopAll(2 * time.Second)
}
```

Add the matching `internal/postgres/testdata/fake-postgres-server.go`:

```go
//go:build ignore

// Synthetic postgres server used by manager_test. Reads -D <dir>, opens
// the data dir's postgresql.conf to discover the port, and binds a TCP
// listener so the supervisor's TCP ready-check passes.
package main

import (
	"net"
	"os"
	"os/signal"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"syscall"
)

func main() {
	var dir string
	for i, a := range os.Args {
		if a == "-D" && i+1 < len(os.Args) {
			dir = os.Args[i+1]
		}
	}
	if dir == "" {
		os.Exit(2)
	}
	conf, _ := os.ReadFile(filepath.Join(dir, "postgresql.conf"))
	re := regexp.MustCompile(`(?m)^port\s*=\s*(\d+)`)
	m := re.FindStringSubmatch(string(conf))
	port := 54017
	if len(m) == 2 {
		if n, err := strconv.Atoi(strings.TrimSpace(m[1])); err == nil {
			port = n
		}
	}
	l, err := net.Listen("tcp", "127.0.0.1:"+strconv.Itoa(port))
	if err != nil {
		os.Exit(3)
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

- [ ] **Step 2: Run the test, confirm failure**

```bash
go test ./internal/server/ -v -run TestReconcileBinaryServices_StartsWantedPostgres
```

Expected failure: postgres source not yet wired into reconciler.

- [ ] **Step 3: Modify `reconcileBinaryServices` in `internal/server/manager.go`**

Replace the existing function body. Locate the current implementation (around line 170) and rewrite as:

```go
// reconcileBinaryServices brings supervisor state in line with the wanted
// set computed from two sources:
//   1. registry: single-version services (rustfs, mailpit) marked Kind=binary
//      and Enabled.
//   2. internal/postgres: multi-version, on-disk + state.json driven.
// The diff/start/stop loop is shared across both sources.
func (m *ServerManager) reconcileBinaryServices(ctx context.Context) error {
	if m.supervisor == nil {
		return nil
	}

	reg, err := registry.Load()
	if err != nil {
		return fmt.Errorf("reconcile binary: load registry: %w", err)
	}

	// wanted: supervisorKey -> buildable supervisor.Process.
	wanted := map[string]supervisor.Process{}
	var startErrors []string

	// Source 1 — single-version binary services.
	for name, svc := range services.AllBinary() {
		entry := reg.Services[name]
		if entry == nil || entry.Kind != "binary" {
			continue
		}
		if entry.Enabled != nil && !*entry.Enabled {
			continue
		}
		proc, err := buildSupervisorProcess(svc)
		if err != nil {
			startErrors = append(startErrors, fmt.Sprintf("%s: build: %v", name, err))
			continue
		}
		wanted[svc.Binary().Name] = proc
	}

	// Source 2 — postgres, multi-version.
	pgMajors, err := postgres.WantedMajors()
	if err != nil {
		fmt.Fprintf(os.Stderr, "reconcile binary: postgres.WantedMajors: %v\n", err)
	}
	for _, major := range pgMajors {
		proc, err := postgres.BuildSupervisorProcess(major)
		if err != nil {
			startErrors = append(startErrors, fmt.Sprintf("postgres-%s: build: %v", major, err))
			continue
		}
		wanted["postgres-"+major] = proc
	}

	// Diff: stop unneeded.
	for _, supKey := range m.supervisor.SupervisedNames() {
		if _, ok := wanted[supKey]; !ok {
			if err := m.supervisor.Stop(supKey, 10*time.Second); err != nil {
				fmt.Fprintf(os.Stderr, "reconcile binary: stop %s: %v\n", supKey, err)
			}
		}
	}

	// Diff: start needed.
	for supKey, proc := range wanted {
		if m.supervisor.IsRunning(supKey) {
			continue
		}
		if err := m.supervisor.Start(ctx, proc); err != nil {
			startErrors = append(startErrors, fmt.Sprintf("%s: start: %v", supKey, err))
			continue
		}
	}

	if len(startErrors) > 0 {
		return fmt.Errorf("binary reconcile: %d service(s) failed: %s", len(startErrors), strings.Join(startErrors, "; "))
	}
	return nil
}
```

Add the import:
```go
"github.com/prvious/pv/internal/postgres"
```

- [ ] **Step 4: Run the test, confirm pass**

```bash
go test ./internal/server/ -v -run TestReconcileBinaryServices_StartsWantedPostgres
go test ./internal/server/ -v   # full server tests still pass
```

- [ ] **Step 5: Commit**

```bash
gofmt -w internal/server/ internal/postgres/
go vet ./...
go build ./...
git add internal/server/manager.go internal/server/manager_test.go internal/postgres/testdata/fake-postgres-server.go
git commit -m "feat(server): reconcileBinaryServices picks up postgres majors"
```

---

## Task 18: Disambiguation helper for `[major]` arg

**Files:**
- Create: `internal/commands/postgres/dispatch.go`
- Create: `internal/commands/postgres/dispatch_test.go`

Centralize the "single installed → infer; multiple → error" rule for `start`/`stop`/`restart`/`logs`/`status`.

- [ ] **Step 1: Write failing test**

Create `internal/commands/postgres/dispatch_test.go`:

```go
package postgres

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func install(t *testing.T, major string) {
	t.Helper()
	bin := config.PostgresBinDir(major)
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(bin, "postgres"), []byte{}, 0o755); err != nil {
		t.Fatal(err)
	}
}

func TestResolveMajor_NoArgs_OneInstalled(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	install(t, "17")
	got, err := resolveMajor(nil)
	if err != nil {
		t.Fatalf("resolveMajor: %v", err)
	}
	if got != "17" {
		t.Errorf("resolveMajor = %q, want 17", got)
	}
}

func TestResolveMajor_NoArgs_NoneInstalled(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if _, err := resolveMajor(nil); err == nil {
		t.Error("expected error when nothing installed")
	}
}

func TestResolveMajor_NoArgs_MultipleInstalled(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	install(t, "17")
	install(t, "18")
	if _, err := resolveMajor(nil); err == nil {
		t.Error("expected error when multiple installed and no arg given")
	}
}

func TestResolveMajor_ExplicitArg(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	install(t, "17")
	install(t, "18")
	got, err := resolveMajor([]string{"17"})
	if err != nil {
		t.Fatalf("resolveMajor: %v", err)
	}
	if got != "17" {
		t.Errorf("resolveMajor = %q, want 17", got)
	}
}

func TestResolveMajor_ExplicitNotInstalled(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	install(t, "17")
	if _, err := resolveMajor([]string{"18"}); err == nil {
		t.Error("expected error when explicit major not installed")
	}
}
```

- [ ] **Step 2: Run test, confirm failure**

```bash
go test ./internal/commands/postgres/ -v
```

- [ ] **Step 3: Implement `internal/commands/postgres/dispatch.go`**

```go
// Package postgres holds cobra commands for the postgres:* / pg:* group.
package postgres

import (
	"fmt"
	"strings"

	pg "github.com/prvious/pv/internal/postgres"
)

// resolveMajor implements the disambiguation rule for commands taking an
// optional [major] argument:
//   - explicit arg: must be installed, returned verbatim.
//   - no arg + exactly one installed major: returns that major.
//   - no arg + zero installed: error suggesting `pv postgres:install`.
//   - no arg + multiple installed: error listing them.
func resolveMajor(args []string) (string, error) {
	installed, err := pg.InstalledMajors()
	if err != nil {
		return "", err
	}
	if len(args) > 0 {
		want := args[0]
		for _, m := range installed {
			if m == want {
				return want, nil
			}
		}
		return "", fmt.Errorf("postgres %s is not installed (run `pv postgres:install %s`)", want, want)
	}
	switch len(installed) {
	case 0:
		return "", fmt.Errorf("no postgres majors installed (run `pv postgres:install`)")
	case 1:
		return installed[0], nil
	default:
		return "", fmt.Errorf("multiple postgres majors installed (%s); specify which one", strings.Join(installed, ", "))
	}
}
```

- [ ] **Step 4: Run test, confirm pass**

```bash
go test ./internal/commands/postgres/ -v
```

- [ ] **Step 5: Commit**

```bash
gofmt -w internal/commands/postgres/
go vet ./...
go build ./...
git add internal/commands/postgres/dispatch.go internal/commands/postgres/dispatch_test.go
git commit -m "feat(commands/postgres): resolveMajor disambiguation helper"
```

---

## Task 19: `postgres:install` command

**Files:**
- Create: `internal/commands/postgres/install.go`
- Create: `internal/commands/postgres/download.go`

`install` orchestrates `download` + sets state; `download` is the lower rung that just fetches+extracts.

- [ ] **Step 1: Implement `download.go`**

```go
package postgres

import (
	"fmt"
	"net/http"
	"time"

	"github.com/prvious/pv/internal/binaries"
	pg "github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var downloadCmd = &cobra.Command{
	Use:     "postgres:download <major>",
	GroupID: "postgres",
	Short:   "Download a PostgreSQL tarball into private storage",
	Hidden:  true,
	Args:    cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		major := args[0]
		client := &http.Client{Timeout: 5 * time.Minute}
		return ui.StepProgress(fmt.Sprintf("Downloading PostgreSQL %s...", major),
			func(progress binaries.ProgressFunc) (string, error) {
				if err := pg.InstallProgress(client, major, progress); err != nil {
					return "", err
				}
				return fmt.Sprintf("Installed PostgreSQL %s", major), nil
			})
	},
}
```

(`ui.StepProgress` signature — verify from `internal/ui/`.)

- [ ] **Step 2: Verify `ui.StepProgress` signature**

```bash
grep -n "func StepProgress" internal/ui/*.go
```

If the signature differs (e.g., it takes a different callback shape), adjust the wrapper in Step 1 to match. Typical existing pattern:

```go
ui.StepProgress(label, func(set ui.SetProgress) (string, error) { ... })
```

If the existing rustfs install uses a particular pattern, mirror it (see `cmd/install.go` lines around `Pulling rustfs`).

- [ ] **Step 3: Implement `install.go`**

```go
package postgres

import (
	"fmt"

	pg "github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

const defaultMajor = "18"

var installCmd = &cobra.Command{
	Use:     "postgres:install [major]",
	GroupID: "postgres",
	Short:   "Install (or re-install) a PostgreSQL major",
	Long:    "Downloads PostgreSQL binaries, runs initdb, and registers the major as wanted-running. Default major: 18.",
	Example: `# Install PostgreSQL 18 (default)
pv postgres:install

# Install PostgreSQL 17 alongside 18
pv postgres:install 17`,
	Args: cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		major := defaultMajor
		if len(args) > 0 {
			major = args[0]
		}

		// If already on disk, just (re)mark wanted=running and signal daemon.
		if pg.IsInstalled(major) {
			if err := pg.SetWanted(major, "running"); err != nil {
				return err
			}
			ui.Success(fmt.Sprintf("PostgreSQL %s already installed — marked as wanted running.", major))
			return signalDaemon()
		}

		// Run the download/install pipeline.
		if err := downloadCmd.RunE(downloadCmd, []string{major}); err != nil {
			return err
		}
		ui.Success(fmt.Sprintf("PostgreSQL %s installed.", major))
		return signalDaemon()
	},
}

func signalDaemon() error {
	if !server.IsRunning() {
		ui.Subtle("daemon not running — postgres will start on next `pv start`")
		return nil
	}
	return server.SignalDaemon()
}
```

- [ ] **Step 4: Build to verify wiring (commands aren't registered yet so this only checks compile)**

```bash
go build ./...
```

- [ ] **Step 5: Commit**

```bash
gofmt -w internal/commands/postgres/
git add internal/commands/postgres/install.go internal/commands/postgres/download.go
git commit -m "feat(commands/postgres): install + download commands"
```

---

## Task 20: `postgres:uninstall` command

**Files:**
- Create: `internal/commands/postgres/uninstall.go`

- [ ] **Step 1: Implement**

```go
package postgres

import (
	"fmt"
	"time"

	"github.com/prvious/pv/internal/registry"
	pg "github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var uninstallForce bool

var uninstallCmd = &cobra.Command{
	Use:     "postgres:uninstall <major>",
	GroupID: "postgres",
	Short:   "Stop, remove data, and remove a PostgreSQL major",
	Long:    "Stops the supervised process, deletes the data directory, removes binaries and logs, and unbinds linked projects.",
	Example: `pv postgres:uninstall 17 --force`,
	Args:    cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		major := args[0]
		if !pg.IsInstalled(major) {
			ui.Subtle(fmt.Sprintf("PostgreSQL %s is not installed.", major))
			return nil
		}
		if !uninstallForce {
			ok, err := ui.Confirm(fmt.Sprintf("Remove PostgreSQL %s and DELETE its data directory? This cannot be undone.", major))
			if err != nil {
				return err
			}
			if !ok {
				return fmt.Errorf("aborted")
			}
		}

		// Mark stopped + signal daemon to bring the process down before we rm the dirs.
		if err := pg.SetWanted(major, "stopped"); err != nil {
			return err
		}
		if server.IsRunning() {
			_ = server.SignalDaemon()
			// Brief grace period so the supervisor's Stop completes before we
			// remove the binary tree and data dir.
			time.Sleep(2 * time.Second)
		}

		if err := pg.Uninstall(major); err != nil {
			return err
		}

		// Unbind from projects.
		reg, err := registry.Load()
		if err != nil {
			return err
		}
		reg.UnbindPostgresMajor(major)
		if err := reg.Save(); err != nil {
			return err
		}

		ui.Success(fmt.Sprintf("PostgreSQL %s uninstalled.", major))
		return nil
	},
}

func init() {
	uninstallCmd.Flags().BoolVar(&uninstallForce, "force", false, "Skip the confirmation prompt")
}
```

`registry.UnbindPostgresMajor` is added in Task 28; if that task hasn't run yet, this will fail to build — that's fine, we order Task 28 before this command compiles into a registered cobra root in Task 27.

- [ ] **Step 2: Verify `ui.Confirm` exists**

```bash
grep -n "func Confirm" internal/ui/*.go
```

If `ui.Confirm` doesn't exist, use the existing huh-based form pattern shown in `internal/commands/uninstall.go` or wherever a confirm prompt is currently used; mirror it.

- [ ] **Step 3: Commit (do not push yet — registry helper is added in Task 28)**

```bash
gofmt -w internal/commands/postgres/
git add internal/commands/postgres/uninstall.go
git commit -m "feat(commands/postgres): uninstall command"
```

---

## Task 21: `postgres:update` command

**Files:**
- Create: `internal/commands/postgres/update.go`

- [ ] **Step 1: Implement**

```go
package postgres

import (
	"fmt"
	"net/http"
	"time"

	"github.com/prvious/pv/internal/binaries"
	pg "github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var updateCmd = &cobra.Command{
	Use:     "postgres:update <major>",
	GroupID: "postgres",
	Short:   "Re-download a PostgreSQL major (data dir untouched)",
	Args:    cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		major := args[0]
		if !pg.IsInstalled(major) {
			return fmt.Errorf("postgres %s is not installed", major)
		}

		// Stop running process before swap.
		if err := pg.SetWanted(major, "stopped"); err != nil {
			return err
		}
		if server.IsRunning() {
			_ = server.SignalDaemon()
			time.Sleep(2 * time.Second)
		}

		client := &http.Client{Timeout: 5 * time.Minute}
		if err := ui.StepProgress(fmt.Sprintf("Updating PostgreSQL %s...", major),
			func(progress binaries.ProgressFunc) (string, error) {
				if err := pg.UpdateProgress(client, major, progress); err != nil {
					return "", err
				}
				return fmt.Sprintf("Updated PostgreSQL %s", major), nil
			}); err != nil {
			return err
		}

		ui.Success(fmt.Sprintf("PostgreSQL %s updated.", major))
		if server.IsRunning() {
			return server.SignalDaemon()
		}
		return nil
	},
}
```

- [ ] **Step 2: Commit**

```bash
gofmt -w internal/commands/postgres/
git add internal/commands/postgres/update.go
git commit -m "feat(commands/postgres): update command"
```

---

## Task 22: `postgres:start` / `:stop` / `:restart`

**Files:**
- Create: `internal/commands/postgres/start.go`
- Create: `internal/commands/postgres/stop.go`
- Create: `internal/commands/postgres/restart.go`

- [ ] **Step 1: Implement `start.go`**

```go
package postgres

import (
	"fmt"

	pg "github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var startCmd = &cobra.Command{
	Use:     "postgres:start [major]",
	GroupID: "postgres",
	Short:   "Mark a PostgreSQL major as wanted-running",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		major, err := resolveMajor(args)
		if err != nil {
			return err
		}
		if err := pg.SetWanted(major, "running"); err != nil {
			return err
		}
		ui.Success(fmt.Sprintf("PostgreSQL %s marked running.", major))
		if server.IsRunning() {
			return server.SignalDaemon()
		}
		ui.Subtle("daemon not running — will start on next `pv start`")
		return nil
	},
}
```

- [ ] **Step 2: Implement `stop.go`**

```go
package postgres

import (
	"fmt"

	pg "github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var stopCmd = &cobra.Command{
	Use:     "postgres:stop [major]",
	GroupID: "postgres",
	Short:   "Mark a PostgreSQL major as wanted-stopped",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		major, err := resolveMajor(args)
		if err != nil {
			return err
		}
		if err := pg.SetWanted(major, "stopped"); err != nil {
			return err
		}
		ui.Success(fmt.Sprintf("PostgreSQL %s marked stopped.", major))
		if server.IsRunning() {
			return server.SignalDaemon()
		}
		return nil
	},
}
```

- [ ] **Step 3: Implement `restart.go`**

```go
package postgres

import (
	"fmt"
	"time"

	pg "github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var restartCmd = &cobra.Command{
	Use:     "postgres:restart [major]",
	GroupID: "postgres",
	Short:   "Stop and start a PostgreSQL major",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		major, err := resolveMajor(args)
		if err != nil {
			return err
		}
		if err := pg.SetWanted(major, "stopped"); err != nil {
			return err
		}
		if server.IsRunning() {
			_ = server.SignalDaemon()
			time.Sleep(2 * time.Second)
		}
		if err := pg.SetWanted(major, "running"); err != nil {
			return err
		}
		if server.IsRunning() {
			if err := server.SignalDaemon(); err != nil {
				return err
			}
		}
		ui.Success(fmt.Sprintf("PostgreSQL %s restarted.", major))
		return nil
	},
}
```

- [ ] **Step 4: Commit**

```bash
gofmt -w internal/commands/postgres/
git add internal/commands/postgres/start.go internal/commands/postgres/stop.go internal/commands/postgres/restart.go
git commit -m "feat(commands/postgres): start/stop/restart commands"
```

---

## Task 23: `postgres:list` command

**Files:**
- Create: `internal/commands/postgres/list.go`

- [ ] **Step 1: Implement**

```go
package postgres

import (
	"fmt"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
	pg "github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var listCmd = &cobra.Command{
	Use:     "postgres:list",
	GroupID: "postgres",
	Short:   "List installed PostgreSQL majors",
	RunE: func(cmd *cobra.Command, args []string) error {
		installed, err := pg.InstalledMajors()
		if err != nil {
			return err
		}
		if len(installed) == 0 {
			ui.Subtle("No PostgreSQL majors installed.")
			return nil
		}

		st, _ := pg.LoadState()
		vs, _ := binaries.LoadVersions()
		reg, _ := registry.Load()
		status, _ := server.ReadDaemonStatus()

		rows := [][]string{}
		for _, major := range installed {
			port, _ := pg.PortFor(major)
			version := "?"
			if vs != nil {
				if v := vs.Get("postgres-" + major); v != "" {
					version = v
				}
			}
			runState := "stopped"
			supKey := "postgres-" + major
			if status != nil {
				if s, ok := status.Supervised[supKey]; ok && s.Running {
					runState = "running"
				}
			}
			wanted := st.Majors[major].Wanted
			projects := []string{}
			if reg != nil {
				for _, p := range reg.List() {
					if p.Services != nil && p.Services.Postgres == major {
						projects = append(projects, p.Name)
					}
				}
			}
			rows = append(rows, []string{
				major,
				version,
				fmt.Sprintf("%d", port),
				fmt.Sprintf("%s (%s)", runState, wanted),
				config.ServiceDataDir("postgres", major),
				fmt.Sprintf("%v", projects),
			})
		}

		ui.Table([]string{"MAJOR", "VERSION", "PORT", "STATUS", "DATA DIR", "PROJECTS"}, rows)
		return nil
	},
}
```

- [ ] **Step 2: Verify `ui.Table` signature**

```bash
grep -n "func Table" internal/ui/*.go
```

If `ui.Table` takes a different shape (e.g., takes a slice of `Row` structs), adapt.

- [ ] **Step 3: Commit**

```bash
gofmt -w internal/commands/postgres/
git add internal/commands/postgres/list.go
git commit -m "feat(commands/postgres): list command"
```

---

## Task 24: `postgres:logs` and `postgres:status`

**Files:**
- Create: `internal/commands/postgres/logs.go`
- Create: `internal/commands/postgres/status.go`

- [ ] **Step 1: Implement `logs.go`**

```go
package postgres

import (
	"io"
	"os"
	"os/exec"

	"github.com/prvious/pv/internal/config"
	"github.com/spf13/cobra"
)

var logsFollow bool

var logsCmd = &cobra.Command{
	Use:     "postgres:logs [major]",
	GroupID: "postgres",
	Short:   "Tail a PostgreSQL major's log file",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		major, err := resolveMajor(args)
		if err != nil {
			return err
		}
		path := config.PostgresLogPath(major)
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

- [ ] **Step 2: Implement `status.go`**

```go
package postgres

import (
	"fmt"
	"os"

	pg "github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var statusCmd = &cobra.Command{
	Use:     "postgres:status [major]",
	GroupID: "postgres",
	Short:   "Show PostgreSQL major status",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		var majors []string
		if len(args) > 0 {
			m, err := resolveMajor(args)
			if err != nil {
				return err
			}
			majors = []string{m}
		} else {
			ms, err := pg.InstalledMajors()
			if err != nil {
				return err
			}
			majors = ms
		}
		if len(majors) == 0 {
			ui.Subtle("No PostgreSQL majors installed.")
			return nil
		}

		status, _ := server.ReadDaemonStatus()
		for _, major := range majors {
			port, _ := pg.PortFor(major)
			supKey := "postgres-" + major
			if status != nil {
				if s, ok := status.Supervised[supKey]; ok && s.Running {
					fmt.Fprintf(os.Stderr, "postgres %s: running on :%d (pid %d)\n", major, port, s.PID)
					continue
				}
			}
			fmt.Fprintf(os.Stderr, "postgres %s: stopped\n", major)
		}
		return nil
	},
}
```

- [ ] **Step 3: Commit**

```bash
gofmt -w internal/commands/postgres/
git add internal/commands/postgres/logs.go internal/commands/postgres/status.go
git commit -m "feat(commands/postgres): logs + status commands"
```

---

## Task 25: `register.go` + `cmd/postgres.go` bridge — wire `postgres:*` and `pg:*`

**Files:**
- Create: `internal/commands/postgres/register.go`
- Create: `cmd/postgres.go`
- Modify: `cmd/root.go` (add the postgres group)

Cobra's `Aliases` field renames a single command — it doesn't help with namespace aliasing across many commands. We instead clone each command into a `pg:*` variant and add both. Same RunE, separate Cobra entries.

- [ ] **Step 1: Implement `register.go`**

```go
package postgres

import (
	"strings"

	"github.com/spf13/cobra"
)

// Register wires every postgres:* command + a pg:* alias variant onto parent.
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
		downloadCmd,
	}
	for _, c := range cmds {
		parent.AddCommand(c)
		parent.AddCommand(aliasCommand(c, "postgres:", "pg:"))
	}
}

// aliasCommand returns a shallow clone of c whose Use, name, and visibility
// reflect a fromPrefix→toPrefix rewrite. The clone's RunE points at the
// original — single source of truth for the implementation.
func aliasCommand(c *cobra.Command, fromPrefix, toPrefix string) *cobra.Command {
	clone := *c
	clone.Use = strings.Replace(c.Use, fromPrefix, toPrefix, 1)
	// Mark the alias as hidden in --help to avoid duplicating every entry,
	// while still being a real, callable command.
	clone.Hidden = true
	clone.RunE = c.RunE
	return &clone
}

// Convenience wrappers for orchestrators (mirrors the mago/php/composer pattern).
func RunInstall(args []string) error {
	return installCmd.RunE(installCmd, args)
}
func RunUpdate(args []string) error {
	return updateCmd.RunE(updateCmd, args)
}
func RunUninstall(args []string) error {
	return uninstallCmd.RunE(uninstallCmd, args)
}
```

- [ ] **Step 2: Implement `cmd/postgres.go`**

```go
package cmd

import (
	postgres "github.com/prvious/pv/internal/commands/postgres"
	"github.com/spf13/cobra"
)

func init() {
	rootCmd.AddGroup(&cobra.Group{
		ID:    "postgres",
		Title: "PostgreSQL Management:",
	})
	postgres.Register(rootCmd)
}
```

- [ ] **Step 3: Build to verify wiring**

```bash
go build -o /tmp/pv .
/tmp/pv --help | grep -E "postgres:|pg:" | head
```

Expected: postgres group with `postgres:install`, `postgres:list`, etc. The `pg:*` aliases are hidden from help but should still be invokable:

```bash
/tmp/pv pg:list  # should not error with "unknown command"
```

(May exit non-zero with "no postgres majors installed" — that's expected and means routing works.)

- [ ] **Step 4: Commit**

```bash
gofmt -w internal/commands/postgres/ cmd/postgres.go
go vet ./...
git add internal/commands/postgres/register.go cmd/postgres.go
git commit -m "feat(cmd): wire postgres:* and pg:* command groups"
```

---

## Task 26: Add `registry.UnbindPostgresMajor` helper

**Files:**
- Modify: `internal/registry/registry.go`
- Modify: `internal/registry/registry_test.go`

- [ ] **Step 1: Write failing test**

Append to `internal/registry/registry_test.go`:

```go
func TestUnbindPostgresMajor(t *testing.T) {
	r := &Registry{
		Services: map[string]*ServiceInstance{},
		Projects: []Project{
			{Name: "a", Services: &ProjectServices{Postgres: "17"}},
			{Name: "b", Services: &ProjectServices{Postgres: "18"}},
			{Name: "c", Services: &ProjectServices{Postgres: "17"}},
			{Name: "d", Services: nil},
		},
	}
	r.UnbindPostgresMajor("17")
	cases := map[string]string{"a": "", "b": "18", "c": ""}
	for name, want := range cases {
		got := ""
		for _, p := range r.Projects {
			if p.Name == name && p.Services != nil {
				got = p.Services.Postgres
			}
		}
		if got != want {
			t.Errorf("project %s.Postgres = %q, want %q", name, got, want)
		}
	}
}
```

- [ ] **Step 2: Run, confirm failure**

```bash
go test ./internal/registry/ -v -run TestUnbindPostgresMajor
```

- [ ] **Step 3: Implement in `internal/registry/registry.go`**

Append:

```go
// UnbindPostgresMajor clears Services.Postgres on every project bound to
// the given major. Projects bound to other majors are unaffected.
// Tighter than UnbindService("postgres") — that would clear all bindings
// regardless of major.
func (r *Registry) UnbindPostgresMajor(major string) {
	for i := range r.Projects {
		if r.Projects[i].Services == nil {
			continue
		}
		if r.Projects[i].Services.Postgres == major {
			r.Projects[i].Services.Postgres = ""
		}
	}
}
```

- [ ] **Step 4: Run, confirm pass**

```bash
go test ./internal/registry/ -v -run TestUnbindPostgresMajor
```

- [ ] **Step 5: Commit**

```bash
gofmt -w internal/registry/
go vet ./...
go build ./...
git add internal/registry/registry.go internal/registry/registry_test.go
git commit -m "feat(registry): UnbindPostgresMajor (per-major unbind)"
```

---

## Task 27: `laravel.UpdateProjectEnvForPostgres`

**Files:**
- Modify: `internal/laravel/env.go`
- Modify: `internal/laravel/env_test.go`

Third env-update helper, parallel to `UpdateProjectEnvForService` (docker) and `UpdateProjectEnvForBinaryService` (singleton binary).

- [ ] **Step 1: Write failing test**

Append to `internal/laravel/env_test.go`:

```go
func TestUpdateProjectEnvForPostgres(t *testing.T) {
	tmp := t.TempDir()
	envPath := filepath.Join(tmp, ".env")
	if err := os.WriteFile(envPath, []byte("# initial\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	bound := &registry.ProjectServices{Postgres: "17"}
	if err := UpdateProjectEnvForPostgres(tmp, "my_app", "17", bound); err != nil {
		t.Fatalf("UpdateProjectEnvForPostgres: %v", err)
	}
	data, err := os.ReadFile(envPath)
	if err != nil {
		t.Fatal(err)
	}
	body := string(data)
	for _, want := range []string{"DB_CONNECTION=pgsql", "DB_PORT=54017", "DB_DATABASE=my_app"} {
		if !strings.Contains(body, want) {
			t.Errorf("missing %q in .env:\n%s", want, body)
		}
	}
}
```

- [ ] **Step 2: Run, confirm failure**

```bash
go test ./internal/laravel/ -v -run TestUpdateProjectEnvForPostgres
```

- [ ] **Step 3: Implement in `internal/laravel/env.go`**

Append:

```go
// UpdateProjectEnvForPostgres mirrors UpdateProjectEnvForService and
// UpdateProjectEnvForBinaryService for the postgres native-binary case.
// postgres has its own EnvVars signature (projectName, major) — it doesn't
// satisfy services.Service or services.BinaryService.
func UpdateProjectEnvForPostgres(projectPath, projectName, major string, bound *registry.ProjectServices) error {
	envPath := filepath.Join(projectPath, ".env")
	if _, err := os.Stat(envPath); os.IsNotExist(err) {
		return nil
	}
	pgVars, err := postgres.EnvVars(projectName, major)
	if err != nil {
		return err
	}
	smartVars := SmartEnvVars(bound)
	for k, v := range smartVars {
		pgVars[k] = v
	}
	backupPath := envPath + ".pv-backup"
	return services.MergeDotEnv(envPath, backupPath, pgVars)
}
```

Add the import:
```go
"github.com/prvious/pv/internal/postgres"
```

- [ ] **Step 4: Run, confirm pass**

```bash
go test ./internal/laravel/ -v -run TestUpdateProjectEnvForPostgres
```

- [ ] **Step 5: Commit**

```bash
gofmt -w internal/laravel/
go vet ./...
go build ./...
git add internal/laravel/env.go internal/laravel/env_test.go
git commit -m "feat(laravel): UpdateProjectEnvForPostgres helper"
```

---

## Task 28: Rewrite postgres detection in `automation/steps/detect_services.go`

**Files:**
- Modify: `internal/automation/steps/detect_services.go`
- Modify: `internal/automation/steps/detect_services_test.go` (if it exists)

Replace `findServiceByName(reg, "postgres")` with `postgres.InstalledMajors()` lookup + version selection.

- [ ] **Step 1: Read current implementation around the postgres branch**

```bash
grep -n -A 30 "case \"postgres\":" internal/automation/steps/detect_services.go
```

- [ ] **Step 2: Rewrite**

In `internal/automation/steps/detect_services.go`, locate the loop that probes for services. The current pattern probably calls `findServiceByName(reg, "postgres")` and then `bindProjectService(reg, ctx.ProjectName, "postgres", svcKey)` if non-empty. Replace the postgres-specific branch:

```go
// Postgres binding (native binary path; no longer routed via reg.Services).
if pgIsRequired { // whatever the existing match condition is
	majors, err := postgres.InstalledMajors()
	if err == nil && len(majors) > 0 {
		// Prefer the highest installed major.
		major := majors[len(majors)-1]
		bindProjectPostgres(ctx.Registry, ctx.ProjectName, major)
		bound++
	} else {
		ui.Subtle("postgres detected but not installed. Run: pv postgres:install")
	}
}
```

Add a helper `bindProjectPostgres` (sibling of `bindProjectService`):

```go
func bindProjectPostgres(reg *registry.Registry, projectName, major string) {
	for i := range reg.Projects {
		if reg.Projects[i].Name != projectName {
			continue
		}
		if reg.Projects[i].Services == nil {
			reg.Projects[i].Services = &registry.ProjectServices{}
		}
		reg.Projects[i].Services.Postgres = major
		return
	}
}
```

Add the import:
```go
"github.com/prvious/pv/internal/postgres"
```

- [ ] **Step 3: Update or remove `findServiceByName(reg, "postgres")` callers**

```bash
grep -n "postgres" internal/automation/steps/detect_services.go
```

Remove any postgres-keyed branch from `bindProjectService` if its switch still has one — postgres is now handled separately above.

- [ ] **Step 4: Update tests**

Open `internal/automation/steps/detect_services_test.go`. Migrate any test that pre-loads `reg.Services["postgres:17"]` to instead pre-stage `~/.pv/postgres/17/bin/postgres` (use `t.Setenv("HOME", t.TempDir())` + `os.MkdirAll(config.PostgresBinDir("17"), 0o755)` + write a stub `postgres` file). Test code lives in this file so it stays close to the change.

- [ ] **Step 5: Run tests**

```bash
go test ./internal/automation/... -v
```

- [ ] **Step 6: Commit**

```bash
gofmt -w internal/automation/steps/
go vet ./...
go build ./...
git add internal/automation/steps/detect_services.go internal/automation/steps/detect_services_test.go
git commit -m "feat(automation): postgres binding reads from internal/postgres"
```

---

## Task 29: Switch `CreateDatabaseStep` to bundled `psql` for postgres

**Files:**
- Modify: `internal/laravel/steps.go`

Existing `CreateDatabaseStep` for docker path runs `psql` inside the container via `engine.Exec(...)`. For native postgres there's no container; we shell out to the bundled `psql` directly via absolute path.

- [ ] **Step 1: Read the current step**

```bash
grep -n -A 50 "type CreateDatabaseStep" internal/laravel/steps.go
```

- [ ] **Step 2: Add a postgres branch in `CreateDatabaseStep.Run`**

The existing logic currently handles only the database-name resolution. The actual `CREATE DATABASE` is performed via the docker `services.Service.CreateDatabase` interface in the docker `service:add`/`hooks` flow — not in this step. Re-check: if `CreateDatabaseStep.Run` only registers the db name (no actual SQL), no edit needed here. The actual creation moves to a new helper called from `addBinary` or wherever the docker `service:add` runs `CreateDatabase`.

Since postgres is no longer added via `service:add`, we need a postgres-aware DB-create routine. Add it where it's needed — in `internal/postgres/`:

Create `internal/postgres/database.go`:

```go
package postgres

import (
	"fmt"
	"os/exec"
	"path/filepath"
	"strconv"

	"github.com/prvious/pv/internal/config"
)

// CreateDatabase creates dbName on the given postgres major using the
// bundled psql via absolute path. Idempotent: a SELECT-then-CREATE pattern
// avoids "database already exists" errors.
func CreateDatabase(major, dbName string) error {
	port, err := PortFor(major)
	if err != nil {
		return err
	}
	psql := filepath.Join(config.PostgresBinDir(major), "psql")
	args := []string{
		"-h", "127.0.0.1",
		"-p", strconv.Itoa(port),
		"-U", "postgres",
		"-tAc",
		fmt.Sprintf("SELECT 1 FROM pg_database WHERE datname = '%s'", dbName),
	}
	out, err := exec.Command(psql, args...).Output()
	if err != nil {
		return fmt.Errorf("psql probe: %w", err)
	}
	if string(out) == "1\n" {
		return nil
	}
	createArgs := []string{
		"-h", "127.0.0.1",
		"-p", strconv.Itoa(port),
		"-U", "postgres",
		"-c",
		fmt.Sprintf(`CREATE DATABASE "%s"`, dbName),
	}
	if _, err := exec.Command(psql, createArgs...).Output(); err != nil {
		return fmt.Errorf("psql create: %w", err)
	}
	return nil
}
```

Then call this from the appropriate location. Likely candidate: `laravel.CreateDatabaseStep.Run` for the postgres path. Check the existing implementation — it currently records the DB name in the registry but the actual CREATE happens elsewhere for docker. Mirror that placement, but switch to native: in `CreateDatabaseStep.Run`, after registry record-keeping, if `proj.Services.Postgres != ""`, call `postgres.CreateDatabase(major, dbName)`.

- [ ] **Step 3: Implement the call site**

In `internal/laravel/steps.go`, near the end of `CreateDatabaseStep.Run`:

```go
proj := ctx.Registry.Find(ctx.ProjectName)
if proj != nil && proj.Services != nil && proj.Services.Postgres != "" {
	if err := postgres.CreateDatabase(proj.Services.Postgres, dbName); err != nil {
		return "", fmt.Errorf("create postgres db: %w", err)
	}
}
```

Add the import:
```go
"github.com/prvious/pv/internal/postgres"
```

- [ ] **Step 4: Build to verify**

```bash
go build ./...
go test ./internal/laravel/ ./internal/postgres/ -v
```

- [ ] **Step 5: Commit**

```bash
gofmt -w internal/postgres/ internal/laravel/
git add internal/postgres/database.go internal/laravel/steps.go
git commit -m "feat(postgres): native CreateDatabase via bundled psql"
```

---

## Task 30: Wire `UpdateProjectEnvForPostgres` into the link pipeline

**Files:**
- Modify: `internal/laravel/steps.go`

`DetectServicesStep` (laravel) currently only writes smart vars (cache/session/queue). For postgres we need to write the connection vars. Today this happens via the docker hooks (`updateLinkedProjectsEnv` in `internal/commands/service/hooks.go`); for native postgres there's no `service:add` flow, so the env-write must happen during `pv link`.

- [ ] **Step 1: Add a step that writes postgres env vars on link**

In `internal/laravel/steps.go`, in the existing `DetectServicesStep.Run` (or a sibling step that runs after binding), append:

```go
proj := ctx.Registry.Find(ctx.ProjectName)
if proj != nil && proj.Services != nil && proj.Services.Postgres != "" {
	if err := UpdateProjectEnvForPostgres(ctx.ProjectPath, ctx.ProjectName, proj.Services.Postgres, proj.Services); err != nil {
		ui.Subtle(fmt.Sprintf("Could not write postgres env vars: %v", err))
	}
}
```

Make sure the `ui` import is present (it already is in this file based on earlier reads).

- [ ] **Step 2: Build and run tests**

```bash
go build ./...
go test ./internal/laravel/ -v
```

- [ ] **Step 3: Commit**

```bash
gofmt -w internal/laravel/
git add internal/laravel/steps.go
git commit -m "feat(laravel): write postgres env vars during link"
```

---

## Task 31: Delete docker `services.Postgres` + drop from registry

**Files:**
- Delete: `internal/services/postgres.go`
- Delete: `internal/services/postgres_test.go`
- Modify: `internal/services/service.go`

- [ ] **Step 1: Delete the files**

```bash
git rm internal/services/postgres.go internal/services/postgres_test.go
```

- [ ] **Step 2: Drop from `services.go`**

In `internal/services/service.go`, remove the `"postgres": &Postgres{}` entry from the `registry` map.

- [ ] **Step 3: Build — confirm what no longer compiles**

```bash
go build ./...
```

Expected: build errors in callers that still reference the deleted struct (mostly tests + setup wizard).

- [ ] **Step 4: Walk the build errors and fix them**

For each error:
- If it's a test using `&services.Postgres{}` as a fixture: replace with a docker-postgres-shaped fixture using a different service (mysql), OR delete the test if it was specifically about postgres.
- If it's the `internal/commands/setup/setup.go` services list: remove `"postgres"` from the multi-select.
- If it's `internal/commands/service/hooks_test.go:145`: the fixture `Services: &registry.ProjectServices{Postgres: "17"}` is fine (struct field still exists) — but if the test asserts against `services.Lookup("postgres")` returning a struct, it must be retargeted.

Run after each fix:
```bash
go build ./...
```

- [ ] **Step 5: Run all tests**

```bash
go test ./...
```

Fix anything still failing.

- [ ] **Step 6: Commit**

```bash
gofmt -w .
go vet ./...
git add -u
git commit -m "refactor: remove docker Postgres service"
```

---

## Task 32: Drop postgres from setup wizard's services list

**Files:**
- Modify: `internal/commands/setup/setup.go`

May already be covered by Task 31's walk-the-errors step; this is a sanity check.

- [ ] **Step 1: Locate**

```bash
grep -n "postgres" internal/commands/setup/*.go
```

If there's a `huh.Option` or a string slice that includes "postgres", remove it.

- [ ] **Step 2: Build + test**

```bash
go build ./...
go test ./internal/commands/setup/ -v
```

- [ ] **Step 3: Commit (only if a change was needed)**

```bash
gofmt -w internal/commands/setup/
git add internal/commands/setup/
git commit -m "refactor(setup): drop postgres from docker-services multi-select" || echo "no changes"
```

---

## Task 33: Drop postgres from `service:*` example text

**Files:**
- Modify: `internal/commands/service/add.go`
- Modify: `internal/commands/service/remove.go` (if it has a postgres example)

Cosmetic but visible. The error from `service:add postgres` should now read "unknown service postgres".

- [ ] **Step 1: Search**

```bash
grep -n "postgres" internal/commands/service/*.go | grep -v "_test.go"
```

- [ ] **Step 2: Edit each file to remove postgres-specific example text and Long descriptions**

In `internal/commands/service/add.go`, the `Long:` text says "Add a backing service (mail, mysql, postgres, redis, s3)" — remove "postgres,". Examples that say `pv service:add postgres` — remove.

Search-and-edit; do NOT edit other code references that still need to compile.

- [ ] **Step 3: Build + test**

```bash
go build ./...
go test ./internal/commands/service/ -v
```

- [ ] **Step 4: Commit**

```bash
gofmt -w internal/commands/service/
git add internal/commands/service/
git commit -m "refactor(service): drop postgres from example text"
```

---

## Task 34: Migrate any remaining test fixtures with `Services.Postgres`

**Files:**
- Various test files

`registry.ProjectServices.Postgres` field still exists, but its meaning is now "this project is bound to native postgres major X." Any test that used it as a docker-fixture pre-condition (i.e., expecting `reg.Services["postgres:17"]` to also exist) needs to either drop that expectation or pre-stage `~/.pv/postgres/17/bin/postgres`.

- [ ] **Step 1: Find offenders**

```bash
grep -rn "Postgres: \"" internal/ --include="*_test.go"
```

- [ ] **Step 2: For each match, decide**

- If the test is asserting docker behavior for postgres → either retarget to mysql (which still uses docker) or delete.
- If the test is about per-project state independent of the runtime mechanism → leave it alone.

- [ ] **Step 3: Run all tests**

```bash
go test ./...
```

- [ ] **Step 4: Commit**

```bash
gofmt -w .
git add -u
git commit -m "test: migrate remaining Services.Postgres fixtures"
```

---

## Task 35: Wire `pv install` / `pv update` / `pv uninstall` to call postgres orchestrator helpers (only if needed)

**Files:**
- Modify: `cmd/install.go` (potentially)
- Modify: `cmd/update.go` (potentially)
- Modify: `cmd/uninstall.go` (potentially)

The user explicitly said postgres install is opt-in (`pv postgres:install`), NOT auto-run on `pv install`. Confirm by reading these orchestrators and verifying they don't try to install postgres.

- [ ] **Step 1: Read the orchestrators**

```bash
grep -n -E "postgres|Postgres" cmd/install.go cmd/update.go cmd/uninstall.go
```

- [ ] **Step 2: For `cmd/update.go`**

If `pv update` should also re-pull every installed postgres major (to pick up rolling-artifact updates), add a loop:

```go
// Postgres updates (per installed major).
majors, _ := postgres.InstalledMajors()
for _, major := range majors {
	if err := postgresCmds.RunUpdate([]string{major}); err != nil {
		ui.Subtle(fmt.Sprintf("postgres %s update failed: %v", major, err))
	}
}
```

Use the alias `postgresCmds "github.com/prvious/pv/internal/commands/postgres"`.

This is opt-in per the spec — it only runs for already-installed majors. Recommend including it; matches the `php.RunUpdate()` call pattern.

- [ ] **Step 3: For `cmd/uninstall.go`**

If `pv uninstall` (full pv uninstall) should remove postgres state too — add a loop:

```go
majors, _ := postgres.InstalledMajors()
for _, major := range majors {
	_ = postgresCmds.RunUninstall([]string{major})
}
```

Decide based on existing pattern in `cmd/uninstall.go`.

- [ ] **Step 4: Build + test**

```bash
go build ./...
go test ./...
```

- [ ] **Step 5: Commit if any change was made**

```bash
gofmt -w cmd/
git add cmd/
git commit -m "feat(orchestrators): include postgres in pv update/uninstall" || echo "no changes"
```

---

## Task 36: E2E test — postgres lifecycle

**Files:**
- Create: `scripts/e2e/postgres-binary.sh`
- Modify: `.github/workflows/e2e.yml`

Mirror existing e2e scripts (`scripts/e2e/mail-binary.sh`, `scripts/e2e/s3-binary.sh`).

- [ ] **Step 1: Look at an existing script for the conventions**

```bash
cat scripts/e2e/mail-binary.sh 2>/dev/null | head -80
```

- [ ] **Step 2: Create `scripts/e2e/postgres-binary.sh`**

```bash
#!/usr/bin/env bash
# E2E: install postgres 17 + 18, verify both supervised, link a project,
# uninstall.
set -euo pipefail

source "$(dirname "$0")/helpers.sh"

PV=${PV:-pv}

echo "==> postgres:install 17"
sudo -E "$PV" postgres:install 17

echo "==> postgres:install 18"
sudo -E "$PV" postgres:install 18

echo "==> postgres:list shows both"
sudo -E "$PV" postgres:list | tee /tmp/pv-pg-list.txt
grep -q "^17 " /tmp/pv-pg-list.txt
grep -q "^18 " /tmp/pv-pg-list.txt

echo "==> wait for both ports to accept connections"
wait_for_tcp 127.0.0.1 54017 30
wait_for_tcp 127.0.0.1 54018 30

echo "==> SELECT version() via bundled psql for each"
~/.pv/postgres/17/bin/psql -h 127.0.0.1 -p 54017 -U postgres -tAc "SELECT version();"
~/.pv/postgres/18/bin/psql -h 127.0.0.1 -p 54018 -U postgres -tAc "SELECT version();"

echo "==> stop 17, verify only 18 still serving"
sudo -E "$PV" postgres:stop 17
sleep 2
if nc -z 127.0.0.1 54017 ; then echo "::error::17 still serving after stop"; exit 1; fi
nc -z 127.0.0.1 54018

echo "==> uninstall both"
sudo -E "$PV" postgres:uninstall 17 --force
sudo -E "$PV" postgres:uninstall 18 --force

echo "==> postgres:list reports nothing"
sudo -E "$PV" postgres:list | tee /tmp/pv-pg-list2.txt
if grep -q "^1[78] " /tmp/pv-pg-list2.txt; then
  echo "::error::expected empty list"; exit 1
fi

echo "==> OK"
```

- [ ] **Step 3: Add a `wait_for_tcp` helper to `scripts/e2e/helpers.sh` if it doesn't exist**

```bash
grep -n "wait_for_tcp" scripts/e2e/helpers.sh
```

If absent, append:

```bash
# wait_for_tcp HOST PORT [TIMEOUT_SEC]
# Returns 0 once HOST:PORT accepts a TCP connection, or fails after TIMEOUT.
wait_for_tcp() {
  local host="$1"; local port="$2"; local timeout="${3:-30}"
  local i=0
  while ! nc -z "$host" "$port" 2>/dev/null; do
    i=$((i+1))
    if [ "$i" -ge "$timeout" ]; then
      echo "wait_for_tcp: ${host}:${port} not accepting after ${timeout}s" >&2
      return 1
    fi
    sleep 1
  done
}
```

- [ ] **Step 4: Make the script executable + commit**

```bash
chmod +x scripts/e2e/postgres-binary.sh
git add scripts/e2e/postgres-binary.sh scripts/e2e/helpers.sh
git commit -m "test(e2e): postgres-binary lifecycle"
```

- [ ] **Step 5: Wire into `.github/workflows/e2e.yml`**

```bash
grep -n "mail-binary\|s3-binary" .github/workflows/e2e.yml
```

Mirror the pattern. Add a phase:

```yaml
      - name: postgres-binary lifecycle
        run: bash scripts/e2e/postgres-binary.sh
```

- [ ] **Step 6: Commit**

```bash
git add .github/workflows/e2e.yml
git commit -m "ci(e2e): wire postgres-binary phase"
```

---

## Task 37: End-to-end verification on a clean macOS arm64

Final sanity pass — run pv from this branch on a fresh-ish home directory.

- [ ] **Step 1: Build**

```bash
go build -o /tmp/pv-test .
```

- [ ] **Step 2: Sandbox into a temp HOME**

```bash
export HOME=$(mktemp -d)
mkdir -p "$HOME/.pv"
```

- [ ] **Step 3: Install both majors**

```bash
/tmp/pv-test postgres:install 17
/tmp/pv-test postgres:install 18
/tmp/pv-test postgres:list
```

- [ ] **Step 4: Manually start the daemon and check supervisor state**

```bash
/tmp/pv-test start &
sleep 3
cat "$HOME/.pv/daemon-status.json" | grep postgres
```

Expected: both `postgres-17` and `postgres-18` are running with non-zero PIDs.

- [ ] **Step 5: Connect to each major**

```bash
"$HOME/.pv/postgres/17/bin/psql" -h 127.0.0.1 -p 54017 -U postgres -c "SELECT version();"
"$HOME/.pv/postgres/18/bin/psql" -h 127.0.0.1 -p 54018 -U postgres -c "SELECT version();"
```

- [ ] **Step 6: Stop one, verify the other is unaffected**

```bash
/tmp/pv-test postgres:stop 17
sleep 2
"$HOME/.pv/postgres/17/bin/pg_isready" -h 127.0.0.1 -p 54017 || echo "17 stopped (expected)"
"$HOME/.pv/postgres/18/bin/pg_isready" -h 127.0.0.1 -p 54018  # should still report ready
```

- [ ] **Step 7: Daemon restart preserves state**

```bash
/tmp/pv-test stop
/tmp/pv-test start &
sleep 3
cat "$HOME/.pv/daemon-status.json"
```

Expected: PG 18 is running (it was wanted=running before stop/start). PG 17 is NOT running (it was wanted=stopped).

- [ ] **Step 8: Uninstall and verify cleanup**

```bash
/tmp/pv-test postgres:uninstall 17 --force
/tmp/pv-test postgres:uninstall 18 --force
ls "$HOME/.pv/postgres/" 2>/dev/null
ls "$HOME/.pv/services/postgres/" 2>/dev/null
cat "$HOME/.pv/data/state.json"
```

Expected: empty directories (or missing); state.json's postgres key has no majors.

- [ ] **Step 9: All clean — done**

No commit; this is a manual verification step.

---

## Self-Review

**Spec coverage (cross-checked against `docs/superpowers/specs/2026-05-05-postgres-native-binaries-design.md`):**

| Spec section | Plan task(s) |
|---|---|
| Locked decision: trust auth on 127.0.0.1 | Task 11 (initdb args), Task 10 (RewriteHBA) |
| Locked decision: major-only versions, default 18 | Task 19 (default major), Task 5 (PortFor enforces numeric major) |
| Locked decision: docker postgres removed entirely | Tasks 31–34 |
| Locked decision: explicit install model | Task 19 (no auto-install elsewhere) |
| Locked decision: `internal/postgres/` mirroring phpenv | Tasks 5–16 |
| Locked decision: `postgres:*` + `pg:*` aliases | Tasks 19–25 |
| Locked decision: utilities NOT on PATH | (no PATH exposure code added; verified by absence) |
| Architecture: package layout | Tasks 2–16, 18–25 |
| Reconciler: 2-source wanted set | Task 17 |
| Filesystem: `~/.pv/postgres/<major>/`, `~/.pv/data/state.json` | Tasks 2, 3, 7 |
| `postgresql.conf` overrides | Task 10 |
| `pg_hba.conf` rewrite | Task 10 |
| State file schema (per-service keyed) | Tasks 2, 7 |
| Install flow (download → initdb → conf → state) | Task 12 |
| Uninstall flow | Task 13, Task 20 |
| Update flow | Task 14, Task 21 |
| Project binding integration | Tasks 26–30 |
| 3-shape EnvVars (postgres free function) | Task 15 |
| Crash recovery via supervisor budget | (covered by existing supervisor code; no plan task needed) |
| E2E tests | Task 36 |
| Manual verification | Task 37 |

**Placeholder scan:** Two intentional "Verify-then-adapt" steps remain (Task 19 Step 2, Task 23 Step 2, Task 20 Step 2) — they explicitly tell the engineer to grep for the existing helper signature and adapt if it differs. These are not placeholders for implementation choices; they're guards against signature drift in the `internal/ui/` package across pv versions. Acceptable.

**Type/symbol consistency:**
- `MajorState{Wanted: ...}` — defined in Task 7, used in 7/8/12/13/14/19/20/21/22.
- `postgres.PortFor(major) (int, error)` — defined Task 5, used in 10, 12, 14, 15, 16, 23, 24, 29.
- `postgres.IsInstalled(major) bool` — defined Task 6, used in 12, 14, 19, 20, 21.
- `postgres.SetWanted(major, wanted) error` — defined Task 7, used in 12, 14, 19, 20, 21, 22.
- `postgres.WantedMajors() ([]string, error)` — defined Task 8, used in 17.
- `postgres.BuildSupervisorProcess(major) (supervisor.Process, error)` — defined Task 16, used in 17.
- `postgres.InstalledMajors() ([]string, error)` — defined Task 6, used in 17, 18, 23, 24, 28, 35.
- `postgres.RunInstall(args []string)` etc. — defined Task 25, used in 35.
- `registry.UnbindPostgresMajor(major)` — defined Task 26, used in Task 20.
- `binaries.PostgresURL(major)` — defined Task 4, used in 12 (via resolvePostgresURL), 14.
- `binaries.ExtractTarGzAll(archive, dest)` — defined Task 12 Step 0, used in 12, 14.

All consistent.

**Scope:** All tasks contribute to the same coherent change. The plan is large but cohesive — one new package, one new command group, one reconciler tweak, plus the docker removal and rewiring. Not splittable without breaking the build mid-way.
