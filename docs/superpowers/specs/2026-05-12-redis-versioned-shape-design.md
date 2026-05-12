# Redis versioned shape (align with postgres/mysql)

## Goal

Refactor `internal/redis/` from a flat single-version package to a
version-parameterized shape matching `internal/mysql/` and
`internal/postgres/`. The public API takes a `version string` everywhere;
paths become `~/.pv/redis/{version}/`. Only one version is shipped
(currently 8.6), but the API supports multiple.

This is the consistency step before extracting a shared interface across
all three binary-service packages.

## Scope

- All `internal/redis/` functions grow a `version string` parameter.
- Binary path: `~/.pv/redis/{version}/` (was `~/.pv/redis/`).
- Data path: `~/.pv/data/redis/{version}/` (was `~/.pv/data/redis/`).
- Log path: `~/.pv/logs/redis-{version}.log` (was `~/.pv/logs/redis.log`).
- State: versioned map matching mysql — `{"redis": {"versions": {"8.6": {"wanted": "running"}}}}`.
- Registry: `ProjectServices.Redis` changes from `bool` to `string`.
- Supervisor key: `redis-{version}` (was `redis`).
- Manager iteration: mirrors mysql's `WantedVersions()` loop.
- Default version: `"8.6"` — used whenever caller omits.

## Filesystem layout

```
~/.pv/
├── redis/
│   ├── 7.4/                              (if installed)
│   │   ├── redis-server
│   │   └── redis-cli
│   └── 8.6/                              (default)
│       ├── redis-server
│       └── redis-cli
├── data/
│   └── redis/
│       ├── 7.4/
│       │   └── dump.rdb
│       └── 8.6/
│           └── dump.rdb
└── logs/
    ├── redis-7.4.log
    └── redis-8.6.log
```

## Config paths

```go
// New:
func RedisVersionDir(version string) string    // ~/.pv/redis/{version}/
func RedisDataDir(version string) string       // ~/.pv/data/redis/{version}/
func RedisLogPath(version string) string       // ~/.pv/logs/redis-{version}.log
func RedisDefaultVersion() string              // "8.6"

// Stay:
func RedisDir() string                         // ~/.pv/redis/ (parent)
func RedisDataRoot() string                    // ~/.pv/data/redis/ (parent)
```

`RedisVersionDir` is the binary root (matching postgres where `PostgresBinDir(major)` is the version dir itself — no separate `bin/` subdirectory).

## State shape

Matches mysql exactly. Flat `{"wanted": "running"}` replaced by versioned map:

```go
type State struct {
    Versions map[string]VersionState `json:"versions"`
}
type VersionState struct {
    Wanted string `json:"wanted"`
}
```

On-disk shape:

```json
{
  "postgres": { "majors": { "17": { "wanted": "running" } } },
  "mysql":    { "versions": { "8.4": { "wanted": "running" } } },
  "redis":    { "versions": { "8.6": { "wanted": "running" } } }
}
```

Functions:
- `LoadState()` → `State` (with `Versions` map, never nil)
- `SaveState(s State)` → writes the `"redis"` slice
- `SetWanted(version, wanted)` → validates + saves
- `RemoveVersion(version)` → deletes one version from map
- `WantedVersions()` → returns wanted+running versions (mirrors mysql's `WantedVersions`)
- `InstalledVersions()` → lists version dirs on disk with valid binaries

## Public API changes

Every function that currently takes no version parameter grows one:

| Current | New |
|---|---|
| `Install(client)` | `Install(client, version)` |
| `InstallProgress(client, progress)` | `InstallProgress(client, version, progress)` |
| `Uninstall(force)` | `Uninstall(version, force)` |
| `Update(client)` | `Update(client, version)` |
| `IsInstalled()` | `IsInstalled(version)` |
| `IsWanted()` | (removed — replaced by `WantedVersions()`) |
| `SetWanted(wanted)` | `SetWanted(version, wanted)` |
| `ProbeVersion()` | `ProbeVersion(version)` |
| `BuildSupervisorProcess()` | `BuildSupervisorProcess(version)` |
| `PortFor()` | `PortFor(version)` |
| `EnvVars(projectName)` | `EnvVars(version, projectName)` |
| `ServerBinary()` | `ServerBinary(version)` |
| `CLIBinary()` | `CLIBinary(version)` |

New:
- `WantedVersions()` → `[]string`
- `InstalledVersions()` → `[]string`
- `RemoveVersion(version)` — state cleanup
- `BindLinkedProjects(version)` — retroactive auto-bind
- `WaitStopped(version, timeout)` — per-version port wait

## Install flow

```
Install(client, version):
  1. Resolve URL (version-agnostic — we only ship one artifact)
  2. Download + extract into staging, then rename to ~/.pv/redis/{version}/
  3. chown tree to SUDO_USER
  4. Create data dir: ~/.pv/data/redis/{version}/, chown
  5. Probe redis-server --version, record in versions.json as "redis-{version}"
  6. SetWanted(version, WantedRunning)
  7. BindLinkedProjects(version)
  8. Signal daemon
```

## Uninstall flow

```
Uninstall(version, force):
  1. SetWanted(version, WantedStopped) + signal daemon
  2. WaitStopped(version, 10s)
  3. Remove ~/.pv/redis/{version}/
  4. Remove ~/.pv/logs/redis-{version}.log
  5. If force: remove ~/.pv/data/redis/{version}/
  6. RemoveState entry for version
  7. Remove "redis-{version}" from versions.json
  8. reg.UnbindRedisVersion(version)
```

## Update flow

```
Update(client, version):
  1. wasWanted = SetWanted(version, WantedStopped) -> signal + wait
  2. Redownload + extract via staging-rename over ~/.pv/redis/{version}/
  3. Probe version, update versions.json
  4. If wasWanted: SetWanted(version, WantedRunning) -> signal
```

## Port allocation

`PortFor(version)` returns a version-aware port matching mysql's formula:
`6300 + major*100 + minor*10`. Examples: 7.4 → 6740, 8.6 → 6860.

The constant `RedisPort = 6379` is removed.

## Process / supervisor

`BuildSupervisorProcess(version)` uses version-specific paths:

```go
func BuildSupervisorProcess(version string) (supervisor.Process, error) {
    binPath := filepath.Join(config.RedisVersionDir(version), "redis-server")
    return supervisor.Process{
        Name:    "redis-" + version,
        Binary:  binPath,
        Args:    buildRedisArgs(version),
        LogFile: config.RedisLogPath(version),
        ...
    }
}
```

`buildRedisArgs(version)` uses `PortFor(version)` and `RedisDataDir(version)`.

## Manager integration

The flat `redis.IsWanted()` call becomes a versioned iteration, mirroring the mysql block:

```go
// Before:
if redis.IsWanted() {
    proc, err := redis.BuildSupervisorProcess()
    wanted["redis"] = proc
}

// After:
rdVersions, rdErr := redis.WantedVersions()
for _, version := range rdVersions {
    proc, err := redis.BuildSupervisorProcess(version)
    wanted["redis-"+version] = proc
}
```

Transient-error guard for redis follows the same pattern as postgres/mysql.

## Registry changes

`ProjectServices.Redis` changes from `bool` to `string` (JSON tag:
`"redis,omitempty"`). Semantics: the string holds the version bound to
the project. Empty string means "not bound."

New helpers in registry.go:

```go
func (r *Registry) UnbindRedisVersion(version string) {
    for i := range r.Projects {
        if r.Projects[i].Services == nil { continue }
        if r.Projects[i].Services.Redis == version {
            r.Projects[i].Services.Redis = ""
        }
    }
}
```

`UnbindService("redis")` is updated to clear `Services.Redis` to `""`
instead of `false`.

## Commands layer

Every `redis:*` command grows an optional `version` arg, defaulting to
`config.RedisDefaultVersion()`:

| Command | Args |
|---|---|
| `redis:install [version]` | Defaults to 8.6 |
| `redis:uninstall [version] [--force]` | Defaults to 8.6 |
| `redis:update [version]` | Defaults to 8.6 |
| `redis:start [version]` | Defaults to 8.6 |
| `redis:stop [version]` | Defaults to 8.6 |
| `redis:restart [version]` | Defaults to 8.6 |
| `redis:list` | Shows all installed versions |
| `redis:status [version]` | Defaults to 8.6 |
| `redis:logs [version] [-f]` | Defaults to 8.6 |
| `redis:download [version]` | Hidden, debug only |

`redis:list` becomes a multi-row table when multiple versions are
installed (matching `mysql:list`).

## Environment variables

`EnvVars(version, projectName)` returns the same values as before but
uses `PortFor(version)` for `REDIS_PORT`:

```go
func EnvVars(version, projectName string) map[string]string {
    return map[string]string{
        "REDIS_HOST":     "127.0.0.1",
        "REDIS_PORT":     strconv.Itoa(PortFor(version)),
        "REDIS_PASSWORD": "null",
    }
}
```

Auto-bind on install and `pv link` use `version` to set
`Services.Redis = version`.

## Files touched

```
internal/redis/           — all files: public API changes
internal/config/paths.go  — RedisVersionDir, RedisDataDir(version), RedisLogPath(version),
                           RedisDefaultVersion, RedisDataRoot
internal/registry/
  ├── registry.go         — ProjectServices.Redis bool → string; UnbindRedisVersion
  └── registry_test.go    — update tests
internal/server/
  └── manager.go          — redis block: iteration over WantedVersions()
internal/commands/redis/  — all commands: version arg with default
cmd/
  ├── redis.go            — bridge (no change unless version plumbing needed)
  ├── install.go          — pass version to RunInstall
  ├── update.go           — pass version to RunUpdate
  └── uninstall.go        — pass version to RunUninstall
internal/laravel/
  ├── env.go              — redis envvar path uses version
  └── steps.go            — DetectServicesStep sets Services.Redis = version
internal/automation/steps/
  └── detect_services.go  — set Services.Redis = version
docs/superpowers/specs/
  └── 2026-05-09-redis-native-binary-design.md  — superseded; reference kept
```

## Locked decisions

| Topic | Old | New |
|---|---|---|
| Version | Single, no param | Version-parameterized, defaults to 8.6 |
| Binary path | `~/.pv/redis/` | `~/.pv/redis/{version}/` |
| Data path | `~/.pv/data/redis/` | `~/.pv/data/redis/{version}/` |
| Log path | `~/.pv/logs/redis.log` | `~/.pv/logs/redis-{version}.log` |
| State shape | `{"wanted": "running"}` | `{"versions": {"8.6": {"wanted": "running"}}}` |
| Registry binding | `Redis bool` | `Redis string` (version) |
| Supervisor key | `redis` | `redis-{version}` |
| Port | 6379 (constant) | `6300 + major*100 + minor*10` |
| Unbind | `UnbindService("redis")` | `UnbindRedisVersion(version)` |
| Default version | N/A | `"8.6"` |
| Multi-version | Not supported | API supports it (only ship one) |
