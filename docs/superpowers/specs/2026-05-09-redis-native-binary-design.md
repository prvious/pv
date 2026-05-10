# Redis native binary (off Colima/Docker)

## Goal

Replace the docker-backed Redis service with a native binary managed
directly by pv. Single version (whatever the artifacts pipeline ships
from upstream `redis/redis`), supervised by the existing pv daemon,
with its own data directory at `~/.pv/data/redis/`. No Colima VM, no
Docker for redis.

The artifacts pipeline (`.github/workflows/build-artifacts.yml` `redis:`
job, landed in PR #78) builds redis from the `redis/redis` GitHub
release and ships `redis-mac-arm64.tar.gz` containing `redis-server`
and `redis-cli`. This spec covers the consumer side: download,
install, supervisor integration, and command surface.

This is the third pass of the same kind of work — postgres (PR #75)
and mysql (PR #80) preceded it. Architecture and reconciler integration
mirror those. Where this spec is shorter, it's because the precedent
exists.

## Scope (v1)

- Platform: macOS arm64 (Apple Silicon). Matches the artifacts pipeline.
- Version: single — the latest GA on the `redis/redis` releases page,
  resolved at artifact-build time. Tracked in `~/.pv/data/versions.json`
  under key `redis` for diagnostic purposes only; pv does not switch
  between versions.
- Auth: none (no `requirepass`); `bind 127.0.0.1` is the security
  boundary. Matches the existing Docker posture.
- Persistence: RDB snapshotting in `~/.pv/data/redis/dump.rdb` using
  redis-server's compiled-in default save policy
  (3600s/1key, 300s/100keys, 60s/10000keys). AOF off.
- Install model: explicit. Users opt in via `pv redis:install`. Setup
  wizard offers a single "Redis (native binary)" checkbox.
- `pv link` auto-binds redis to every Laravel project unconditionally
  when redis is installed (mirrors the mailpit/rustfs single-version
  precedent). Per-project bind tracked via the existing
  `ProjectServices.Redis bool` field (already present from the docker
  era; semantics unchanged — true means "this project receives redis
  envvars").
- Owned package: `internal/redis/` (mirrors `internal/postgres/` and
  `internal/mysql/` rather than the `internal/svchooks/` shared shape
  used by mailpit/rustfs). User has flagged that mailpit/rustfs will
  also migrate to per-package layouts in a later pass; redis blazes the
  single-version-in-its-own-package trail.
- Command surface: top-level `redis:*` namespace, no aliases.

## Non-goals (v1)

- Migrating data from the previous docker-backed redis. Repo is
  early-stage; we assume no user has data to preserve.
- Multi-version coexistence (e.g., redis 7 alongside redis 8). Not
  needed today; if added later, the package layout is parallel-evolution
  ready (single-record state in `state.json` would grow into a
  `versions` map, parallel to mysql's structure).
- Exposing `redis-cli` on `PATH`. Internal absolute-path use only —
  same deferment we made for postgres' `psql` and mysql's `mysql`.
- AOF (append-only file) persistence. RDB is sufficient for dev work.
- TLS, replication, sentinel, cluster modes. Single instance,
  single-node, loopback-only.
- Linux / x86_64 macOS. Add when the artifacts pipeline grows them.
- Restructuring `internal/svchooks/` or migrating mailpit/rustfs. Deferred
  to a follow-up PR.
- Removing the `service:*` command group. Per Q4/C, `service:*` stays
  unchanged — `service:add redis` will return its existing
  "unknown service" error after the docker registry empties.

## Locked decisions

| Topic | Decision | Rationale |
|---|---|---|
| Auth | No password, bind `127.0.0.1` only, `--protected-mode no` | Loopback-only is the security boundary; matches Docker posture; no .env drift from a stored password. |
| Version | Single, upstream-tracked at artifact-build time | YAGNI — multi-version is unmotivated for redis in dev. |
| Persistence | RDB via redis-server defaults; data dir at `~/.pv/data/redis/` | Survives `pv restart`; negligible disk cost on dev machines; matches user expectation. |
| Port | `6379` (default) | No collision risk (single-version), matches every Laravel default. |
| Bind | `127.0.0.1` | Spec posture — never expose redis to the network. |
| Docker redis | Removed entirely | Repo is early-stage; no migration. |
| Install gate | Explicit `pv redis:install` | Predictable; matches the mysql/postgres precedent. |
| Owning package | `internal/redis/` | Mirrors postgres/mysql. User has flagged future plan to migrate mailpit/rustfs to the same shape. |
| Command group | `redis:*` only, no aliases | Already-short canonical name. |
| `pv link` auto-bind | Unconditional on every Laravel project once redis is installed | Mirrors mailpit/rustfs single-version pattern; redis-as-cache/session is the path of least surprise in Laravel. |
| Per-project binding | `ProjectServices.Redis bool` (existing field, reused) | Already present from the docker era; bool fits single-version. If multi-version ever lands, change to string version. |
| Setup wizard | New "Redis (native binary)" checkbox alongside MySQL | One-click happy path for typical Laravel stack. |
| Service:* fate | Keep as-is, no redis redirect | Q4/C — minimal touch; existing "unknown service" error is fine. |
| Approach | Approach 1 — `internal/redis/` mirroring postgres/mysql | See "Why this approach" below. |

## Why this approach

Three architectures considered:

1. **`internal/redis/` mirroring `internal/postgres/` and
   `internal/mysql/`** *(chosen)* — redis lives in its own package.
   Reconciler reads from a fourth wanted-set source (`redis.IsWanted()`)
   alongside the existing postgres / mysql / `services.AllBinary()`
   sources. Diff/start/stop loop is unchanged.
2. **Reuse `services.BinaryService` + `internal/svchooks/` (the
   mailpit/rustfs shape)** — redis becomes another `BinaryService`.
   Smallest diff. But user has explicitly opted to phase out the
   `services` namespace for native binaries. Choosing this approach
   would mean writing code we'd have to throw away within a release or
   two.
3. **Promote redis into a shared `internal/binsvc/` abstraction**
   spanning postgres / mysql / redis (and eventually mailpit / rustfs)
   — single Lifecycle interface. Tempting, but premature. Postgres has
   `initdb`, mysql has `mysqld --initialize-insecure`, redis has no
   initialization step. The "shared shape" is shallow today; a
   refactor later (when all five services are in their own packages
   and we can see the actual variation) is cheaper than a wrong
   abstraction now.

Approach 1 wins because:
- It aligns with the user's stated direction (phase out `services` for
  native binaries).
- The cost of a fourth parallel package is small; the parallel
  evolution gives each binary room for its own quirks.
- `internal/redis/` is genuinely simpler than postgres/mysql (no
  initdb, single-version, no datadir migration concerns), so the
  package is light.

## Architecture

### Package layout

```
internal/
├── redis/                              NEW — single-version lifecycle
│   ├── install.go                       download + extract + chown + register
│   ├── uninstall.go                     stop + rm bin + rm log + (force) rm data
│   ├── update.go                        stop + redownload + restart
│   ├── process.go                       BuildSupervisorProcess() supervisor.Process
│   ├── installed.go                     IsInstalled() — checks for bin/redis-server
│   ├── state.go                         read/write the "redis" key in state.json
│   ├── wanted.go                        IsWanted() — installed AND wanted=running
│   ├── port.go                          Port() = 6379 (constant)
│   ├── envvars.go                       EnvVars(projectName) map[string]string
│   ├── version.go                       redis-server --version probe
│   ├── waitstopped.go                   poll port until refused (10s budget)
│   ├── privileges.go                    chownToTarget + dropSysProcAttr
│   ├── database.go                      BindLinkedProjects() retroactive bind
│   └── testdata/
│       └── fake-redis-server.go        Go main: --version + long-run modes
│
├── binaries/
│   └── redis.go                        NEW — Binary descriptor + URL builder
│
├── commands/redis/                     NEW — cobra wrappers, one per subcommand
│   ├── register.go                      Register(parent) wires redis:*
│   ├── install.go
│   ├── uninstall.go
│   ├── update.go
│   ├── start.go
│   ├── stop.go
│   ├── restart.go
│   ├── list.go                          (one-line summary, since single-version)
│   ├── status.go
│   ├── logs.go
│   └── download.go                      hidden — debug only
│
├── server/manager.go                   TOUCHED — reconcileBinaryServices gains
│                                        a fourth wanted-set source for redis
│
├── automation/steps/detect_services.go TOUCHED — sets Services.Redis=true on
│                                        every Laravel link when redis is installed
│
├── laravel/                            TOUCHED — env.go gains a redis branch
│   ├── env.go                          UpdateProjectEnvForRedis helper
│   └── steps.go                        DetectServicesStep redis branch
│
├── registry/registry.go                NOT TOUCHED — ProjectServices.Redis bool
│                                        already exists; UnbindService("redis")
│                                        already clears it. No new helper needed.
│
├── config/paths.go                     TOUCHED — RedisDir() / RedisDataDir() /
│                                        RedisLogPath() helpers; EnsureDirs
│                                        registers RedisDir()
│
└── services/
    ├── redis.go                        DELETED
    ├── redis_test.go                   DELETED
    └── service.go                      drop "redis" from registry map (now empty)

cmd/
├── redis.go                            NEW — bridge: redis.Register(rootCmd) in init()
├── install.go                          TOUCHED — wizard hook (redis checkbox)
├── update.go                           TOUCHED — pass over installed redis
└── uninstall.go                        TOUCHED — pass over installed redis
```

The `services.BinaryService` interface is **not** modified. Redis
does not register in `binaryRegistry` — it is reached through its own
package, parallel to postgres and mysql.

### Command surface

| Command | Behavior |
|---|---|
| `redis:install` | Download tarball → extract → chown → mark wanted=running → bind every linked Laravel project (auto-bind) → signal daemon. No version arg. Idempotent. |
| `redis:uninstall [--force]` | Stop process → `rm -rf` `~/.pv/redis/`. With `--force`, also removes `~/.pv/data/redis/`. Drop state entry, drop versions.json entry, `(r *Registry).UnbindRedis()` to clear all project bindings. Confirm prompt unless `--force`. |
| `redis:update` | Stop → redownload → atomic-replace binary tree → restart if was running. Data dir untouched. |
| `redis:start` | `SetWanted(WantedRunning)` → signal daemon. |
| `redis:stop` | `SetWanted(WantedStopped)` → signal daemon. |
| `redis:restart` | Stop then start in one supervisor pass. |
| `redis:list` | Single-line summary: `redis | <version> | 6379 | <status> | <data dir> | <linked-projects-count>`. (Mirrors the table shape of mysql:list / postgres:list, but with one row.) |
| `redis:status` | `running on port 6379 with pid <n>` or `stopped`. |
| `redis:logs [-f]` | Tail `~/.pv/logs/redis.log`. `-f` follows. |

Hidden command:
- `redis:download` — debug-only; same call as `redis:install`. Mirrors
  the postgres/mysql `:download` pattern.

### Filesystem layout

```
~/.pv/
├── redis/
│   ├── redis-server                    (bundled binary)
│   └── redis-cli
│
├── data/
│   ├── registry.json                   (existing)
│   ├── versions.json                   (existing — gains "redis" key)
│   ├── state.json                      (existing — gains "redis" slice)
│   └── redis/
│       └── dump.rdb                    (RDB snapshot, redis-server-managed)
│
└── logs/
    └── redis.log                       (supervisor-redirected stderr)
```

`config.RedisDir()`, `config.RedisDataDir()`, `config.RedisLogPath()`
helpers added to `internal/config/paths.go` next to the postgres/mysql
helpers.

### `redis-server` boot flags

Single source of truth — passed as command-line flags, no `redis.conf`.
Mirrors what we did for mysql.

```
redis-server
  --bind 127.0.0.1
  --port 6379
  --dir ~/.pv/data/redis/
  --dbfilename dump.rdb
  --pidfile /tmp/pv-redis.pid
  --daemonize no                      # supervised, run in foreground
  --protected-mode no                 # bind 127.0.0.1 already protects
  --appendonly no                     # RDB only
  # save policy: redis-server compiled-in defaults (3600 1 / 300 100 / 60 10000)
```

We deliberately do NOT pass `--logfile` — the supervisor opens
`RedisLogPath()` in the parent (running as root) and inherits the fd
to the child. Same trick used for mysql; sidesteps the dropped-privs
+ root-owned-log-dir problem we hit during mysql CI.

### Reconciler integration

`reconcileBinaryServices` already takes three wanted-set sources
(single-version services via `services.AllBinary()`, postgres, mysql).
This change adds a fourth for redis. The diff/start/stop loop is
unchanged.

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

The supervisor key is `redis` (no version suffix; single-version).
Existing diff/start/stop logic picks it up unchanged.

### `BuildSupervisorProcess()`

```go
func BuildSupervisorProcess() (supervisor.Process, error) {
    binPath := filepath.Join(config.RedisDir(), "redis-server")
    if _, err := os.Stat(binPath); err != nil {
        return supervisor.Process{}, fmt.Errorf("redis: not installed (run pv redis:install)")
    }
    return supervisor.Process{
        Name:         "redis",
        Binary:       binPath,
        Args:         buildRedisArgs(),
        LogFile:      config.RedisLogPath(),
        SysProcAttr:  dropSysProcAttr(),
        Ready:        tcpReady(6379),
        ReadyTimeout: 10 * time.Second,
    }, nil
}
```

### State file

Reuses `~/.pv/data/state.json`. Adds a `"redis"` slice alongside
`"postgres"` and `"mysql"`. Since redis is single-version, the slice
is flat:

```json
{
  "postgres": { "majors": { "17": { "wanted": "running" } } },
  "mysql":    { "versions": { "8.4": { "wanted": "running" } } },
  "redis":    { "wanted": "running" }
}
```

The redis package wraps `internal/state` as `redis.LoadState()` /
`redis.SaveState()` / `redis.SetWanted(wanted)`. Validates against
`WantedRunning` / `WantedStopped`. Mismatches (state says running but
binaries missing) are silently filtered with a one-time stderr
warning, same behavior as postgres/mysql.

State semantics:
- `redis:install` and `redis:start` set `wanted: running`.
- `redis:stop` sets `wanted: stopped`.
- `redis:uninstall` removes the entry.

`redis.IsWanted()` reads this file, intersects with `IsInstalled()`,
returns true iff both are satisfied. A missing or corrupt slice is
treated as `{}` — recovery is `redis:start` after binaries are
restored.

## Data flows

### `pv redis:install`

1. Resolve URL: `https://github.com/prvious/pv/releases/download/artifacts/redis-mac-arm64.tar.gz`. Test override via `PV_REDIS_URL_OVERRIDE`.
2. Download via `binaries.DownloadProgress` (progress bar via `ui.StepProgress`).
3. `binaries.ExtractTarGzAll` into a `<dir>.new` staging path, then atomic `os.Rename` over `~/.pv/redis/`.
4. `chownToTarget` so a sudo-launched pv hands ownership to SUDO_USER.
5. Ensure `~/.pv/data/redis/` exists, chown to SUDO_USER (so dropped redis-server can write `dump.rdb`).
6. Probe `redis-server --version` → record in `versions.json` under `redis`.
7. `SetWanted(WantedRunning)`.
8. `BindLinkedProjects()` — see Section "Auto-bind on install" below.
9. `signalDaemon()` — supervisor reconciles, brings redis-server up.

If already installed (binary tree exists), the install command
short-circuits to: `BindLinkedProjects()` + `SetWanted(WantedRunning)` +
`signalDaemon()`. Idempotent — re-runs are safe.

### `pv redis:uninstall [--force]`

1. Confirm prompt unless `--force`.
2. `SetWanted(WantedStopped)` and `signalDaemon()`; wait up to 10s for
   the TCP port to close. (Redis shutdown is sub-second under
   typical loads.)
3. Remove `~/.pv/redis/`.
4. Remove `~/.pv/logs/redis.log`.
5. If `force`: remove `~/.pv/data/redis/`.
6. Drop the `"redis"` slice from `state.json` (via `RemoveState()`).
7. Drop `redis` from `versions.json`.
8. `reg.UnbindService("redis")` — already exists; clears
   `Services.Redis` for every project. Save registry. (No new
   `UnbindRedis()` helper needed; postgres/mysql have version-specific
   helpers because they're multi-version, redis is single.)
9. `.env` files of unbound projects are **not** rewritten — same
   behavior as postgres/mysql uninstall. The stale `REDIS_HOST=...`
   lines are harmless until someone re-installs or re-edits.

### `pv redis:update`

1. `SetWanted(WantedStopped)` and `signalDaemon()`; wait up to 10s.
2. Redownload + extract over `~/.pv/redis/` via the staging-rename
   pattern.
3. Probe new version + record in `versions.json`.
4. `SetWanted(WantedRunning)` and `signalDaemon()`.

Data dir untouched. RDB files written by an old redis-server are
forward-compatible with newer versions on the same major.

### `pv link <project>` (auto-bind on link)

1. Existing pipeline runs.
2. `automation/steps/detect_services.go` gains a redis branch:
   - If `redis.IsInstalled()` returns true and the project is
     Laravel-shaped, bind unconditionally:
     `proj.Services.Redis = true`.
   - If redis is not installed, no bind happens. (No "redis detected
     but not installed" prompt — redis is a transparent dependency
     for Laravel apps; we don't want to nag.)
3. `laravel.SmartEnvVars` gains a redis branch: if `svc.Redis == true`,
   merge `redis.EnvVars(projectName)` into the env-var map.
4. `laravel.UpdateProjectEnvForRedis(projectPath, projectName)`
   writes `REDIS_HOST/PORT/PASSWORD` to `.env`. Preserves comments;
   only writes if values changed (mirrors the postgres/mysql helpers).

### Auto-bind on install

`redis.BindLinkedProjects()` runs at the end of `redis:install`
(both first-install and idempotent re-install paths). It mirrors
`bindLinkedProjectsToMysql` from PR #80:

1. Walk `registry.Projects`.
2. For each Laravel-shaped project (`Type` is `laravel` or
   `laravel-octane`), set `Services.Redis = true` and call
   `laravel.UpdateProjectEnvForRedis(p.Path, p.Name)`.
3. Save the registry once at the end if anything changed.

This is the retroactive-bind path for projects linked **before**
redis was installed. The forward path (linking after install) is
covered by Section "pv link <project>" above.

### `EnvVars(projectName)`

```go
func EnvVars(projectName string) map[string]string {
    _ = projectName // unused — redis uses no project-scoped value
    return map[string]string{
        "REDIS_HOST":     "127.0.0.1",
        "REDIS_PORT":     "6379",
        "REDIS_PASSWORD": "null",
    }
}
```

`null` is the Laravel convention for "no password" — `config/database.php`
treats the literal string as nil when parsing the `.env`. Matches the
existing Docker `services.Redis.EnvVars` shape exactly so no `.env`
churn for projects linked under the old service.

`projectName` is accepted but unused — keeps the signature parallel
with `mysql.EnvVars` / `postgres.EnvVars` for the dispatcher in
`laravel.SmartEnvVars`.

## Removal of docker redis

**Files deleted:**
- `internal/services/redis.go`
- `internal/services/redis_test.go`

**Files touched:**
- `internal/services/service.go` — drop `"redis": &Redis{}` from the
  registry map. The map is now empty; `services.Lookup`,
  `services.AllDocker`, etc. continue to compile and operate over an
  empty set.
- Any tests that depended on a non-empty docker registry (e.g.,
  `internal/services/lookup_test.go`, `internal/services/service_test.go`)
  — adjust assertions to expect an empty map.
- `internal/commands/service/` — already returns redirect errors for
  s3/mail and "unknown service" for everything else. No change needed
  for redis (per Q4/C, no redis redirect).
- `cmd/install.go`, `cmd/setup.go` — drop any leftover docker-redis
  references in flags / wizard text. Add the new
  "Redis (native binary)" wizard checkbox.
- `cmd/install.go` `--with` flag parser — drop `service[redis:...]`
  syntax (mirrors what we did for mysql).

**Files NOT touched in a meaningful way:**
- `internal/registry/registry.go` — `ProjectServices.Redis bool`
  already exists from the docker era (JSON-tag `"redis,omitempty"`).
  Semantics carry over: `true` means "this project receives redis
  envvars". `UnbindService("redis")` already clears it. No new helper.
- `internal/laravel/steps.go` — the `DetectServicesStep` already calls
  `SmartEnvVars`; we add a redis branch in that dispatcher and a redis
  branch in the per-project env writer.

**Abandoned data**: any user who previously ran `service:add redis`
has data in `~/.pv/data/redis/` (the Docker volume mount). The new
binary uses the **same path** (`~/.pv/data/redis/dump.rdb`), so on
first `redis:install` redis-server will see no `dump.rdb` (Docker had
no AOF and used a different on-disk layout) and start clean. The old
Docker volume contents (if any) are left in place — pv does not
auto-wipe.

## Verification

**Unit tests** (mirroring `internal/mysql/` and `internal/postgres/`):

| Test file | Coverage |
|---|---|
| `port_test.go` | `Port()` returns 6379 |
| `installed_test.go` | `IsInstalled()` checks `bin/redis-server` |
| `state_test.go` | wanted=running/stopped roundtrip; invalid wanted rejected; RemoveState |
| `wanted_test.go` | IsWanted() filters by status correctly; missing-binary case warns once |
| `version_test.go` | ProbeVersion parses `redis-server --version` output |
| `envvars_test.go` | REDIS_HOST/PORT/PASSWORD shape |
| `install_test.go` | end-to-end install path against fake-redis-server stub |
| `uninstall_test.go` | force vs non-force; datadir kept by default |
| `update_test.go` | re-download replaces binary tree; datadir untouched; running state restored |
| `process_test.go` | BuildSupervisorProcess flag composition; SysProcAttr drop; LogFile path |
| `database_test.go` | BindLinkedProjects walks Laravel projects, sets Services.Redis=true, writes envvars |

Test fakes under `internal/redis/testdata/` are Go `main` programs
compiled at test time — never python/ruby/node, per CLAUDE.md.

`internal/server/manager_test.go` extended: reconcile picks up redis
from `IsWanted()`, stops it on transition to wanted=stopped — same
shape as the postgres/mysql assertions in PR #75 and PR #80.

`internal/commands/redis/*_test.go`: each command's argument parsing
and dispatch.

**E2E phase** at `scripts/e2e/redis-binary.sh`, wired into
`.github/workflows/e2e.yml` after the mysql phase. Mirrors
`scripts/e2e/postgres-binary.sh` and `scripts/e2e/mysql-binary.sh`:

1. `pv redis:install` → succeeds.
2. Port 6379 reachable (`wait_for_tcp 127.0.0.1 6379 30`).
3. `~/.pv/daemon-status.json` lists `"redis"`.
4. Connect via bundled `redis-cli`: `PING` → `PONG`; `SET k v` /
   `GET k` roundtrip returns `v`.
5. Pre-link a Laravel project before running `pv redis:install`,
   then assert that `.env` has `REDIS_HOST=127.0.0.1` after install
   (auto-bind retroactive).
6. `pv redis:uninstall --force` cleans tree + datadir + state.
   Re-running `pv redis:list` shows nothing supervised.

`scripts/e2e/diagnostics.sh` extended to dump `~/.pv/logs/redis.log`,
the `"redis"` slice of `state.json`, and `~/.pv/data/redis/` contents
(parallel to the mysql / postgres diagnostics blocks added in PR #80).

**Manual verification before merge:**
- `pv redis:install` → redis listed by `pv redis:list` and supervised.
- `pv link` on a Laravel project → `.env` gets `REDIS_HOST/PORT/PASSWORD`.
- Stop the daemon, restart it — redis comes back up.
- `pv redis:uninstall --force` → clean removal.

`go build ./...`, `go vet ./...`, `gofmt -l .`, `go test ./...` all
clean.

## Failure modes

| Failure | Behavior |
|---|---|
| Tarball download fails | `redis:install` errors before any on-disk changes; user retries. |
| Crash during runtime | Supervisor's existing 5-restarts-in-60s budget. After exhaustion, redis is dropped from supervision; `state.json` still says `wanted: running`; next daemon restart retries. |
| `state.json` `redis` slice corrupt | Treated as empty; warning logged once; user runs `redis:start` to recover. |
| Binary deleted out of band but `state.json` says `running` | `IsWanted()` filters to "installed AND wanted-running"; missing case silently skipped, warning on stderr. |
| User runs `redis:install` while daemon is down | Install completes, `state.json` set, signal-daemon is a no-op; next `pv start` brings the process up. |
| Two pv binaries try to install concurrently | Both download; second one's `os.Rename` over the staging dir wins. Worst case: a half-extracted tarball gets stomped by the winning rename. Acceptable for v1; revisit with a file lock if it becomes a problem. |
| RDB file corrupt at startup | redis-server refuses to start; supervisor logs the error. User options: delete `~/.pv/data/redis/dump.rdb` and restart. We do NOT auto-delete — corruption typically signals something the user wants to know about. |
| Port 6379 already bound (e.g., user has Homebrew redis running) | redis-server fails to bind; supervisor logs the error and retries on next reconcile tick. The hint in `~/.pv/logs/redis.log` will point the user at the conflict. |

## Migration / rollout

1. Land artifacts pipeline (already done — `build-artifacts.yml`
   `redis:` job in PR #78). The first pipeline run after this spec
   merges will publish `redis-mac-arm64.tar.gz` to the rolling
   `artifacts` release.
2. Implement `internal/redis/` package + `internal/binaries/redis.go`.
3. Implement `internal/commands/redis/` + `cmd/redis.go` bridge.
4. Extend `reconcileBinaryServices` with the fourth wanted-set source.
5. Wire `laravel.SmartEnvVars` and `UpdateProjectEnvForRedis` for the
   redis branch.
6. Wire `automation/steps/detect_services.go` to set
   `Services.Redis=true` for Laravel projects when redis is installed.
7. (Skipped — `ProjectServices.Redis` and the unbind path already exist.)
8. Delete `internal/services/redis.go` + `_test.go`; update remaining
   service-registry tests for the now-empty docker map.
9. Setup wizard: add "Redis (native binary)" checkbox.
10. Update `cmd/install.go`, `cmd/update.go`, `cmd/uninstall.go` orchestrators with redis passes.
11. Add `scripts/e2e/redis-binary.sh` e2e phase.
12. Manual verification on macOS arm64.
13. Merge.

No coordination needed with end users — repo is early-stage. Abandoned
Docker-era redis data (if any) is left in place but unreferenced.
