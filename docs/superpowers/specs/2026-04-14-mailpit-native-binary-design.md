# Mailpit as a Native Supervised Binary (Mail Service)

**Date:** 2026-04-14
**Status:** Approved

## Relationship to prior specs

This spec depends on `docs/superpowers/specs/2026-04-14-rustfs-native-binary-design.md` landing first. That spec establishes:

- The `BinaryService` interface (`internal/services/binary.go`)
- The `supervisor` package (`internal/supervisor/`)
- The `reconcileBinaryServices` phase inside `ServerManager.Reconcile()`
- The `Kind` and `Enabled` fields on `registry.ServiceInstance`
- The `resolveKind` dispatcher used by `service:*` commands
- The `buildSupervisorProcess` helper that translates a `BinaryService` into a `supervisor.Process`

This spec is **purely additive**: a second `BinaryService` implementation, plus a removal of mail from the Docker service registry. No interface changes, no supervisor changes, no new command logic.

## Problem

The `mail` service currently runs Mailpit via Docker (`axllent/mailpit:latest`) — a Go binary shipped as a container image. For every `pv service:add mail` we pull and run a ~50 MB container image when the underlying tool is a ~15 MB single static binary that upstream distributes directly on GitHub Releases.

Mailpit is the strongest candidate for binary migration: it's a Go program, upstream ships cross-platform tarballs for all our target platforms, and it has no persistent state users interact with outside of its own on-disk mail database.

## Goals

- Run Mailpit as a native binary supervised by the pv daemon.
- Remove the Docker image dependency for mail entirely.
- Reuse the `BinaryService` infrastructure built for RustFS — one more `BinaryService` implementation, nothing structural.

## Non-Goals

- Do **not** touch other Docker services (mysql, postgres, redis) — they stay on Docker until separately migrated.
- Do **not** change Mailpit's on-disk database format or storage location semantics beyond mapping them to `config.ServiceDataDir`.
- Do **not** add SMTP authentication or any security hardening. Local-dev tool; matches current behavior.
- Do **not** support multi-version Mailpit. Single latest version; no user has a "my app needs Mailpit v1.20 specifically" story.

## Verified facts

These were confirmed via GitHub API (not assumed from memory):

- **Upstream:** `axllent/mailpit`
- **Latest release as of spec date:** `v1.29.6`
- **Assets** (exact filenames):
  - `mailpit-darwin-arm64.tar.gz`
  - `mailpit-darwin-amd64.tar.gz`
  - `mailpit-linux-arm64.tar.gz`
  - `mailpit-linux-amd64.tar.gz`
  - (plus linux-386, linux-arm, windows — not targeted)
- **Archive contents:** a single `mailpit` binary at the archive root (plus license/readme). **VERIFY during implementation** by extracting once.

## Architecture

### New files

| Path | Purpose |
|------|---------|
| `internal/services/mailpit.go` | `Mailpit` struct implementing `BinaryService` (registered as `"mail"`) |
| `internal/services/mailpit_test.go` | Unit tests for `Mailpit` method outputs |
| `internal/binaries/mailpit.go` | Platform-specific archive name + download URL (mirrors `binaries/mago.go` and `binaries/rustfs.go`) |
| `scripts/e2e/mail-binary.sh` | E2E phase exercising the full binary flow |

### Deleted files

| Path | Reason |
|------|--------|
| `internal/services/mail.go` | Docker implementation replaced by `mailpit.go`. The old `Mail` struct (implementing the Docker `Service` interface) is gone. |
| `internal/services/mail_test.go` | Tests for deleted struct. Replaced by `mailpit_test.go`. |

### Modified files

| Path | Change |
|------|--------|
| `internal/services/service.go` | Remove `"mail": &Mail{}` from the Docker `registry` map. `Available()` return list stays correct because `mail` now lives in `binaryRegistry`. |
| `internal/services/binary.go` | Add `"mail": &Mailpit{}` to `binaryRegistry`. |
| `internal/binaries/manager.go` | Add `"mailpit":` cases in `DownloadURL()` and `LatestVersionURL()`. Do **not** add to `Tools()` — Mailpit is a backing service, not a user-exposed tool. |
| `.github/workflows/e2e.yml` | Add `scripts/e2e/mail-binary.sh` phase. |

## Components

### `Mailpit` implementation (`internal/services/mailpit.go`)

```go
package services

import (
    "time"

    "github.com/prvious/pv/internal/binaries"
)

type Mailpit struct{}

func (m *Mailpit) Name() string        { return "mail" }
func (m *Mailpit) DisplayName() string { return "Mail (Mailpit)" }

func (m *Mailpit) Binary() binaries.Binary { return binaries.Mailpit }

func (m *Mailpit) Args(dataDir string) []string {
    // VERIFY during implementation: exact flag names by running
    // `./mailpit --help` on the downloaded binary. The flags below
    // are the plausible defaults from Mailpit's docs but have NOT
    // been verified against a running binary in this spec.
    return []string{
        "--smtp", ":1025",
        "--listen", ":8025",
        "--database", dataDir + "/mailpit.db",
    }
}

func (m *Mailpit) Env() []string { return nil }

func (m *Mailpit) Port() int        { return 1025 } // SMTP
func (m *Mailpit) ConsolePort() int { return 8025 } // HTTP UI

func (m *Mailpit) WebRoutes() []WebRoute {
    return []WebRoute{
        {Subdomain: "mail", Port: 8025},
    }
}

func (m *Mailpit) EnvVars(_ string) map[string]string {
    return map[string]string{
        "MAIL_MAILER":   "smtp",
        "MAIL_HOST":     "127.0.0.1",
        "MAIL_PORT":     "1025",
        "MAIL_USERNAME": "",
        "MAIL_PASSWORD": "",
    }
}

func (m *Mailpit) ReadyCheck() ReadyCheck {
    // Mailpit exposes /livez reliably — use HTTP probe, not TCP.
    return ReadyCheck{
        HTTPEndpoint: "http://127.0.0.1:8025/livez",
        Timeout:      30 * time.Second,
    }
}
```

**Intentional parity with the current Docker mail service:** every `EnvVars` key, both port numbers, and the `WebRoute` are preserved so linked Laravel projects keep working without `.env` rewrites.

### `binaries.Mailpit` (`internal/binaries/mailpit.go`)

```go
package binaries

import (
    "fmt"
    "runtime"
)

var Mailpit = Binary{
    Name:         "mailpit",
    DisplayName:  "Mailpit",
    NeedsExtract: true, // .tar.gz
}

var mailpitPlatformNames = map[string]map[string]string{
    "darwin": {
        "arm64": "darwin-arm64",
        "amd64": "darwin-amd64",
    },
    "linux": {
        "amd64": "linux-amd64",
        "arm64": "linux-arm64",
    },
}

func mailpitArchiveName() (string, error) {
    archMap, ok := mailpitPlatformNames[runtime.GOOS]
    if !ok {
        return "", fmt.Errorf("unsupported OS for Mailpit: %s", runtime.GOOS)
    }
    platform, ok := archMap[runtime.GOARCH]
    if !ok {
        return "", fmt.Errorf("unsupported architecture for Mailpit: %s/%s", runtime.GOOS, runtime.GOARCH)
    }
    return fmt.Sprintf("mailpit-%s.tar.gz", platform), nil
}

func mailpitURL(version string) (string, error) {
    archive, err := mailpitArchiveName()
    if err != nil {
        return "", err
    }
    return fmt.Sprintf("https://github.com/axllent/mailpit/releases/download/%s/%s", version, archive), nil
}
```

Then `binaries/manager.go`:

```go
// DownloadURL
case "mailpit":
    return mailpitURL(version)

// LatestVersionURL
case "mailpit":
    return "https://api.github.com/repos/axllent/mailpit/releases/latest"
```

## Data Flow

Identical to the rustfs flow in the prior spec. Summary:

- `pv service:add mail` → download Mailpit binary → register `{Kind: "binary", Port: 1025, ConsolePort: 8025, Enabled: &trueVal}` → `server.SignalDaemon()`
- `pv start` / SIGHUP → `ServerManager.Reconcile()` → `reconcileBinaryServices()` sees mail registered and enabled → `buildSupervisorProcess(&Mailpit{})` → `supervisor.Start`
- `pv service:stop mail` → set `Enabled=false` → `SignalDaemon()` → reconcile stops the supervised process (FrankenPHP untouched)
- `pv service:remove mail` → unregister + delete binary → `SignalDaemon()` → reconcile stops the supervised process; data directory preserved
- `pv service:destroy mail` → `remove` + delete `config.ServiceDataDir("mail", "latest")`

`buildSupervisorProcess` constructs:

- Binary path: `~/.pv/internal/bin/mailpit`
- Data dir: `~/.pv/services/mail/latest/data` (created via `os.MkdirAll` before spawn)
- Log file: `~/.pv/logs/mailpit.log`
- Args: `mailpit.Args(dataDir)` — see VERIFY note
- Ready: HTTP GET to `http://127.0.0.1:8025/livez`, expect 2xx, 30 s timeout

## Error Handling

All cases inherited from the rustfs spec's error table. Mail-specific additions:

| Failure | Where caught | Behavior |
|---------|-------------|----------|
| Port 1025 already in use (another SMTP server, or previous mailpit orphan) | `ReadyCheck` timeout | Supervisor kills spawned process; reconcile reports failure; user sees "port 1025 in use — check `lsof -i :1025`". |
| Pre-existing Docker-shaped `mail` entry | `service:add mail` | Error: "mail already registered (as docker). Run `pv uninstall && pv setup` to reset." No auto-migration, matching the rustfs decision. |
| Mailpit `.db` file corruption | Mailpit itself on startup | Mailpit typically rebuilds or errors at startup; surfaces in the log file. User can `pv service:destroy mail` to wipe and start fresh. |

## Testing Strategy

### Unit tests

- `internal/binaries/mailpit_test.go`:
  - URL + archive-name construction for every supported `(GOOS, GOARCH)`.
  - Error on unsupported platforms.
- `internal/services/mailpit_test.go`:
  - `Name() == "mail"`, `Port() == 1025`, `ConsolePort() == 8025`.
  - `EnvVars("anyproject")` returns the exact keys/values the current Docker `Mail` service returns (pin as a golden map so the migration doesn't silently change `.env` contents).
  - `WebRoutes()` returns a single `{Subdomain: "mail", Port: 8025}` entry.
  - `ReadyCheck()` returns an HTTP endpoint (not TCP), pointing at `/livez`.
  - `LookupBinary("mail")` finds it; the old Docker `Lookup("mail")` no longer does.

### Integration tests

No new integration tests beyond what already exists for the supervisor package and `reconcileBinaryServices()`. Adding Mailpit is covered by the E2E flow.

### E2E

`scripts/e2e/mail-binary.sh`:

```bash
pv start
pv service:add mail
# assert: ~/.pv/internal/bin/mailpit exists and is executable
# assert: daemon-status.json lists "mailpit" as running
# assert: HTTP GET http://127.0.0.1:8025/livez → 200
# assert: TCP connect to 127.0.0.1:1025 succeeds

pv service:stop mail
# assert: daemon-status.json no longer lists "mailpit" as running
# assert: HTTP 8025 no longer answers

pv service:start mail
# assert: back to running

pv service:destroy mail
# assert: registry no longer contains "mail"
# assert: ~/.pv/internal/bin/mailpit is gone
# assert: ~/.pv/services/mail/latest/data is gone

pv stop
```

Added as a new phase in `.github/workflows/e2e.yml` — runs after the rustfs-binary phase so we're exercising both in CI.

### Explicitly NOT tested

- Linux binary correctness — CI is macOS-only.
- Actually sending real mail through the SMTP port (just checking the port is bound). The existing Docker mail tests don't do this either.
- Web-UI functionality (Mailpit's own test suite covers that).

## Verification Items (before implementation starts)

1. Confirm `./mailpit --help` accepts `--smtp :1025`, `--listen :8025`, and `--database <path>`. If any flag name is different, fix `Mailpit.Args()`.
2. Confirm the extracted `.tar.gz` has `mailpit` at the archive root (not inside a subdirectory).
3. Confirm `/livez` endpoint exists on port 8025 in v1.29.6 — used by `ReadyCheck`.
4. Confirm the Mailpit binary is fully statically linked (macOS + Linux) by `otool -L` / `ldd` inspection. If it has dynamic dependencies, we may need to constrain supported OS versions.

## Deferred

- SMTP-over-TLS support (`--smtp-tls-cert` / `--smtp-tls-key`).
- Mailpit `--webhook` integration.
- Mail relay mode (forwarding captured mail to a real upstream SMTP server during dev).
- Structured log parsing beyond "tail the file".
- Migration from other existing mail-service entries (none today beyond the Docker one).
