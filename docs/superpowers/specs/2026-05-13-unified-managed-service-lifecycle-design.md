# Unified managed service lifecycle

## Goal

Standardize `postgres`, `mysql`, `redis`, `mailpit`, and `rustfs` around one managed-service lifecycle model before extracting shared helpers.

All managed services should be version-line based, installed from archived pv artifacts into versioned roots, supervised by the daemon from those roots, and represented in runtime state by stable version identities. RustFS and Mailpit should stop using `latest` as their persistent identity and stop installing as singleton naked binaries.

After that normalization, extract narrow shared mechanics for state, artifacts, command choreography, readiness/waiting, and daemon reconciliation while keeping service packages explicit.

## Background

The service packages now have similar shapes:

```text
internal/postgres/
internal/mysql/
internal/redis/
internal/mailpit/
internal/rustfs/
internal/commands/{postgres,mysql,redis,mailpit,rustfs}/
```

The similar shape is good, but it left duplicated mechanics in each service:

- wanted-state load/save/set/remove logic
- installed/wanted version filtering
- artifact download/extract/swap/update flow
- start/stop/restart/update/uninstall command choreography
- TCP readiness and wait-stopped polling
- command registration and hidden alias cloning
- daemon reconcile loops over wanted services
- registry and `pv.yml` binding glue

Earlier parity work intentionally deferred shared abstractions until all services had the same broad boundaries. That point has arrived, but Mailpit and RustFS still differ in two important ways:

- their user-visible service identity is `latest`
- RustFS is uploaded as a naked binary artifact, and both services install as singleton binaries under `InternalBinDir`

Those differences make shared lifecycle helpers more awkward than necessary. Normalize Mailpit and RustFS first, then extract shared mechanics.

## Locked decisions

- Treat all five managed services as first-class services: Postgres, MySQL, Redis, Mailpit, and RustFS.
- Make every managed service install from a pv-managed archive artifact.
- Make Mailpit and RustFS install into versioned service roots, not singleton paths in `config.InternalBinDir()`.
- Remove `latest` from user-facing service version identity.
- Use stable version lines as runtime identity:
  - Postgres: major line, e.g. `18`
  - MySQL: minor line, e.g. `8.4`
  - Redis: minor line, e.g. `8.6`
  - Mailpit: major line, initially `1`
  - RustFS: prerelease line, initially `1.0.0-beta`
- Omitted CLI version arguments remain a convenience and resolve to the current default supported line.
- Persist resolved version lines in state, registry project bindings, data paths, log paths, and process names.
- Do not introduce a `latest` alias for Postgres, MySQL, Redis, Mailpit, or RustFS.
- Do not attempt automatic cross-line data upgrades, dumps, imports, or migrations.
- Do not preserve compatibility with existing prototype installs that use `mailpit-latest`, `rustfs-latest`, or `latest` state. Users can uninstall/reinstall during the prototype phase.
- Keep explicit service command groups. Do not reintroduce a generic `service:*` namespace.
- Keep service packages explicit. Extract shared mechanics, not a large generic service framework.

## Non-goals

- No automatic stateful upgrades between version lines, such as Postgres 18 to 19 or MySQL 8.4 to 9.7.
- No data migration tooling for databases, Redis, RustFS buckets, or Mailpit data.
- No hidden `.env` inference or service binding outside `pv.yml`.
- No PATH exposure for service binaries beyond existing tool/shim rules.
- No central service framework that forces every service to implement every possible behavior.
- No backward-compatible migration from the current RustFS/Mailpit `latest` state shape.

## Version identity

Service identity is a stable supported line, not a moving alias.

```text
postgres 18
mysql    8.4
redis    8.6
mailpit  1
rustfs   1.0.0-beta
```

The artifact build workflow may resolve the newest patch/release inside a line:

```text
postgres 18        -> latest Postgres 18.x artifact
mysql 8.4          -> latest MySQL 8.4.x artifact
redis 8.6          -> latest Redis 8.6.x artifact
mailpit 1          -> latest Mailpit 1.x artifact
rustfs 1.0.0-beta  -> latest RustFS 1.0.0-beta.x artifact
```

Runtime state still stores the line, not the upstream patch release. The upstream patch/build version belongs in `versions.json`, keyed by service and line, e.g. `mailpit-1`, `rustfs-1.0.0-beta`, `postgres-18`.

Omitted versions resolve to defaults:

```bash
pv postgres:install
pv mysql:install
pv redis:install
pv mailpit:install
pv rustfs:install
```

The command output should make the resolved line visible:

```text
Installing Mailpit 1...
Installing RustFS 1.0.0-beta...
```

`latest` is rejected because it is unstable identity. If `postgres latest` means `18` today and `19` tomorrow, the same state entry can accidentally point at incompatible data. `pv` should not own cross-line upgrades yet.

## Artifact model

All managed services should have archive artifacts with consistent naming and installer expectations.

Example artifact names:

```text
postgres-mac-arm64-18.tar.gz
mysql-mac-arm64-8.4.tar.gz
redis-mac-arm64-8.6.tar.gz
mailpit-mac-arm64-1.tar.gz
rustfs-mac-arm64-1.0.0-beta.tar.gz
```

The workflow can continue resolving upstream releases internally, but uploaded artifacts should be keyed by pv's supported line.

Archive layout should be normalized enough for one extraction path. The exact internal layout can differ when a service needs it, but single-executable services should not upload naked binaries.

Preferred Mailpit/RustFS archive layout:

```text
bin/mailpit
bin/rustfs
```

The installer extracts the archive into the service version root. Service packages then know where their executable is located.

## Filesystem layout

Use one clear path model for every managed service. Binaries live under top-level service roots. Stateful data lives under `~/.pv/data/{service}/{version}`. Logs live under `~/.pv/logs/{service}-{version}.log`.

Do not put Mailpit or RustFS under `~/.pv/services/`. That directory shape is old service-era baggage and should not be part of the normalized lifecycle model. Because the project is pre-GA and backward compatibility is out of scope, Postgres can also move off `config.ServiceDataDir("postgres", major)` if doing so makes the implementation cleaner.

Target shape:

```text
~/.pv/
  postgres/18/...
  mysql/8.4/...
  redis/8.6/...
  mailpit/1/bin/mailpit
  rustfs/1.0.0-beta/bin/rustfs
  data/
    postgres/18/...
    mysql/8.4/...
    redis/8.6/...
    mailpit/1/...
    rustfs/1.0.0-beta/...
  logs/
    postgres-18.log
    mysql-8.4.log
    redis-8.6.log
    mailpit-1.log
    rustfs-1.0.0-beta.log
```

The exact helper names can be decided during implementation, but the path families should stay clear:

- binary roots: `~/.pv/{service}/{version}/...`
- data roots: `~/.pv/data/{service}/{version}/...`
- logs: `~/.pv/logs/{service}-{version}.log`

Use canonical service names for storage paths: `postgres`, `mysql`, `redis`, `mailpit`, and `rustfs`. Keep aliases such as `pg`, `mail`, and `s3` as CLI/route compatibility, not filesystem identity.

The important requirement is stable per-line paths for binaries, data, logs, process names, and registry bindings.

## State shape

Every service stores wanted state by version line.

Postgres may keep `majors` if preserving its current naming is cleaner, but the underlying semantics should match the other services.

```json
{
  "postgres": { "majors": { "18": { "wanted": "running" } } },
  "mysql": { "versions": { "8.4": { "wanted": "running" } } },
  "redis": { "versions": { "8.6": { "wanted": "running" } } },
  "mailpit": { "versions": { "1": { "wanted": "running" } } },
  "rustfs": { "versions": { "1.0.0-beta": { "wanted": "running" } } }
}
```

`latest` should not be accepted by `ValidateVersion` for any service.

## Public service API shape

Each service package should keep an explicit API that callers can read without understanding a central framework.

Common shape:

```go
func DefaultVersion() string
func ResolveVersion(version string) (string, error)
func ValidateVersion(version string) error

func Install(client *http.Client, version string) error
func InstallProgress(client *http.Client, version string, progress binaries.ProgressFunc) error
func Update(client *http.Client, version string) error
func UpdateProgress(client *http.Client, version string, progress binaries.ProgressFunc) error
func Uninstall(version string, force bool) error

func IsInstalled(version string) bool
func InstalledVersions() ([]string, error)
func SetWanted(version, wanted string) error
func RemoveVersion(version string) error
func WantedVersions() ([]string, error)
func BuildSupervisorProcess(version string) (supervisor.Process, error)
func WaitStopped(version string, timeout time.Duration) error
```

Service-specific packages may expose additional functions where real behavior differs, such as Postgres/MySQL database commands or service-specific template variables.

## Shared mechanics to extract

After Mailpit and RustFS are normalized, extract helpers in small packages. These helpers should remove repeated mechanics while leaving service behavior in the service packages.

### `internal/servicestate`

Responsibilities:

- load and save service slices from `internal/state`
- initialize nil maps
- validate wanted values
- set/remove version entries
- filter wanted-running entries by installed versions
- sort deterministic version results
- warn about stale wanted entries consistently

This package should not know how to build service processes or install artifacts.

### `internal/serviceartifact`

Responsibilities:

- resolve archive URLs/names for supported service lines
- download archives with progress
- extract into staging directories
- atomically replace version roots
- record resolved upstream/build versions in `versions.json`
- run explicit hooks before or after extraction/swap

This is a Template Method style helper implemented idiomatically with Go structs and function hooks.

Example shape:

```go
type ArchivePlan struct {
    ServiceName    string
    Version        string
    VersionDir     string
    ResolveURL     func(string) (string, error)
    AfterExtract   func(stagingDir string) error
    AfterInstall   func(versionDir string) error
    ProbeVersion   func(versionDir string) (string, error)
    VersionKey     string
}
```

The shared helper owns the repeated algorithm. The service package supplies strategies for init, probing, path validation, and post-install work.

### `internal/servicecmd`

Responsibilities:

- register command lists
- clone hidden aliases such as `pg:*`, `mail:*`, and `s3:*`
- share lifecycle command choreography where it is truly identical
- keep UI output in command packages through `internal/ui`

This package should avoid owning domain behavior. It can take function adapters for `ResolveVersion`, `IsInstalled`, `SetWanted`, `WaitStopped`, and `UpdateProgress`.

### `internal/servicewait`

Responsibilities:

- shared TCP readiness checks
- shared wait-for-port-closed polling
- optional post-stop hooks for services that need extra validation, such as MySQL PID/process checks

Prefer reusing or extending existing `internal/supervisor` readiness helpers where that is cleaner.

### `internal/servicereconcile`

Responsibilities:

- hold narrow daemon reconciliation descriptors
- let `server.Manager` loop over service descriptors instead of hand-writing the same wanted/build-process pattern five times

Example shape:

```go
type Descriptor struct {
    Name           string
    WantedVersions func() ([]string, error)
    BuildProcess   func(version string) (supervisor.Process, error)
}
```

This is a boundary adapter, not a service framework.

## What stays service-specific

Keep these behaviors explicit in their service packages:

- Postgres: major-line policy, `initdb`, config/HBA rewrite, socket dirs, database create/drop.
- MySQL: minor-line policy, `mysqld --initialize-insecure`, socket/PID behavior, force-data semantics, database create/drop.
- Redis: supported minor policy, Redis args, persistence/protection settings.
- Mailpit: supported major policy, SMTP/web ports, web route, mail env/template variables, Mailpit args.
- RustFS: supported prerelease line policy, credentials, API/console ports, S3 routes/env/template variables, RustFS args.

The guiding rule is:

```text
shared mechanics, explicit service behavior
```

## Command layer

First-class service commands remain:

```text
postgres:*
pg:*       hidden alias
mysql:*
redis:*
mailpit:*
mail:*     hidden alias
rustfs:*
s3:*       hidden alias
```

MySQL should continue not having a `my:*` alias.

Command packages keep ownership of:

- Cobra command definitions
- prompts and confirmations
- `internal/ui` output
- progress wrappers
- daemon signaling
- user-facing connection details

Every successful service command that changes daemon-observed state should signal the daemon after the filesystem/state changes are complete. The signal helper may no-op with a subtle message when the daemon is not running, but command code should still route through that helper so service lifecycle commands have one consistent post-change reconciliation hook.

Internal service packages should not signal the daemon. They own filesystem, state, artifact, and process-definition work. Cobra command packages own daemon signaling.

Daemon signaling policy:

| Command | Signal timing | Why |
|---|---|---|
| `service:install` | after successful install/wanted-running/config generation | start newly wanted service or refresh status |
| `service:start` | after wanted-running is saved | start service immediately when daemon is running |
| `service:stop` | after wanted-stopped is saved | stop service immediately when daemon is running |
| `service:restart` | after wanted-stopped before waiting, and after wanted-running | drive both halves of restart through reconcile |
| `service:update` | after wanted-stopped before waiting, and after successful update | stop process before binary swap, then reconcile final state/status |
| `service:uninstall` | after wanted-stopped before waiting, and after successful cleanup/unbind/config generation | stop before destructive filesystem work, then reconcile removed service/status/routes |

Commands that only read state or talk to a service without changing daemon-observed topology should not signal. That includes `service:status`, `service:list`, `service:logs`, and current database create/drop commands. If a future command starts mutating registry service bindings, Caddy service routes, wanted state, service install roots, or supervisor process definitions, it should follow the signaling policy above.

Hidden `service:download` commands are currently debug wrappers around full install pipelines for some services. Prefer turning them into internal helpers or ensuring public install commands are the only path that signals, to avoid double signaling. If a hidden download command remains directly executable and mutates wanted/install state, it should use the same final signal behavior as install.

Shared command helpers can remove duplicated registration and lifecycle choreography, but each service command package should still be easy to inspect.

## Registry and pv.yml

Project service bindings should store stable version lines.

Example generated `pv.yml`:

```yaml
postgresql:
  version: "18"

mysql:
  version: "8.4"

redis:
  version: "8.6"

mailpit:
  version: "1"

rustfs:
  version: "1.0.0-beta"
```

If the current schema treats Redis, Mailpit, or RustFS versions as implicit, update the schema so every managed service can declare an explicit version line. Generated files should prefer explicit version lines for clarity.

When a service block exists but omits `version`, `pv` should resolve that service to its current default version line. This lets users simplify generated `pv.yml` files without losing the binding:

```yaml
mailpit:
  env:
    MAIL_HOST: "{{ .smtp_host }}"
```

The example above means “bind Mailpit using `mailpit.DefaultVersion()`”, currently `1`. A missing service block still means the service is not bound. After resolution, registry project bindings should store the concrete resolved version line, not an empty string and not `latest`.

Registry project bindings should also hold these version lines. Empty string means not bound.

Uninstall cleanup should use service-specific unbind helpers, eventually backed by shared binding mechanics if that removes duplication without hiding behavior.

## Update semantics

`update` refreshes an installed line within the same line.

Examples:

```text
postgres:update 18        -> latest supported Postgres 18.x artifact
mysql:update 8.4          -> latest supported MySQL 8.4.x artifact
mailpit:update 1          -> latest supported Mailpit 1.x artifact
rustfs:update 1.0.0-beta  -> latest supported RustFS 1.0.0-beta.x artifact
```

`update` does not move data across lines.

After a successful update, the command layer should always signal the daemon through the service's signal helper. If the service was wanted-running before the update, restore wanted-running before signaling. If it was wanted-stopped, leave it stopped but still signal so the running daemon can reconcile any supervised process or status snapshot affected by the artifact change.

Moving to a new line is explicit:

```bash
pv postgres:install 19
pv mysql:install 9.7
```

The user is responsible for data migration during the prototype phase.

## Implementation order

1. Normalize artifact publishing.
   - Make RustFS upload a `.tar.gz` archive.
   - Keep Mailpit as an archive but normalize its layout to match installer expectations.
   - Ensure artifact names are keyed by pv-supported version lines.
2. Replace Mailpit/RustFS `latest` identity.
   - Add supported version-line constants.
   - Reject `latest` in validation.
   - Update tests, process names, log paths, data paths, registry bindings, and `pv.yml` generation.
3. Move Mailpit/RustFS to versioned install roots.
   - Stop using singleton `InternalBinDir` paths for service binaries.
   - Build supervisor processes from the selected version root.
4. Extract shared helpers.
   - Start with `servicestate` and `serviceartifact`, because they remove the largest duplicated mechanics.
   - Then extract `servicewait`, `servicecmd`, and `servicereconcile` where the seams are clear.
5. Cleanup and consistency pass.
   - Remove stale `latest` assumptions.
   - Remove duplicated alias helpers.
   - Add consistency tests across all service command packages.

## Risks

- Artifact names can drift between `.github/workflows/build-artifacts.yml` and installer URL resolution.
- Shared helpers can become too broad if they abstract service behavior instead of mechanics.
- Command behavior can drift during helper extraction, especially aliases, prompts, force flags, and daemon signaling.
- Mailpit/RustFS Caddy route generation and registry fallback behavior can break when version identity changes.
- The existing prototype installs using `latest` will break. This is acceptable because backward compatibility is explicitly out of scope before GA.

## Testing

Add or update tests for:

- Mailpit/RustFS version validation and default resolution.
- Artifact URL/name/path resolution for all services.
- Mailpit/RustFS archive extraction into versioned roots.
- Installed version detection from versioned roots.
- Supervisor process names: `mailpit-1`, `rustfs-1.0.0-beta`, etc.
- Data/log paths no longer using `latest`.
- Wanted-state helper behavior, including stale wanted entries.
- Command registration and hidden aliases.
- Update/uninstall stop-wait-signal behavior.
- `pv.yml` generation with explicit service version lines.
- Daemon reconciliation through descriptors.

Before handoff for Go changes, run:

```bash
gofmt -w .
go vet ./...
go build ./...
go test ./...
```

## Recommended pattern summary

- Structural: narrow descriptors/adapters at boundaries, especially daemon reconciliation and command registration.
- Behavioral: Template Method for artifact install/update skeletons, with Strategy hooks for service-specific init, probe, args, and cleanup.
- Creational: small factory/builder functions for Cobra commands and supervisor processes.

Avoid a large generic service framework. The goal is maintainable symmetry, not fake sameness.
