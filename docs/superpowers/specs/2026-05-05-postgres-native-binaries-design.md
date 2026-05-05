# PostgreSQL native binaries (off Colima/Docker)

## Goal

Replace the docker-backed PostgreSQL service with native binaries managed
directly by pv. Multiple major versions coexist (PG 17 and PG 18 today),
each supervised by the existing pv daemon, each with its own data dir and
port. No Colima VM, no Docker for postgres.

The artifacts pipeline (`.github/workflows/build-artifacts.yml`) already
ships relocatable bundles at the rolling `artifacts` release:
`postgres-mac-arm64-17.tar.gz` and `postgres-mac-arm64-18.tar.gz`. This
spec covers the consumer side: download, install, initdb, supervisor
integration, and command surface.

## Scope (v1)

- Platform: macOS arm64 (Apple Silicon). Matches the artifacts pipeline.
- Versions: PG 17 and PG 18. Default major when omitted: `18`.
- Auth: `trust` on `127.0.0.1` (loopback only). `.env` keeps
  `DB_USERNAME=postgres` / `DB_PASSWORD=postgres` for parity with the
  MySQL flow; the password is effectively ignored by the server.
- Install model: explicit. Users opt in via `pv postgres:install <major>`.
  `pv link` does **not** auto-install postgres.
- Owned package: `internal/postgres/` (mirrors `internal/phpenv/`).
- Command surface: top-level `postgres:*` namespace with `pg:*` aliases.
  Postgres is no longer a `service:*` — it has its own command group.

## Non-goals (v1)

- Migrating data from the previous docker-backed postgres. The repo is
  early-stage; we assume no user has data to preserve.
- Exposing postgres client utilities (`psql`, `pg_dump`, `pg_restore`,
  `initdb`, etc.) on `PATH`. Multi-version routing is unsolved; defer.
  Internal absolute-path use of `psql` for `CreateDatabaseStep` is fine.
- Cross-major data migration / `pg_upgrade`. Per-major data dirs prevent
  accidental incompatibility; users wipe and re-init if they want a new
  major.
- Linux / x86_64 macOS. Add when the artifacts pipeline grows them.
- Auto-install on `pv link`. Considered and rejected — keeps the friction
  predictable; revisit later if the explicit step becomes annoying.
- A `pv.yml` `postgres:` field for declaring per-project version. Today
  binding flows through `ProjectServices.Postgres` and the auto-detect
  step. Add a `pv.yml` field later if needed.
- Setup-wizard integration. Drop postgres from the docker-services
  multi-select; do not yet add a "install PostgreSQL?" prompt.

## Locked decisions

| Topic | Decision | Rationale |
|---|---|---|
| Auth | trust on `127.0.0.1`/`::1`, trust local socket | Simplest dev UX; can't drift from a stored password. Loopback-only — no network exposure. |
| Version specifier | Major-only (`17`, `18`). Default `18`. | Matches what the artifacts pipeline ships; no ambiguity with `-alpine` / patch suffixes. |
| Docker postgres | Removed entirely | Early-stage, no migration path needed. Git history is the rollback. |
| Install gate | Explicit `pv postgres:install <major>` | Predictable; matches existing `service:add` model users already understand. |
| Owning package | `internal/postgres/` | Mirrors `internal/phpenv/` — proven precedent for versioned native binaries. |
| Command group | `postgres:*` with `pg:*` aliases | Top-level tool surface, like `php:*` / `mago:*` / `composer:*`. Not under `service:*`. |
| Utilities on PATH | Not in v1 | Multi-version routing problem unsolved. Internal absolute-path use only. |
| Approach | Approach 1 — `internal/postgres/` mirroring `phpenv` | See "Why this approach" below. |

## Why this approach

Three architectures considered:

1. **`internal/postgres/` mirroring `phpenv`** *(chosen)* — postgres lives
   in its own package, parallel to `phpenv`. The `services.BinaryService`
   interface stays single-version. Reconciler reads from a second wanted-
   set source (`postgres.WantedMajors()`) but the diff/start/stop loop is
   shared with rustfs/mailpit.
2. **Generalize `BinaryService` to multi-version** — every binary service
   becomes a list of versions. Forces the cost of multi-version on services
   that don't need it (rustfs/mailpit have no runtime version concept).
3. **`VersionedBinaryService` parallel interface in `services/`** — keeps
   rustfs/mailpit untouched but stuffs lifecycle (download, initdb,
   conf templating) into `internal/services/`, which today owns none of
   that — `services.BinaryService` already delegates install to
   `internal/binaries/`.

Approach 1 wins because postgres is genuinely different from rustfs/mailpit
(multi-version, init step, per-major data dirs) and the precedent already
exists: `phpenv` already solves the same shape for FrankenPHP.

## Architecture

### Package layout

```
internal/
├── postgres/                          NEW — version-aware lifecycle
│   ├── install.go                       download + extract + initdb + conf overrides
│   ├── uninstall.go                     stop + rm data dir + rm binaries
│   ├── update.go                        stop + redownload + restart
│   ├── process.go                       BuildSupervisorProcess(major) supervisor.Process
│   ├── installed.go                     scan ~/.pv/postgres/ for installed majors
│   ├── state.go                         read/write the postgres key in ~/.pv/data/state.json
│   ├── port.go                          PortFor(major) = 54000 + major
│   ├── envvars.go                       EnvVars(projectName, major) map[string]string
│   ├── conf.go                          postgresql.conf overrides + pg_hba.conf rewrite
│   └── version.go                       pg_config --version probe
│
├── binaries/
│   └── postgres.go                    NEW — DownloadURL for postgres tarball
│
├── commands/postgres/                 NEW — cobra commands, one file each
│   ├── register.go                      Register(parent) wires postgres:* and pg:*
│   ├── install.go
│   ├── uninstall.go
│   ├── update.go
│   ├── start.go
│   ├── stop.go
│   ├── restart.go
│   ├── list.go
│   ├── logs.go
│   └── status.go
│
├── server/manager.go                  TOUCHED — reconcileBinaryServices gains a
│                                       second wanted-set source
│
└── services/
    ├── postgres.go                    DELETED
    ├── postgres_test.go               DELETED
    └── service.go                     drop "postgres" from registry map

cmd/
└── postgres.go                        NEW — bridge file: Register(rootCmd) in init()
```

The `services.BinaryService` interface is **not** modified. Postgres does
not register in `binaryRegistry`.

### Command surface

| Command | Aliases | Behavior |
|---|---|---|
| `postgres:install [major]` | `pg:install` | Download tarball → extract → `initdb` (idempotent) → write conf overrides → mark wanted=running → signal daemon. Default major: `18`. Idempotent on already-installed. |
| `postgres:uninstall [major]` | `pg:uninstall` | Stop process → `rm -rf` data dir + binaries + log. Unbind from linked projects (clears `Services.Postgres` for matching majors). Confirm prompt unless `--force`. Major required (no default — too destructive). |
| `postgres:update [major]` | `pg:update` | Stop → redownload → re-emit conf overrides → restart. Data dir untouched. Major required. |
| `postgres:start [major]` | `pg:start` | Set `wanted=running` in `state.json` → signal daemon. Disambiguation: if exactly one major is installed, omitting `[major]` uses that one; otherwise error. |
| `postgres:stop [major]` | `pg:stop` | Set `wanted=stopped` → signal daemon. Same disambiguation rule. |
| `postgres:restart [major]` | `pg:restart` | Stop then start in a single supervisor pass. Same disambiguation rule. |
| `postgres:list` | `pg:list` | Print table: `MAJOR | VERSION | PORT | STATUS | DATA DIR | LINKED PROJECTS`. |
| `postgres:logs [major] [-f]` | `pg:logs` | Tail `~/.pv/logs/postgres-<major>.log`. `-f` follows. Same disambiguation rule. |
| `postgres:status [major]` | `pg:status` | One-liner per major (running on port + pid, or stopped). All majors if `[major]` omitted. |

Hidden command (debug only, not surfaced in `--help`):

- `postgres:download <major>` — just the download step. Mirrors the
  `:download` rung in CLAUDE.md's tool-command pattern.

### Filesystem layout

```
~/.pv/
├── postgres/
│   ├── 17/                            (extracted tarball, all 37 binaries + dylibs + share)
│   │   ├── bin/
│   │   ├── lib/
│   │   ├── share/postgresql/
│   │   └── include/
│   └── 18/                            (same shape)
│
├── data/
│   ├── registry.json                  (existing — projects + service registry)
│   ├── versions.json                  (existing — binary version tracking)
│   └── state.json                     NEW — per-service runtime state, keyed by service name
│
├── data/
│   └── postgres/
│       ├── 17/                        (initdb output)
│       │   ├── PG_VERSION             (presence gates re-init)
│       │   ├── postgresql.conf        (vendored from .sample, then patched)
│       │   ├── pg_hba.conf            (rewritten — see below)
│       │   ├── postmaster.pid         (live only)
│       │   └── base/, global/, pg_wal/, …
│       └── 18/                        (same shape)
│
└── logs/
    ├── postgres-17.log                (supervisor-owned, append mode)
    └── postgres-18.log
```

`~/.pv/data/postgres/<major>/` is what `config.ServiceDataDir("postgres",
"<major>")` already returns — no new path helpers needed.

### `postgresql.conf` overrides

Appended to the bottom of the initdb-emitted `postgresql.conf` so they win
over defaults:

```
# Managed by pv — do not hand-edit.
listen_addresses = '127.0.0.1'
port = 54017                          # 54000 + major (54017 for PG17, 54018 for PG18)
unix_socket_directories = '/tmp/pv-postgres-17'
fsync = on                            # never disable, even in dev
synchronous_commit = on
logging_collector = off               # supervisor owns log redirection
log_destination = 'stderr'
shared_buffers = 128MB                # initdb default; explicit for clarity
max_connections = 100                 # initdb default; explicit for clarity
```

`pg_hba.conf` is rewritten (not appended) to:

```
local   all             all                                     trust
host    all             all             127.0.0.1/32            trust
host    all             all             ::1/128                 trust
```

These two files are also rewritten on `postgres:update` so the override
list stays in sync if pv changes its defaults across releases.

### Reconciler integration

Today's `reconcileBinaryServices` has one source of truth: `reg.Services`
filtered to `Kind == "binary"` and enabled. After this change it gains a
second source for postgres. The diff/start/stop loop is unified.

```go
func (m *ServerManager) reconcileBinaryServices(ctx context.Context) error {
    if m.supervisor == nil { return nil }
    reg, err := registry.Load()
    if err != nil { return fmt.Errorf("…: %w", err) }

    // wanted: supervisorKey -> buildable supervisor.Process
    wanted := map[string]supervisor.Process{}
    var startErrors []string

    // Source 1 — single-version binary services (rustfs, mailpit).
    for name, svc := range services.AllBinary() {
        entry := reg.Services[name]
        if entry == nil || entry.Kind != "binary" { continue }
        if entry.Enabled != nil && !*entry.Enabled { continue }
        proc, err := buildSupervisorProcess(svc)
        if err != nil { startErrors = append(startErrors, …); continue }
        wanted[svc.Binary().Name] = proc
    }

    // Source 2 — postgres, multi-version, filesystem + state.json.
    for _, major := range postgres.WantedMajors() {
        proc, err := postgres.BuildSupervisorProcess(major)
        if err != nil { startErrors = append(startErrors, …); continue }
        wanted["postgres-"+major] = proc
    }

    // Diff: stop unneeded.
    for _, supKey := range m.supervisor.SupervisedNames() {
        if _, ok := wanted[supKey]; !ok {
            m.supervisor.Stop(supKey, 10*time.Second)
        }
    }

    // Diff: start needed.
    for supKey, proc := range wanted {
        if m.supervisor.IsRunning(supKey) { continue }
        if err := m.supervisor.Start(ctx, proc); err != nil {
            startErrors = append(startErrors, fmt.Sprintf("%s: %v", supKey, err))
        }
    }

    if len(startErrors) > 0 {
        return fmt.Errorf("binary reconcile: %d failed: %s",
            len(startErrors), strings.Join(startErrors, "; "))
    }
    return nil
}
```

The function's name and overall shape don't change — only the wanted-set
computation grows a second segment.

### `BuildSupervisorProcess(major)`

```go
func BuildSupervisorProcess(major string) (supervisor.Process, error) {
    binDir   := filepath.Join(config.PvDir(), "postgres", major, "bin")
    dataDir  := config.ServiceDataDir("postgres", major)
    logFile  := filepath.Join(config.PvDir(), "logs", "postgres-"+major+".log")
    port     := PortFor(major)  // 54000 + major

    // Sanity: install must have run.
    if _, err := os.Stat(filepath.Join(dataDir, "PG_VERSION")); err != nil {
        return supervisor.Process{}, fmt.Errorf("postgres %s: data dir not initialized (run pv postgres:install %s)", major, major)
    }

    return supervisor.Process{
        Name:         "postgres-" + major,
        Binary:       filepath.Join(binDir, "postgres"),
        Args:         []string{"-D", dataDir},   // port comes from postgresql.conf
        LogFile:      logFile,
        Ready:        tcpReady(port),
        ReadyTimeout: 30 * time.Second,
    }, nil
}
```

Port is set in `postgresql.conf`, not on the command line — single source
of truth. The `tcpReady` helper builds an equivalent of the existing
`buildReadyFunc` for TCP probes.

### State file

`~/.pv/data/state.json` — sits alongside the existing `registry.json` and
`versions.json` under `DataDir()`. Single file, top-level keyed by service
name, so other services can grow their own runtime-state subschemas later
without a new file per service.

```json
{
  "postgres": {
    "majors": {
      "17": { "wanted": "running" },
      "18": { "wanted": "stopped" }
    }
  }
}
```

A new `internal/state/` package owns the file (load/save with mutex,
JSON-marshalled top-level `map[string]json.RawMessage` so each service
package owns its own schema and they don't accidentally collide). The
postgres package wraps it as `state.LoadPostgres()` / `state.SavePostgres()`
so the rest of the codebase doesn't have to know about the JSON layout.

State semantics:
- `postgres:install` and `postgres:start` set `wanted: running`.
- `postgres:stop` sets `wanted: stopped`.
- `postgres:uninstall` removes the entry entirely.

`postgres.WantedMajors()` reads this file, intersects with the set of
majors actually present on disk under `~/.pv/postgres/<n>/`, and returns
only majors that are both installed and `wanted: running`. Mismatches
(e.g. state says "running" but binaries deleted out of band) are silently
filtered, with a stderr warning the first time.

A missing `state.json`, or one missing the `postgres` key, is treated as
`{ majors: {} }` for postgres purposes. A corrupt file (invalid JSON) is
logged as a warning and treated as empty; recovery is `postgres:start <major>`.

## Data flows

### `pv postgres:install 17`

1. Resolve URL: `https://github.com/prvious/pv/releases/download/artifacts/postgres-mac-arm64-17.tar.gz`.
2. Download via `binaries.DownloadProgress` (progress bar via `ui.StepProgress`).
3. `binaries.ExtractTarGz` into `~/.pv/postgres/17/`. Tarball root is
   `bin/lib/share/include/`.
4. Defensive `chmod +x` over `bin/*` and `lib/*.dylib` (they should
   already be executable from the artifacts pipeline).
5. If `~/.pv/data/postgres/17/PG_VERSION` is absent: `os.MkdirAll` the
   data dir, then run
   `~/.pv/postgres/17/bin/initdb -D ~/.pv/data/postgres/17 -U postgres
   --auth=trust --encoding=UTF8 --locale=C`.
6. Append `postgresql.conf` overrides; rewrite `pg_hba.conf`.
7. Probe `~/.pv/postgres/17/bin/pg_config --version` → record `17.5` (or
   whatever) in `~/.pv/data/versions.json` under key `postgres-17`
   (existing `binaries.LoadVersions/Save` API).
8. Update `~/.pv/data/state.json` to set `postgres.majors["17"].wanted = "running"`.
9. Signal the daemon (`server.SignalDaemon()`); the reconciler picks up
   the new entry and starts the process.

If steps 1–4 already happened (binaries on disk), the install is a no-op
that just sets `wanted=running` and signals the daemon — same result as
`postgres:start 17`. Friendly message: "postgres 17 already installed —
ensuring it's running."

If `initdb` fails partway (disk full, permissions, etc.), the partial
data dir is removed before returning so the next attempt is clean.

### `pv postgres:uninstall 17 [--force]`

1. Confirm prompt unless `--force`. Mention that the data dir will be
   destroyed.
2. Set `state.json` `majors["17"].wanted = "stopped"` and signal the
   daemon to stop the process; wait up to 10s.
3. Remove `~/.pv/data/postgres/17/`.
4. Remove `~/.pv/postgres/17/`.
5. Remove `~/.pv/logs/postgres-17.log`.
6. Drop `majors["17"]` entry from `state.json`.
7. Drop `postgres-17` entry from `versions.json`.
8. `reg.UnbindPostgresMajor("17")` — clears `Services.Postgres` for any
   project bound to "17". Save registry.
9. `.env` files of those projects are **not** rewritten (matches today's
   `service:remove` behavior — pv doesn't edit project `.env` on remove).

### `pv postgres:update 17`

1. Set `state.json` `majors["17"].wanted = "stopped"` and signal the
   daemon to stop the process; wait up to 10s.
2. Redownload + extract over `~/.pv/postgres/17/`. Tar overwrite is
   per-file, but for safety the install routine first extracts to a
   `<dir>.new`, then `os.Rename`s atomically over `<dir>`.
3. Data dir untouched (PG_VERSION present → initdb skipped).
4. Re-emit `postgresql.conf` overrides and `pg_hba.conf` so the file
   reflects current pv defaults.
5. Update `versions.json`.
6. Set `state.json` `majors["17"].wanted = "running"` and signal daemon.

### `pv link <project>`

1. Existing pipeline runs.
2. `automation/steps/detect_services.go` detects pgsql usage (composer
   `require.php` heuristics).
3. Lookup for postgres becomes:
   - Read `postgres.InstalledMajors()`.
   - If exactly one is installed → bind to it.
   - If multiple are installed → bind to the highest (`18` over `17`).
   - If none → print: `"postgres detected but not installed. Run: pv postgres:install"`.
4. `laravel.DetectServicesStep` reads `proj.Services.Postgres = "<major>"`,
   calls `services.SmartEnvVars(proj.Services)`, which gains a small
   branch: if `svc.Postgres != ""`, call `postgres.EnvVars(projectName,
   major)` and merge.
5. `laravel.CreateDatabaseStep` runs the bundled `psql` via absolute path
   (`~/.pv/postgres/<major>/bin/psql`) — internal use only, not on PATH.
6. `laravel.RunMigrationsStep` runs `php artisan migrate` inside the
   project; the project's pgsql client connects to `127.0.0.1:54017`.

## Removal of docker postgres

**Files deleted:**
- `internal/services/postgres.go`
- `internal/services/postgres_test.go`

**Files touched:**
- `internal/services/service.go` — drop `"postgres": &Postgres{}` from
  the docker `registry` map.
- `internal/commands/service/{add,remove,start,stop,list,status,dispatch,
  hooks,env}.go` — remove postgres-specific references and example text.
  The functions continue to handle mysql/redis/mail/s3 unchanged.
- `internal/automation/steps/detect_services.go` — replace
  `findServiceByName(reg, "postgres")` with a call into
  `internal/postgres/`.
- `internal/laravel/services.go` (or wherever `SmartEnvVars` lives) —
  branch on `svc.Postgres != ""` and call `postgres.EnvVars(...)` rather
  than reaching into the deleted struct.
- `internal/commands/setup/` — drop postgres from the docker-services
  multi-select.
- Tests: `internal/services/lookup_test.go`, `internal/commands/service/
  hooks_test.go:145`, `internal/automation/steps/detect_services` tests —
  drop or migrate postgres-specific cases.

**Files NOT touched:**
- `internal/registry/registry.go` — `ProjectServices.Postgres` field,
  `ProjectsUsingService("postgres")`, `UnbindService("postgres")` all
  stay. They still work; the underlying meaning shifts (postgres no
  longer has a `reg.Services["postgres:18"]` entry) but the API doesn't
  change. Add a new `UnbindPostgresMajor("17")` helper — tighter than
  `UnbindService` because we want to keep "18" bindings when uninstalling
  "17".
- `internal/laravel/steps.go` — `DetectServicesStep.ShouldRun` /
  `CreateDatabaseStep.ShouldRun` keep using `Services.Postgres != ""` as
  the trigger. Same condition, same behavior.

## Three different `EnvVars` shapes

Note that the codebase ends up with three distinct shapes for the same
conceptual operation:

```go
// docker services.Service:
EnvVars(projectName string, port int) map[string]string

// singleton services.BinaryService:
EnvVars(projectName string) map[string]string

// internal/postgres free function:
postgres.EnvVars(projectName, major string) map[string]string
```

This is intentional — three different things have three different needs.
Postgres doesn't satisfy either existing interface; forcing it through one
of them would shoehorn the multi-version concern into a generic shape
that other services don't care about. `laravel.SmartEnvVars` becomes the
single place that knows about all three shapes and dispatches.

## Verification

**Unit tests:**
- `internal/postgres/`: install dry-run (mock the download), state.json
  read/write, port computation, conf override emission, idempotent reinit
  guard (presence of PG_VERSION).
- `internal/postgres/process_test.go`: `BuildSupervisorProcess` with
  initialized vs uninitialized data dirs.
- `internal/server/manager_test.go`: reconcile picks up postgres majors
  from `WantedMajors()`, stops removed ones.
- `internal/commands/postgres/*_test.go`: each command's argument parsing
  + dispatch.

**E2E tests** in `scripts/e2e/`:
- Phase: install both PG 17 and PG 18; assert both processes are
  supervised and listening on 54017 / 54018; create a database via
  bundled psql; tear down via `postgres:uninstall`. Mirrors the existing
  postgres-bundle test in `scripts/test-postgres-bundle.sh`.
- Phase: link a Laravel project that uses pgsql; assert auto-binding to
  the highest installed major; assert `.env` has the correct DB_PORT;
  assert `php artisan migrate` succeeds against `127.0.0.1:<port>`.

**Manual verification before merge:**
- `pv postgres:install 17` then `pv postgres:install 18` then
  `pv postgres:list` shows both.
- Stop the daemon, start it again — both come back up.
- Kill one postgres process out of band — supervisor restarts within the
  budget; the other major is unaffected.

## Failure modes

| Failure | Behavior |
|---|---|
| Tarball download fails | `postgres:install` errors before any on-disk changes; user retries. |
| `initdb` fails partway | Partial data dir removed; user retries cleanly. |
| Crash during runtime | Supervisor's existing 5-restarts-in-60s budget per supervisor key. PG 17 and PG 18 are independent. After budget exhausted, the major is dropped from supervision; `state.json` still says `wanted: running` and the next daemon restart retries. |
| `state.json` corrupt | Treated as empty; warning logged; user runs `postgres:start <major>` to recover. |
| Binaries deleted out of band but `state.json` says `running` | `WantedMajors()` filters to "installed AND wanted-running"; missing major silently dropped, warning on stderr. |
| User runs `postgres:install` while daemon is down | Install completes, `state.json` set, signal-daemon is a no-op; next `pv start` brings the process up. |
| Two pv binaries try to install the same major concurrently | Both attempt the download; second one's `os.Rename` over the staging dir wins. Initdb is guarded by PG_VERSION. Worst case: a half-extracted tarball gets stomped by the winning rename. Acceptable for v1; revisit with a file lock if it becomes a problem. |

## Migration / rollout

1. Land artifacts pipeline (already done — `build-artifacts.yml` ships
   `postgres-mac-arm64-{17,18}.tar.gz`).
2. Implement `internal/postgres/` package + `internal/binaries/postgres.go`.
3. Implement `internal/commands/postgres/` + `cmd/postgres.go` bridge.
4. Extend `reconcileBinaryServices` with the second wanted-set source.
5. Wire `laravel.SmartEnvVars` to call `postgres.EnvVars` for the
   postgres branch.
6. Wire `automation/steps/detect_services.go` to read installed majors
   from `internal/postgres/` instead of `reg.Services`.
7. Delete `internal/services/postgres.go` + `_test.go`; update remaining
   references.
8. Update e2e scripts.
9. Manual verification on macOS arm64.
10. Merge.

No coordination needed with end users — the repo is early-stage, no
data-migration step exists.
