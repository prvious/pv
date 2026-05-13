# RustFS and Mailpit service parity

## Goal

Refactor `internal/rustfs/` and `internal/mailpit/` so they match the structure now used by `internal/postgres/`, `internal/mysql/`, and `internal/redis/`.

RustFS and Mailpit are old binary-service code. They still use top-level `registry.Services[...]` entries and registry `Enabled` flags as service lifecycle state. The current service shape has moved past that: databases keep install/runtime state in their own internal packages and `~/.pv/data/state.json`, while the registry stores project bindings.

This change makes all tools/services follow the same broad structure before any shared abstraction is extracted.

## Locked decisions

- Remove top-level `registry.Services["s3"]` and `registry.Services["mail"]` usage for RustFS and Mailpit.
- Do not support or migrate legacy registry state. There are no users yet, so existing old-format installs can be broken/reset by this refactor.
- Keep project bindings, but make them string-valued like the databases:
  - `ProjectServices.Mail string` with value `"latest"` when bound.
  - `ProjectServices.S3 string` with value `"latest"` when bound.
- Use `"latest"` as the single managed instance identifier for both services.
- Add default-version helpers matching Redis' default-version behavior: omitted command/internal versions resolve to `"latest"`.
- Keep the public API version-shaped so RustFS/Mailpit can support multiple managed versions later without another signature refactor. For this PR, `"latest"` is the only valid version.
- Keep actual downloaded binary versions in `versions.json` under `mailpit` and `rustfs`.
- Keep data under `config.ServiceDataDir(service, "latest")`, e.g. `~/.pv/services/s3/latest/data`.
- Keep `mail:*` and `s3:*` aliases.
- Keep command behavior user-facing compatible where practical: `mailpit:install`, `rustfs:start`, `mailpit:logs`, etc.

## Non-goals

- Do not migrate old `registry.Services` entries to the new state shape.
- Do not preserve old boolean `ProjectServices.Mail` or `ProjectServices.S3` JSON values.
- Do not extract a shared service abstraction in this PR.
- Do not remove duplication between `rustfs`, `mailpit`, `redis`, `mysql`, and `postgres` in this PR.
- Do not introduce multi-version user selection for RustFS or Mailpit.

The code will intentionally contain duplicated lifecycle structure after this change. That duplication is acceptable because the immediate goal is shape parity across services. A future PR can extract shared helpers or interfaces once all service packages have the same boundaries and signatures.

## Target structure

RustFS and Mailpit should mirror the database-style split:

```text
internal/rustfs/      pure lifecycle/state/process helpers
internal/mailpit/     pure lifecycle/state/process helpers
internal/commands/*/  cobra commands, UI, prompts, progress, daemon signaling
internal/server/      daemon reconcile loop over wanted versions
internal/registry/    project binding fields only
```

Internal packages should stop printing command UI for lifecycle operations. The command packages own `ui.Step`, `ui.StepProgress`, confirmations, success/subtle output, and daemon signaling. Internal packages return errors and do filesystem/state work.

The current `internal/rustfs/proc` and `internal/mailpit/proc` packages exist to avoid import cycles caused by the parent packages importing `server` and `caddy`. This refactor removes those parent-package imports, so the process builders should move back into `internal/rustfs` and `internal/mailpit`, and the `proc` packages should be deleted.

## State shape

RustFS and Mailpit get service slices in `~/.pv/data/state.json`, matching the versioned shape used by MySQL and Redis:

```json
{
  "rustfs": {
    "versions": {
      "latest": { "wanted": "running" }
    }
  },
  "mailpit": {
    "versions": {
      "latest": { "wanted": "running" }
    }
  }
}
```

Each package defines the same state helpers:

```go
func DefaultVersion() string { return "latest" }

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

func LoadState() (State, error)
func SaveState(State) error
func SetWanted(version, wanted string) error
func RemoveVersion(version string) error
func WantedVersions() ([]string, error)
```

`SetWanted` validates both the wanted value and the service version. For now, only `"latest"` is valid.

The version helper behavior should mirror Redis:

```go
func ResolveVersion(version string) (string, error) {
    if version == "" {
        return DefaultVersion(), nil
    }
    if err := ValidateVersion(version); err != nil {
        return "", err
    }
    return version, nil
}
```

`ValidateVersion` accepts `"latest"` only in this PR. That gives callers the same version-parameterized shape as Redis/MySQL/Postgres while keeping the current single-instance behavior.

## Registry changes

`registry.Services` should no longer be used for RustFS/Mailpit install, runtime, route, or command state.

Change project bindings to strings:

```go
type ProjectServices struct {
    Mail     string `json:"mail,omitempty"`
    MySQL    string `json:"mysql,omitempty"`
    Postgres string `json:"postgres,omitempty"`
    Redis    string `json:"redis,omitempty"`
    S3       string `json:"s3,omitempty"`
}
```

Semantics:
- Empty string means not bound.
- `"latest"` means bound to the singleton Mailpit/RustFS instance.

Registry helpers should match the database pattern:

```go
func (r *Registry) UnbindMailVersion(version string)
func (r *Registry) UnbindS3Version(version string)
```

`ProjectsUsingService`, `UnbindService`, and any tests using `ProjectServices.Mail` or `ProjectServices.S3` must be updated for string fields.

Because legacy compatibility is explicitly out of scope, `UnmarshalJSON` does not need to convert old boolean `mail` or `s3` fields.

## Internal package API

Both `internal/rustfs` and `internal/mailpit` should expose the same public lifecycle shape:

```go
func ResolveVersion(version string) (string, error)
func ValidateVersion(version string) error

func Install(client *http.Client, version string) error
func InstallProgress(client *http.Client, version string, progress binaries.ProgressFunc) error

func Update(client *http.Client, version string) error
func UpdateProgress(client *http.Client, version string, progress binaries.ProgressFunc) error

func Uninstall(version string, force bool) error

func IsInstalled(version string) bool
func InstalledVersions() ([]string, error)

func BuildSupervisorProcess(version string) (supervisor.Process, error)
func WaitStopped(version string, timeout time.Duration) error
func EnvVars(version, projectName string) map[string]string
func TemplateVars(version string) map[string]string
```

`ResolveVersion("")` returns `"latest"`. Any non-empty value other than `"latest"` is an error.

## Install flow

Internal package:

```text
Install(client, "latest"):
  1. Ensure pv directories exist.
  2. Resolve latest upstream release for the binary.
  3. Download/install binary into ~/.pv/internal/bin/.
  4. Record actual binary version in versions.json.
  5. Create data dir under ~/.pv/services/{service}/latest/data.
  6. SetWanted("latest", WantedRunning).
```

Command package:

```text
rustfs:install / mailpit:install:
  1. Resolve version arg, default latest.
  2. If already installed, refresh wanted=running.
  3. Otherwise call hidden download command or internal install with progress.
  4. Generate service site configs.
  5. Signal daemon if running.
  6. Print connection details and success output.
```

The command layer owns the progress bar and final output.

## Start, stop, restart

These commands should match Redis in shape.

```text
start:
  SetWanted(version, running)
  signal daemon if running

stop:
  SetWanted(version, stopped)
  signal daemon if running

restart:
  SetWanted(version, stopped)
  signal daemon
  WaitStopped(version, timeout)
  SetWanted(version, running)
  signal daemon
```

There is no registry `Enabled` flag in these flows.

## Update flow

Update should mirror Redis/MySQL:

```text
1. Verify the service is installed.
2. Capture whether wanted state is running.
3. Set wanted stopped.
4. Signal daemon and wait for stop.
5. Redownload/install the binary.
6. Update versions.json.
7. Restore wanted running only if it was running before.
8. Signal daemon.
```

The command layer owns progress UI. The internal package owns binary replacement and version recording.

## Uninstall flow

Uninstall should mirror the destructive database/service shape:

```text
1. Command prompts unless --force is set.
2. Set wanted stopped.
3. Signal daemon and wait for stop.
4. Remove the binary from ~/.pv/internal/bin/.
5. Remove the service log file.
6. Remove state entry for latest.
7. Remove versions.json entry for the binary.
8. If force/delete-data: remove config.ServiceDataDir(service, "latest").
9. Apply service fallbacks to linked projects.
10. Unbind projects with ProjectServices.Mail/S3 == "latest".
11. Regenerate service site configs.
```

Top-level `registry.Services` removal is not part of the flow because those entries should no longer exist.

## Server reconciliation

`internal/server/manager.go` should stop reading:

```go
reg.Services["s3"]
reg.Services["mail"]
```

Instead, it should mirror Redis:

```go
rustfsVersions, rustfsErr := rustfs.WantedVersions()
for _, version := range rustfsVersions {
    proc, err := rustfs.BuildSupervisorProcess(version)
    wanted["rustfs-"+version] = proc
}

mailpitVersions, mailpitErr := mailpit.WantedVersions()
for _, version := range mailpitVersions {
    proc, err := mailpit.BuildSupervisorProcess(version)
    wanted["mailpit-"+version] = proc
}
```

Supervisor process names should be versioned:
- `rustfs-latest`
- `mailpit-latest`

The transient-error stop guard should include these prefixes, matching the postgres/mysql/redis guard pattern.

## Caddy service console generation

`caddy.GenerateServiceSiteConfigs` currently derives service console routes from `registry.Services`. That must change because `registry.Services` is no longer the source of installed services.

The route generator should derive routes from service install/wanted state:
- RustFS route files exist when `rustfs.IsInstalled("latest")` is true or `rustfs.WantedVersions()` includes `"latest"`.
- Mailpit route files exist when `mailpit.IsInstalled("latest")` is true or `mailpit.WantedVersions()` includes `"latest"`.

This keeps service console routes tied to installed services, not project bindings.

## Commands layer

`internal/commands/rustfs/register.go` and `internal/commands/mailpit/register.go` should expose wrappers with args, matching Redis/Postgres/MySQL:

```go
func RunInstall(args []string) error
func RunUpdate(args []string) error
func RunUninstall(args []string) error
func UninstallForce(version string) error
```

Commands should accept an optional `[version]` argument and default to `latest`, matching Redis' command shape. Only `latest` is valid in this PR.

Examples:

```text
rustfs:install [version]
rustfs:update [version]
rustfs:start [version]
rustfs:stop [version]
rustfs:restart [version]
rustfs:status [version]
rustfs:logs [version]

mailpit:install [version]
mailpit:update [version]
mailpit:start [version]
mailpit:stop [version]
mailpit:restart [version]
mailpit:status [version]
mailpit:logs [version]
```

The alias behavior stays:
- `s3:*` aliases `rustfs:*`.
- `mail:*` aliases `mailpit:*`.

## Orchestrator integration

Core orchestrators should stop special-casing RustFS/Mailpit via registry service entries.

- `pv install --with=service[s3]` calls `rustfscmd.RunInstall([]string{"latest"})` or nil args if the command resolves default.
- `pv install --with=service[mail]` calls `mailpitcmd.RunInstall(...)`.
- `pv update` updates installed RustFS/Mailpit by asking their internal packages for installed versions, not by loading `registry.Services`.
- `pv uninstall` should call `rustfscmd.UninstallForce("latest")` and `mailpitcmd.UninstallForce("latest")` if installed.

## Files touched

Expected implementation areas:

```text
internal/rustfs/              state, wanted, installed, install, update, uninstall, process, status/log helpers
internal/mailpit/             same as rustfs
internal/rustfs/proc/         delete after moving process builder/WebRoutes into parent package
internal/mailpit/proc/        delete after moving process builder/WebRoutes into parent package
internal/commands/rustfs/     command orchestration and Run* signatures
internal/commands/mailpit/    command orchestration and Run* signatures
internal/server/manager.go    wanted-state reconciliation for rustfs/mailpit
internal/caddy/caddy.go       service route generation without registry.Services
internal/registry/registry.go ProjectServices Mail/S3 string fields and unbind helpers
internal/automation/steps/    pv.yml service binding writes "latest" for mailpit/rustfs
cmd/install.go               updated RunInstall signatures
cmd/update.go                update installed rustfs/mailpit via package helpers
cmd/uninstall.go             uninstall rustfs/mailpit via command wrappers
README.md                    source layout no longer documents services/ registry as active shape
```

## Testing

Add/update tests for:

- `ProjectServices.Mail` and `ProjectServices.S3` string bindings.
- `UnbindMailVersion("latest")` and `UnbindS3Version("latest")`.
- `ProjectsUsingService("mail")` and `ProjectsUsingService("s3")` with string bindings.
- RustFS/Mailpit `LoadState`, `SetWanted`, `WantedVersions`, `RemoveVersion`.
- RustFS/Mailpit `Install` records wanted-running and actual binary version.
- RustFS/Mailpit `Update` preserves stopped/running wanted state like Redis/MySQL.
- RustFS/Mailpit `Uninstall` removes binary, state, version manifest entry, data on force, and project bindings.
- Server reconciliation starts/stops `rustfs-latest` and `mailpit-latest` from state, not registry.
- Caddy service route generation works without `registry.Services`.
- Alias registration remains hidden and shares implementation.

Run after implementation:

```bash
gofmt -w .
go vet ./...
go build ./...
go test ./...
```

## Success criteria

- No RustFS/Mailpit production code reads or writes top-level `registry.Services["s3"]` or `registry.Services["mail"]`.
- RustFS/Mailpit runtime state lives in `state.json` through package-owned helpers.
- RustFS/Mailpit project bindings are string-valued and use `"latest"`.
- RustFS/Mailpit commands and internal APIs default omitted versions to `"latest"` while preserving version-parameterized signatures.
- Command packages own UI, prompts, progress, and daemon signaling.
- Server reconciliation treats RustFS/Mailpit like Redis/MySQL/Postgres wanted services.
- Tests pass with `go test ./...`, and the repo remains `gofmt`/`go vet` clean.
