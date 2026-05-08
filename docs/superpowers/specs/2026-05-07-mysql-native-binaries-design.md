# MySQL native binaries (off Colima/Docker)

## Goal

Replace the docker-backed MySQL service with native binaries managed
directly by pv. Multiple versions coexist (8.0, 8.4, 9.7), each
supervised by the existing pv daemon, each with its own data dir and
port. No Colima VM, no Docker for mysql.

The artifacts pipeline (`.github/workflows/build-artifacts.yml`) already
ships relocatable bundles at the rolling `artifacts` release:
`mysql-mac-arm64-8.0.tar.gz`, `mysql-mac-arm64-8.4.tar.gz`, and
`mysql-mac-arm64-9.7.tar.gz`. This spec covers the consumer side:
download, install, init, supervisor integration, and command surface.

This is the second pass of the same work that PR #75 did for PostgreSQL.
Architecture, package shape, and reconciler integration mirror
`docs/superpowers/specs/2026-05-05-postgres-native-binaries-design.md`
deliberately. Where this spec is shorter than that one, it's because
the precedent already exists.

## Scope (v1)

- Platform: macOS arm64 (Apple Silicon). Matches the artifacts pipeline.
- Versions: 8.0 (legacy LTS, EOL April 2026), 8.4 (current LTS, default),
  9.7 (Innovation). Default version when omitted: `8.4`.
- Auth: empty-password `root@localhost`, `bind-address=127.0.0.1`
  (loopback only). Mirrors postgres trust auth and matches the existing
  Docker `MYSQL_ALLOW_EMPTY_PASSWORD=yes` posture.
- Install model: explicit. Users opt in via `pv mysql:install <version>`.
  `pv link` does **not** auto-install mysql; it auto-binds only when
  a mysql is already installed and `DB_CONNECTION=mysql` is explicit
  in the project's `.env` / `config/database.php`.
- Setup wizard: a single "MySQL 8.4 (LTS)" checkbox. Other versions are
  reachable via explicit `pv mysql:install 8.0` / `pv mysql:install 9.7`.
- Owned package: `internal/mysql/` (mirrors `internal/postgres/`).
- Command surface: top-level `mysql:*` namespace, no aliases. The name
  is already short.

## Non-goals (v1)

- Migrating data from the previous docker-backed mysql. The repo is
  early-stage; we assume no user has data to preserve. Abandoned data
  under `~/.pv/data/mysql/<docker-version>/` is left in place — pv does
  not auto-wipe it.
- Exposing mysql client utilities (`mysql`, `mysqldump`, `mysqladmin`,
  `mysqlbinlog`, etc.) on `PATH`. Multi-version routing is unsolved;
  defer. Internal absolute-path use of `mysql` for `CreateDatabaseStep`
  is fine.
- Cross-version data migration / `mysql_upgrade`. Per-version data dirs
  prevent accidental incompatibility; users run `mysqldump | mysql`
  themselves if they want to move data between versions.
- Linux / x86_64 macOS. Add when the artifacts pipeline grows them.
- Auto-install on `pv link`. Considered and rejected — keeps the
  friction predictable; revisit later if the explicit step becomes
  annoying.
- A `pv.yml` `mysql:` field for declaring per-project version. Today
  binding flows through `ProjectServices.Mysql` and the auto-detect
  step. Add a `pv.yml` field later if needed.
- `mysqld_safe` / `mysqld_multi`. The pv daemon already supervises
  long-running processes; `mysqld_multi` is Perl, which we'd refuse
  on principle (CLAUDE.md: no scripting-language deps).
- X Protocol (mysqlx). Disabled at boot (`--mysqlx=OFF`) so the
  default 33060 port doesn't get squatted; nothing in the pv flow
  uses it.
- Random-password / locked-root auth. Loopback-only + empty password
  matches what users already have via the Docker service and
  matches the postgres trust model. No real threat is solved by
  generating a password we'd then write into `.env`.
- A short alias namespace like `my:*`. Considered and rejected:
  `mysql:` is already 5 chars, and `my:*` is generic enough to bite
  us if we ever want a `my:profile` or similar.

## Locked decisions

| Topic | Decision | Rationale |
|---|---|---|
| Auth | empty-password `root@localhost`, bind `127.0.0.1`, `--skip-name-resolve`, `--mysqlx=OFF` | Simplest dev UX; loopback-only; matches Docker posture. |
| Version specifier | `major.minor` strings (`"8.0"`, `"8.4"`, `"9.7"`) | Matches the artifact filenames; future-proof for adding 9.8/9.9 without 8.0-vs-8.4 fudging. |
| Default version | `8.4` (current LTS) | Stable, current LTS; what `docs/mysql.md` flags as the safe default. |
| Port scheme | `33000 + major*10 + minor` → 8.0=33080, 8.4=33084, 9.7=33097 | Memorable (port suffix = version digits), collision-free, far from default 3306. |
| Docker mysql | Removed entirely (file + tests + service registry entry + wizard wiring) | Early-stage, no migration path needed. Git history is the rollback. |
| Install gate | Explicit `pv mysql:install <version>` | Predictable; matches the postgres precedent. |
| Owning package | `internal/mysql/` | Mirrors `internal/postgres/` — proven precedent. |
| Command group | `mysql:*` only, no aliases | Short enough as-is; no `pg:*`-style shorthand needed. |
| `pv link` auto-bind | Only when `DB_CONNECTION=mysql` is **explicit** in `.env` / `config/database.php` | Safer than relying on Laravel's compiled default; doesn't step on undecided projects. |
| Per-project DB | Auto-create on link, named after the project (slugified directory) | Mirrors postgres `CreateDatabaseStep`. |
| Utilities on PATH | Not in v1 | Multi-version routing problem unsolved. Internal absolute-path use only. |
| Approach | Approach 1 — `internal/mysql/` mirroring `internal/postgres/` | See "Why this approach" below. |

## Why this approach

Three architectures considered:

1. **`internal/mysql/` mirroring `internal/postgres/`** *(chosen)* —
   mysql lives in its own package, parallel to postgres. The
   `services.BinaryService` interface stays single-version. Reconciler
   reads from a third wanted-set source (`mysql.WantedVersions()`) but
   the diff/start/stop loop is shared with rustfs/mailpit/postgres.
2. **Generalize `BinaryService` to multi-version** — every binary
   service becomes a list of versions. Forces the cost of multi-version
   on services that don't need it. Already rejected during the postgres
   pass; the same logic applies.
3. **Share a `multiVersionService` interface between postgres and
   mysql** — postgres and mysql have the same shape (versioned, init
   step, per-version datadir). Tempting, but premature. Postgres' init
   uses `initdb`; mysql's uses `mysqld --initialize-insecure`. Postgres
   has `postgresql.conf` + `pg_hba.conf`; mysql has neither (we
   pass everything as flags). The "shared shape" is shallow; an
   abstraction now would need to be re-thought as soon as a third
   versioned binary appears with a different init flow. Two
   independent packages is cheaper than one premature abstraction.

Approach 1 wins because it's a known-good shape and the cost of
parallel evolution between two packages (postgres and mysql) is small
compared to the cost of a wrong abstraction.

## Architecture

### Package layout

```
internal/
├── mysql/                              NEW — version-aware lifecycle
│   ├── install.go                       download + extract + initdb + chown + register
│   ├── uninstall.go                     stop + rm data dir (if --force) + rm binaries
│   ├── update.go                        stop + redownload + restart
│   ├── process.go                       BuildSupervisorProcess(version) supervisor.Process
│   ├── installed.go                     scan ~/.pv/mysql/ for installed versions
│   ├── state.go                         read/write the mysql key in ~/.pv/data/state.json
│   ├── port.go                          PortFor(version) = 33000 + major*10 + minor
│   ├── envvars.go                       EnvVars(projectName, version) map[string]string
│   ├── initdb.go                        RunInitdb(version) — mysqld --initialize-insecure
│   ├── database.go                      CREATE DATABASE IF NOT EXISTS <project> via bundled mysql client
│   ├── version.go                       mysqld --version probe
│   ├── waitstopped.go                   poll until process is fully stopped
│   ├── wanted.go                        WantedVersions() — installed AND wanted-running
│   └── testdata/
│       ├── fake-mysqld.go              Go main: pretends to be mysqld for tests
│       └── fake-mysql-version.go       Go main: emits a mysqld --version line
│
├── binaries/
│   └── mysql.go                        NEW — DownloadURL for mysql tarball
│
├── commands/mysql/                     NEW — cobra commands, one file each
│   ├── register.go                      Register(parent) wires mysql:*
│   ├── dispatch.go                      Run* helpers exported for orchestrators
│   ├── install.go
│   ├── uninstall.go
│   ├── update.go
│   ├── start.go
│   ├── stop.go
│   ├── restart.go
│   ├── list.go
│   ├── logs.go
│   ├── status.go
│   └── download.go                      hidden — debug only
│
├── server/manager.go                   TOUCHED — reconcileBinaryServices gains a
│                                        third wanted-set source (mysql)
│
├── automation/steps/detect_services.go TOUCHED — detect mysql, return version
│                                        from internal/mysql.InstalledVersions()
│
├── laravel/                            TOUCHED — env.go gains a mysql branch
│   ├── env.go                          inject DB_* envvars from mysql.EnvVars
│   └── steps.go                        DetectServicesStep + CreateDatabaseStep
│                                        gain a mysql branch
│
├── registry/registry.go                TOUCHED — ProjectServices.MySQL semantics
│                                        change (now a "8.0"/"8.4"/"9.7" version,
│                                        was a Docker tag); UnbindMysqlVersion
│                                        helper added
│
├── config/paths.go                     TOUCHED — MysqlDir() / MysqlVersionDir(v) /
│                                        MysqlBinDir(v) / MysqlLogPath(v)
│
└── services/
    ├── mysql.go                        DELETED
    ├── mysql_test.go                   DELETED
    └── service.go                      drop "mysql" from the docker registry map

cmd/
├── mysql.go                            NEW — bridge: mysql.Register(rootCmd) in init()
├── install.go                          TOUCHED — orchestrator hook (wizard-gated)
├── update.go                           TOUCHED — pass over installed mysql versions
└── uninstall.go                        TOUCHED — pass over installed mysql versions
```

The `services.BinaryService` interface is **not** modified. MySQL does
not register in `binaryRegistry`.

### Command surface

| Command | Behavior |
|---|---|
| `mysql:install [version]` | Download tarball → extract → `mysqld --initialize-insecure` (idempotent — skipped if datadir already populated) → mark wanted=running → signal daemon. Default version: `8.4`. Idempotent on already-installed: re-run is a no-op for files, refreshes wanted=running. |
| `mysql:uninstall <version> [--force]` | Stop process → `rm -rf` binary tree at `~/.pv/mysql/<version>/`. With `--force`, also removes the datadir at `~/.pv/data/mysql/<version>/`. Drop state entry. Unbind from linked projects (clears `Services.Mysql` for matching versions). Confirm prompt unless `--force`. Version required (no default — too destructive). |
| `mysql:update <version>` | Stop → redownload → atomic-replace the binary tree → restart if it was running. Datadir untouched. Version required. |
| `mysql:start <version>` | Set `wanted=running` in `state.json` → signal daemon. Disambiguation: if exactly one version is installed, omitting `[version]` uses that one; otherwise error with a list. |
| `mysql:stop <version>` | Set `wanted=stopped` → signal daemon. Same disambiguation rule. |
| `mysql:restart <version>` | Stop then start in a single supervisor pass. Same disambiguation rule. |
| `mysql:list` | Print table: `VERSION | PRECISE | PORT | STATUS | DATA DIR | LINKED PROJECTS`. |
| `mysql:logs [version] [-f]` | Tail `~/.pv/logs/mysql-<version>.log` (the supervisor-redirected `mysqld` stderr). `-f` follows. Same disambiguation rule. |
| `mysql:status [version]` | One-liner per version (running on port + pid, or stopped). All versions if `[version]` omitted. |

Hidden command (debug only, not surfaced in `--help`):

- `mysql:download <version>` — just the download step. Mirrors the
  `:download` rung in CLAUDE.md's tool-command pattern.

### Filesystem layout

```
~/.pv/
├── mysql/
│   ├── 8.0/                            (extracted tarball: bin/, lib/, share/, ...)
│   │   ├── bin/
│   │   ├── lib/
│   │   └── share/
│   ├── 8.4/                            (same shape)
│   └── 9.7/                            (same shape)
│
├── data/
│   ├── registry.json                   (existing — projects + service registry)
│   ├── versions.json                   (existing — binary version tracking)
│   └── state.json                      (existing from PR #75 — adds a "mysql" slice)
│
├── data/
│   └── mysql/
│       ├── 8.0/                        (mysqld --initialize-insecure output)
│       │   ├── auto.cnf                (presence gates re-init)
│       │   ├── mysql/                  (system schema)
│       │   ├── ibdata1, ib_buffer_pool, undo_001, undo_002
│       │   └── #innodb_redo/
│       ├── 8.4/                        (same shape)
│       └── 9.7/                        (same shape)
│
└── logs/
    ├── mysql-8.0.log                   (supervisor-owned, append mode)
    ├── mysql-8.4.log
    └── mysql-9.7.log
```

`config.MysqlDir()`, `config.MysqlVersionDir(version)`,
`config.MysqlBinDir(version)`, `config.MysqlDataDir(version)`,
`config.MysqlLogPath(version)` are added to `internal/config/paths.go`
mirroring the existing postgres helpers.

### `mysqld` boot flags

Single source of truth — passed as command-line flags, no `my.cnf`.
Mirrors postgres' approach of having pv own all configuration.

```
mysqld
  --datadir=~/.pv/data/mysql/<version>/
  --basedir=~/.pv/mysql/<version>/
  --port=<33000+major*10+minor>
  --bind-address=127.0.0.1
  --socket=/tmp/pv-mysql-<version>.sock
  --pid-file=/tmp/pv-mysql-<version>.pid
  --log-error=~/.pv/logs/mysql-<version>.log
  --mysqlx=OFF
  --skip-name-resolve
  --user=<current-user>           # only when running as root, dropped privs
```

`--mysqlx=OFF` ensures the X Protocol port (default 33060) isn't bound,
which would otherwise collide if a user installed two majors.
`--skip-name-resolve` means we never wait for reverse-DNS during
authentication on a loopback connection.

### Reconciler integration

PR #75 already extended `reconcileBinaryServices` to take a second
wanted-set source (postgres). This change adds a third for mysql.
The diff/start/stop loop is unchanged.

```go
// Source 3 — mysql, multi-version, filesystem + state.json.
for _, version := range mysql.WantedVersions() {
    proc, err := mysql.BuildSupervisorProcess(version)
    if err != nil { startErrors = append(startErrors, …); continue }
    wanted["mysql-"+version] = proc
}
```

The supervisor key is `mysql-<version>` (e.g. `mysql-8.4`). The
existing diff loop sees an unknown key and stops it; sees a missing
key and starts it. No new diff logic.

### `BuildSupervisorProcess(version)`

```go
func BuildSupervisorProcess(version string) (supervisor.Process, error) {
    binDir   := config.MysqlBinDir(version)
    dataDir  := config.MysqlDataDir(version)
    logFile  := config.MysqlLogPath(version)
    port, err := PortFor(version)
    if err != nil { return supervisor.Process{}, err }

    // Sanity: install must have run.
    if _, err := os.Stat(filepath.Join(dataDir, "auto.cnf")); err != nil {
        return supervisor.Process{},
            fmt.Errorf("mysql %s: data dir not initialized (run pv mysql:install %s)",
                version, version)
    }

    return supervisor.Process{
        Name:         "mysql-" + version,
        Binary:       filepath.Join(binDir, "mysqld"),
        Args:         buildMysqldArgs(version, dataDir, port),
        LogFile:      logFile,
        Ready:        tcpReady(port),
        ReadyTimeout: 30 * time.Second,
    }, nil
}
```

`auto.cnf` is mysqld's per-server-uuid file; its presence is the
durable marker that `--initialize-insecure` ran successfully.
`tcpReady` is the same helper postgres uses.

### State file

Reuses `~/.pv/data/state.json` from PR #75. Adds a `"mysql"` slice
alongside the existing `"postgres"` slice:

```json
{
  "postgres": {
    "majors": {
      "17": { "wanted": "running" },
      "18": { "wanted": "stopped" }
    }
  },
  "mysql": {
    "versions": {
      "8.4": { "wanted": "running" },
      "9.7": { "wanted": "stopped" }
    }
  }
}
```

The mysql package wraps it as `mysql.LoadState()` / `mysql.SaveState()`
so the rest of the codebase doesn't have to know about the JSON layout.
`mysql.SetWanted(version, wanted)` validates against the
`WantedRunning` / `WantedStopped` set, same as the postgres package.

State semantics:
- `mysql:install` and `mysql:start` set `wanted: running`.
- `mysql:stop` sets `wanted: stopped`.
- `mysql:uninstall` removes the entry entirely.

`mysql.WantedVersions()` reads this file, intersects with the set of
versions actually present on disk under `~/.pv/mysql/<version>/`, and
returns only versions that are both installed and `wanted: running`.
Mismatches are silently filtered, with a stderr warning the first
time. A missing or corrupt slice is treated as `{ versions: {} }` —
recovery is `mysql:start <version>`.

## Data flows

### `pv mysql:install 8.4`

1. Resolve URL: `https://github.com/prvious/pv/releases/download/artifacts/mysql-mac-arm64-8.4.tar.gz`. Test override via `PV_MYSQL_URL_OVERRIDE` env var.
2. Download via `binaries.DownloadProgress` (progress bar via `ui.StepProgress`).
3. `binaries.ExtractTarGzAll` into a `<dir>.new` staging path, then atomic `os.Rename` over `~/.pv/mysql/8.4/`.
4. `chownToTarget` so a sudo-launched pv hands ownership to the real user.
5. If `~/.pv/data/mysql/8.4/auto.cnf` is absent: `os.MkdirAll` the data dir, then run
   `~/.pv/mysql/8.4/bin/mysqld --initialize-insecure --datadir=~/.pv/data/mysql/8.4 --basedir=~/.pv/mysql/8.4 --user=<current-user>`.
6. Probe `~/.pv/mysql/8.4/bin/mysqld --version` → record `8.4.9` (or whatever) in `~/.pv/data/versions.json` under key `mysql-8.4`.
7. Update `~/.pv/data/state.json` to set `mysql.versions["8.4"].wanted = "running"`.
8. Signal the daemon (`server.SignalDaemon()`); the reconciler picks up the new entry and starts the process.

If steps 1–4 already happened (binaries on disk), the install is a no-op
that just sets `wanted=running` and signals the daemon — same result as
`mysql:start 8.4`. Friendly message: "mysql 8.4 already installed —
ensuring it's running."

If `--initialize-insecure` fails partway (disk full, permissions,
selinux on Linux later, etc.), the partial data dir is removed before
returning so the next attempt is clean.

### `pv mysql:uninstall 8.4 [--force]`

1. Confirm prompt unless `--force`. Mention that without `--force` the
   datadir is preserved; with `--force` it is destroyed.
2. Set `state.json` `mysql.versions["8.4"].wanted = "stopped"` and
   signal the daemon to stop the process; wait up to 30s (mysqld
   shutdown can take a moment to flush InnoDB).
3. Remove `~/.pv/mysql/8.4/`.
4. Remove `~/.pv/logs/mysql-8.4.log`.
5. If `--force`: remove `~/.pv/data/mysql/8.4/`.
6. Drop `versions["8.4"]` entry from `state.json`.
7. Drop `mysql-8.4` entry from `versions.json`.
8. `reg.UnbindMysqlVersion("8.4")` — clears `Services.MySQL` for any
   project bound to "8.4". Save registry.
9. `.env` files of those projects are **not** rewritten (matches
   today's `service:remove` behavior — pv doesn't edit project `.env`
   on remove).

### `pv mysql:update 8.4`

1. Set `state.json` `mysql.versions["8.4"].wanted = "stopped"` and
   signal the daemon to stop the process; wait up to 30s.
2. Redownload + extract over `~/.pv/mysql/8.4/` via the staging-rename
   pattern.
3. Datadir untouched (`auto.cnf` present → init skipped).
4. Update `versions.json`.
5. Set `state.json` `mysql.versions["8.4"].wanted = "running"` and
   signal daemon.

InnoDB redo-log compatibility caveat: a different patch version started
against the same datadir after a hard crash might refuse the redo log.
Documented as a known rough edge; out of scope for this spec.

### `pv link <project>`

1. Existing pipeline runs.
2. `automation/steps/detect_services.go` detects mysql usage:
   `DB_CONNECTION=mysql` is **explicit** in `.env` or
   `config/database.php`. Unset/missing does **not** trigger mysql
   binding (matches Q5/A1).
3. Lookup for mysql:
   - Read `mysql.InstalledVersions()`.
   - If exactly one is installed → bind to it.
   - If multiple are installed → bind to the highest (lex order:
     `9.7 > 8.4 > 8.0`).
   - If none → print: `"mysql detected but not installed. Run: pv mysql:install"`.
4. `laravel.DetectServicesStep` reads `proj.Services.MySQL = "<version>"`,
   calls `services.SmartEnvVars(proj.Services)`, which gains a small
   branch: if `svc.MySQL != ""`, call `mysql.EnvVars(projectName,
   version)` and merge.
5. `laravel.CreateDatabaseStep` runs the bundled `mysql` client via
   absolute path
   (`~/.pv/mysql/<version>/bin/mysql --socket=/tmp/pv-mysql-<version>.sock -u root`)
   — internal use only, not on PATH. Issues
   `CREATE DATABASE IF NOT EXISTS \`<project>\`;`. Idempotent — re-link
   is safe.
6. `laravel.RunMigrationsStep` runs `php artisan migrate` inside the
   project; the project's mysql client connects to `127.0.0.1:33084`
   (or whatever port).

If `mysqld` for the bound version is not running when link starts,
`CreateDatabaseStep` ensures it via the daemon (sets wanted=running,
signals, waits for ready). Mirrors postgres behavior.

### `EnvVars(projectName, version)`

```go
func EnvVars(projectName, version string) (map[string]string, error) {
    port, err := PortFor(version)
    if err != nil { return nil, err }
    return map[string]string{
        "DB_CONNECTION": "mysql",
        "DB_HOST":       "127.0.0.1",
        "DB_PORT":       strconv.Itoa(port),
        "DB_DATABASE":   projectName,
        "DB_USERNAME":   "root",
        "DB_PASSWORD":   "",
    }, nil
}
```

Same shape as postgres. `laravel.SmartEnvVars` already dispatches by
type; we add the mysql branch.

## Removal of docker mysql

**Files deleted:**
- `internal/services/mysql.go`
- `internal/services/mysql_test.go`

**Files touched:**
- `internal/services/service.go` — drop `"mysql": &MySQL{}` from the
  docker `registry` map.
- `internal/commands/service/{add,remove,start,stop,list,status,
  dispatch,hooks,env}.go` — remove mysql-specific references and
  example text. The functions continue to handle redis/mail/s3
  unchanged.
- `internal/automation/steps/detect_services.go` — replace
  `findServiceByName(reg, "mysql")` with a call into `internal/mysql/`.
- `internal/laravel/services.go` (or wherever `SmartEnvVars` lives) —
  branch on `svc.MySQL != ""` and call `mysql.EnvVars(...)`.
- `internal/commands/setup/` — replace the docker-mysql checkbox with
  a "MySQL 8.4 (LTS)" binary checkbox.
- Tests: `internal/services/lookup_test.go`, `internal/commands/service/
  hooks_test.go`, `internal/automation/steps/detect_services` tests —
  drop or migrate mysql-specific cases.

**Files NOT touched in a meaningful way:**
- `internal/registry/registry.go` — `ProjectServices.MySQL` field
  exists already (it's a string version specifier today, JSON tag
  `"mysql"`); we keep the field, just change what valid values mean
  (now `"8.0"` / `"8.4"` / `"9.7"`, was a Docker tag like `"8.4"` or
  `"latest"`). The JSON schema stays compatible. Add a new
  `UnbindMysqlVersion(version string)` method — tighter than
  `UnbindService("mysql")` because we want to keep "9.7" bindings
  when uninstalling "8.4".
- `internal/laravel/steps.go` — `DetectServicesStep.ShouldRun` /
  `CreateDatabaseStep.ShouldRun` keep using `Services.MySQL != ""`
  as the trigger.

**Abandoned datadirs**: data left behind by the old Docker service
under `~/.pv/data/mysql/<docker-version>/` (where `<docker-version>`
was things like `8.4` or `latest`) is **not** auto-removed. Two
reasons: (1) the new datadir layout is the same path shape, so
collisions are unlikely; (2) auto-deletion of a user's data is a
footgun even in an early-stage project.

## Three different `EnvVars` shapes

Same observation as the postgres spec — three shapes are intentional:

```go
// docker services.Service:
EnvVars(projectName string, port int) map[string]string

// singleton services.BinaryService:
EnvVars(projectName string) map[string]string

// internal/postgres / internal/mysql free function:
EnvVars(projectName, version string) (map[string]string, error)
```

`laravel.SmartEnvVars` is the single dispatcher. We add a fourth case
to its switch — the mysql branch — and that's the full integration
surface.

## Verification

**Unit tests** (mirroring `internal/postgres/`'s suite):

| Test file | Coverage |
|---|---|
| `port_test.go` | `33000 + major*10 + minor` formula across 8.0 / 8.4 / 9.7 + invalid versions rejected |
| `version_test.go` | `ProbeVersion` parsing of `mysqld --version` output |
| `state_test.go` | wanted=running/stopped roundtrip, invalid wanted rejected, RemoveVersion |
| `wanted_test.go` | LoadState → WantedVersions filters by status correctly |
| `installed_test.go` | IsInstalled checks for `bin/mysqld` |
| `envvars_test.go` | DB_HOST/PORT/DATABASE/USERNAME/PASSWORD shape per-version |
| `install_test.go` | end-to-end install path against fake-mysqld stub |
| `uninstall_test.go` | force vs non-force, datadir kept by default |
| `update_test.go` | re-download replaces binary tree, datadir untouched, running state preserved |
| `process_test.go` | BuildSupervisorProcess flag composition |
| `initdb_test.go` | RunInitdb invokes `mysqld --initialize-insecure` correctly |

Test fakes under `internal/mysql/testdata/` are Go `main` programs
compiled at test time — never python/ruby/node, per CLAUDE.md.

`internal/server/manager_test.go` extended: reconcile picks up mysql
versions from `WantedVersions()`, stops removed ones — mirrors the
postgres assertion added in PR #75.

`internal/commands/mysql/*_test.go`: each command's argument parsing
and dispatch.

**E2E phase** at `scripts/e2e/mysql-binary.sh`, wired into
`.github/workflows/e2e.yml` as a new phase (next number after PR #75's
postgres-binary phase). Mirrors `scripts/e2e/postgres-binary.sh`:

1. `pv mysql:install 8.4` and `pv mysql:install 9.7` in parallel —
   both succeed.
2. Both bind their ports — `nc -z 127.0.0.1 33084 && nc -z 127.0.0.1 33097`.
3. Connect with the bundled `mysql` client over the unix sockets, run
   `SELECT VERSION();` — versions match expectations.
4. Cross-version isolation: `CREATE DATABASE x` on 8.4 is not visible
   on 9.7.
5. `pv mysql:list` shows both rows.
6. `pv mysql:uninstall 8.4 --force` cleans tree + datadir + state;
   `mysql:list` shows only 9.7 left.
7. `pv mysql:uninstall 9.7 --force` cleans the rest.

The `scripts/test-mysql-bundle.sh` smoke test (already in repo, used
by the artifacts pipeline) is **not** what we run here — the artifacts
pipeline owns it. The e2e script is pv-side, exercising the CLI
surface.

**Manual verification before merge:**
- `pv mysql:install 8.4` then `pv mysql:install 9.7` then
  `pv mysql:list` shows both.
- Stop the daemon, start it again — both come back up.
- Kill one mysqld out of band — supervisor restarts within the
  budget; the other version is unaffected.
- `pv link` on a Laravel project with `DB_CONNECTION=mysql` — `.env`
  picks up the right port; `php artisan migrate` succeeds.

`go test ./...`, `go vet ./...`, `gofmt -l .` all pass clean.

## Failure modes

| Failure | Behavior |
|---|---|
| Tarball download fails | `mysql:install` errors before any on-disk changes; user retries. |
| `mysqld --initialize-insecure` fails partway | Partial datadir removed; user retries cleanly. |
| Crash during runtime | Supervisor's existing 5-restarts-in-60s budget per supervisor key. Each version independent. After budget exhausted, the version is dropped from supervision; `state.json` still says `wanted: running` and the next daemon restart retries. |
| `state.json` `mysql` slice corrupt | Treated as empty; warning logged once; user runs `mysql:start <version>` to recover. |
| Binaries deleted out of band but `state.json` says `running` | `WantedVersions()` filters to "installed AND wanted-running"; missing version silently dropped, warning on stderr. |
| User runs `mysql:install` while daemon is down | Install completes, `state.json` set, signal-daemon is a no-op; next `pv start` brings the process up. |
| Two pv binaries try to install the same version concurrently | Both download; second one's `os.Rename` over the staging dir wins. Init is guarded by `auto.cnf`. Worst case: a half-extracted tarball gets stomped by the winning rename. Acceptable for v1; revisit with a file lock if it becomes a problem. |
| Datadir initialized on 8.0, user runs `mysql:install 9.7` | Different version, different datadir at `~/.pv/data/mysql/9.7/` — no conflict. The 8.0 datadir is untouched. |
| User attempts to start mysqld on a port that's already bound | mysqld fails to bind, supervisor logs the error, reconciler retries on the next tick. The hint in the error log will point at port conflicts. |

## Migration / rollout

1. Land artifacts pipeline (already done — `build-artifacts.yml` ships
   `mysql-mac-arm64-{8.0,8.4,9.7}.tar.gz`).
2. Implement `internal/mysql/` package + `internal/binaries/mysql.go`.
3. Implement `internal/commands/mysql/` + `cmd/mysql.go` bridge.
4. Extend `reconcileBinaryServices` with the third wanted-set source.
5. Wire `laravel.SmartEnvVars` to call `mysql.EnvVars` for the mysql
   branch.
6. Wire `automation/steps/detect_services.go` to read installed
   versions from `internal/mysql/` instead of `reg.Services`.
7. Delete `internal/services/mysql.go` + `_test.go`; update remaining
   references.
8. Update setup wizard: add MySQL 8.4 LTS checkbox, drop the
   docker-mysql multi-select option.
9. Update `cmd/install.go`, `cmd/update.go`, `cmd/uninstall.go`
   orchestrators with mysql passes.
10. Add `scripts/e2e/mysql-binary.sh` e2e phase.
11. Manual verification on macOS arm64.
12. Merge.

No coordination needed with end users — the repo is early-stage, no
data-migration step exists. Abandoned Docker-era mysql datadirs are
left in place but unreferenced.
