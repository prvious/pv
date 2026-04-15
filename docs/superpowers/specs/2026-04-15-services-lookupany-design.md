# `services.LookupAny` — Unified Service Lookup Across Docker + Binary Registries

**Date:** 2026-04-15
**Status:** Approved

## Relationship to prior specs

This spec follows the rustfs and mailpit native-binary migrations. Those PRs introduced a second registry — `binaryRegistry` — alongside the existing Docker `registry` in `internal/services/`. The PR review for the mailpit migration identified six callsites that consult only the Docker registry via `services.Lookup` and silently mishandle any service that lives in `binaryRegistry`. This spec defines the helper and per-callsite fixes that close those gaps.

This spec is **purely additive plus targeted callsite fixes**. No interface changes to `Service` or `BinaryService`. No changes to the supervisor, the dispatcher, or the registry.

## Problem

After the rustfs (s3) and mailpit (mail) binary-service migrations, six callsites in the codebase still call `services.Lookup(name)` and treat a "name not in the Docker registry" error as "service does not exist." For names that live in `binaryRegistry` (currently `s3` and `mail`), this produces user-visible silent failures:

| Callsite | User-visible symptom |
|---|---|
| `cmd/install.go:61` | `pv install --with=service[mail]` errors out: "unknown service" |
| `cmd/setup.go:70-74` | Setup wizard's service multi-select hides mail and s3 entirely |
| `cmd/setup.go:252-256` | Even if a binary service is somehow selected, the post-wizard provisioning loop silently skips it |
| `cmd/doctor.go:621` | `pv doctor` reports a healthy mail or s3 service as "unknown service type — registry may be out of date" |
| `internal/commands/service/env.go:45` | `pv service:env` (no args) prints "Skipping unknown service \"mail\"" for each binary service |
| `internal/commands/service/env.go:71` | `pv service:env mail` returns a hard error |

These are all the same shape: a callsite that needs to read service metadata or behavior, fails open, and either omits the binary service or misreports it. None of them are in the `service:*` command path, which already routes through `internal/commands/service/dispatch.go`'s private `resolveKind` and handles binary services correctly.

## Goals

- Eliminate the six silent failures above so that any registered binary service is treated as a first-class citizen by `install`, `setup`, `doctor`, and `service:env`.
- Introduce a single small public helper (`services.LookupAny`) that consults both registries, so future callsites have an obvious path that doesn't replicate the bug.
- Keep the existing `services.Lookup` and `services.LookupBinary` functions untouched — they have callers (notably the `service:*` dispatcher and the `caddy` route generator) that intentionally consult one registry only.

## Non-Goals

- Do **not** refactor `internal/commands/service/dispatch.go`'s private `resolveKind` to delegate to `LookupAny`. `resolveKind` enforces an additional invariant (rejecting an attempt to register a binary service when a Docker-shaped registry entry of the same name already exists) that is irrelevant to the read-only callsites this spec targets. Collapsing them would weaken the dispatcher's contract.
- Do **not** address `internal/commands/service/stop.go:65-68`'s `applyFallbacksToLinkedProjects` issue. That callsite has a different bug shape (it iterates registered services and rewrites linked projects' `.env` files, which is wrong for binary services that are still supervised). It needs a kind-aware skip, not a `LookupAny` call. Tracked as a separate follow-up PR.
- Do **not** add a `String()` method to `Kind`. YAGNI; nothing currently formats `Kind` for display.
- Do **not** add E2E coverage of `pv setup` selecting mail in the wizard. Existing E2E `install.sh` exercises `pv setup` end-to-end without selecting binary services; expanding the fixtures is a separate effort.

## Verified facts

These were confirmed by reading the current code on `main` (commit `4a432f8`):

- `services.Lookup` consults only `registry`; `services.LookupBinary` consults only `binaryRegistry`. Both already exist and behave as documented.
- `services.Available()` returns the deduplicated union of names from both registries (added in the mailpit migration).
- `internal/commands/service/dispatch.go:24-25` already does `LookupBinary` first, then `Lookup` — the lookup-order convention this spec adopts.
- The error string returned by `services.Lookup` is `unknown service %q (available: %s)` formatted via `services.Available()`. Adopting the same string in `LookupAny` means callsites switching from `Lookup` to `LookupAny` produce identical error UX.
- Both `Service` and `BinaryService` interfaces have `Name() string` and `DisplayName() string` methods. They diverge on everything else: `Service.EnvVars(projectName, port)` vs `BinaryService.EnvVars(projectName)`; `Service.DefaultVersion()` exists while `BinaryService` has no version concept (always "latest").
- `service.RunAdd([]string{name})` (single-arg form) routes through `dispatch.go`'s `resolveKind` and correctly handles either kind. The post-wizard fix in `setup.go` can rely on this rather than calling `DefaultVersion()` first. **VERIFY during implementation** by reading `service.RunAdd`'s arg handling — fall back to passing `[name, "latest"]` if the single-arg form doesn't exist.

## Architecture

### New file

| Path | Purpose |
|------|---------|
| `internal/services/lookup.go` | `Kind` enum + `LookupAny` function (the entire public API for this PR) |
| `internal/services/lookup_test.go` | 4 unit tests pinning the function's contract |

### Modified files

| Path | Change |
|------|--------|
| `cmd/install.go` | Replace `Lookup` validation in `parseWith` with `LookupAny` |
| `cmd/install_test.go` | Add 3 cases covering `--with=service[s3]`, `--with=service[mail]`, `--with=service[mongodb]` |
| `cmd/setup.go` | Extract `buildServiceOptions()` helper using `LookupAny`; route binary kind in post-wizard loop |
| `cmd/setup_test.go` | Test `buildServiceOptions()` (new file if it doesn't already exist) |
| `cmd/doctor.go` | Use `LookupAny` to skip binary services in the per-service Docker check loop, with an explanatory comment |
| `internal/commands/service/env.go` | Branch on `kind` in both the loop and single-service paths; call the right `EnvVars` signature |
| `internal/commands/service/env_test.go` | New file: 3 cases covering Docker-only, binary-only, and mixed registry states |

No other files change. No interface methods are added or removed.

## Components

### `services.LookupAny` (`internal/services/lookup.go`)

```go
package services

import (
    "fmt"
    "strings"
)

type Kind int

const (
    KindUnknown Kind = iota
    KindDocker
    KindBinary
)

// LookupAny resolves a service name across both the Docker and binary
// registries.
//
// Lookup order is binary first, then Docker — matching the convention used
// by the service:* command dispatcher (internal/commands/service/dispatch.go).
// A name found in only one registry returns that kind. A name in neither
// registry returns KindUnknown plus a non-nil error whose text matches the
// Lookup error format so callsites switching from Lookup retain the same
// error UX.
//
// When kind != KindUnknown, exactly one of binSvc / docSvc is non-nil.
//
// For service:* commands that also need to enforce a no-collision invariant
// against an active registry.Registry, see resolveKind in
// internal/commands/service/dispatch.go.
func LookupAny(name string) (kind Kind, binSvc BinaryService, docSvc Service, err error) {
    if svc, ok := LookupBinary(name); ok {
        return KindBinary, svc, nil, nil
    }
    if svc, lookupErr := Lookup(name); lookupErr == nil {
        return KindDocker, nil, svc, nil
    }
    return KindUnknown, nil, nil, fmt.Errorf(
        "unknown service %q (available: %s)",
        name,
        strings.Join(Available(), ", "),
    )
}
```

### `cmd/install.go` change

Before:
```go
if _, err := services.Lookup(s.name); err != nil {
    return spec, fmt.Errorf("unknown service %q in --with (available: %s)",
        s.name, strings.Join(services.Available(), ", "))
}
```

After:
```go
if k, _, _, _ := services.LookupAny(s.name); k == services.KindUnknown {
    return spec, fmt.Errorf("unknown service %q in --with (available: %s)",
        s.name, strings.Join(services.Available(), ", "))
}
```

Note: keep the existing `unknown service %q in --with` wrapper because it adds the `--with` context that's useful in the install command. Discard the inner error from `LookupAny` since its text would be redundant once wrapped.

### `cmd/setup.go` changes

Two changes in this file.

**Change 1: Extract `buildServiceOptions()` helper.**

Before (inline at line 69-75):
```go
var svcOpts []selectOption
for _, name := range services.Available() {
    svc, _ := services.Lookup(name)
    if svc != nil {
        svcOpts = append(svcOpts, selectOption{label: svc.DisplayName(), value: name})
    }
}
```

After (inline call):
```go
svcOpts := buildServiceOptions()
```

New helper at the bottom of the file (or in a dedicated `setup_options.go` if `setup.go` is already large):

```go
// buildServiceOptions returns the wizard's service multi-select options.
// Both Docker and binary services are listed using their DisplayName.
func buildServiceOptions() []selectOption {
    names := services.Available()
    out := make([]selectOption, 0, len(names))
    for _, name := range names {
        kind, binSvc, docSvc, err := services.LookupAny(name)
        if err != nil {
            // Available() is the union of both registries; LookupAny over
            // the same names should never miss. If it does, skip silently
            // rather than error the wizard.
            continue
        }
        var label string
        switch kind {
        case services.KindBinary:
            label = binSvc.DisplayName()
        case services.KindDocker:
            label = docSvc.DisplayName()
        }
        out = append(out, selectOption{label: label, value: name})
    }
    return out
}
```

**Change 2: Route binary kind in post-wizard provisioning.**

Before (line 251-262):
```go
for _, name := range selectedServices {
    svc, _ := services.Lookup(name)
    if svc == nil {
        continue
    }
    svcArgs := []string{name, svc.DefaultVersion()}
    if err := service.RunAdd(svcArgs); err != nil {
        if !errors.Is(err, ui.ErrAlreadyPrinted) {
            ui.Fail(fmt.Sprintf("Service %s failed: %v", name, err))
        }
    }
}
```

After:
```go
for _, name := range selectedServices {
    kind, _, docSvc, err := services.LookupAny(name)
    if err != nil {
        ui.Fail(fmt.Sprintf("Service %s: %v", name, err))
        continue
    }
    var svcArgs []string
    switch kind {
    case services.KindBinary:
        // Binary services are always "latest"; service:add ignores any
        // explicit version for binary kinds.
        svcArgs = []string{name}
    case services.KindDocker:
        svcArgs = []string{name, docSvc.DefaultVersion()}
    }
    if err := service.RunAdd(svcArgs); err != nil {
        if !errors.Is(err, ui.ErrAlreadyPrinted) {
            ui.Fail(fmt.Sprintf("Service %s failed: %v", name, err))
        }
    }
}
```

If `service.RunAdd` requires a two-element slice for both kinds, pass `[]string{name, "latest"}` for the binary case instead. Confirm during implementation.

### `cmd/doctor.go` change

Before (line 616-633):
```go
for key, svc := range svcs {
    svcName, version := services.ParseServiceKey(key)

    status := "unknown"
    if engine != nil {
        svcDef, lookupErr := services.Lookup(svcName)
        if lookupErr != nil {
            status = "lookup_error"
        } else {
            running, runErr := engine.IsRunning(...)
            ...
        }
    }
    ...
}
```

After:
```go
for key, svc := range svcs {
    svcName, version := services.ParseServiceKey(key)

    // Skip binary services: they have no Docker container to probe.
    // Binary supervision health is reported via `pv service:list` and
    // `pv service:status`, which read the daemon's status snapshot
    // directly. Including them here would couple doctor to the daemon's
    // internal status format.
    if kind, _, _, _ := services.LookupAny(svcName); kind == services.KindBinary {
        continue
    }

    status := "unknown"
    if engine != nil {
        svcDef, lookupErr := services.Lookup(svcName)
        if lookupErr != nil {
            status = "lookup_error"
        } else {
            ...
        }
    }
    ...
}
```

The `lookup_error` branch becomes effectively unreachable for currently-known service names, but keep it as a guard against the (impossible-today) case of a registry entry whose name was never registered in either map. It now correctly indicates "registry is genuinely corrupt," matching the existing error text.

### `internal/commands/service/env.go` changes

Both call sites get the same kind-branch shape.

Before (loop at line 43-52):
```go
for key, instance := range svcs {
    svcName, _ := services.ParseServiceKey(key)
    svc, err := services.Lookup(svcName)
    if err != nil {
        ui.Subtle(fmt.Sprintf("Skipping unknown service %q", svcName))
        continue
    }
    envVars := svc.EnvVars(projectName, instance.Port)
    printEnvVars(key, envVars)
}
```

After:
```go
for key, instance := range svcs {
    svcName, _ := services.ParseServiceKey(key)
    kind, binSvc, docSvc, err := services.LookupAny(svcName)
    if err != nil {
        ui.Subtle(fmt.Sprintf("Skipping unknown service %q", svcName))
        continue
    }
    var envVars map[string]string
    switch kind {
    case services.KindBinary:
        envVars = binSvc.EnvVars(projectName)
    case services.KindDocker:
        envVars = docSvc.EnvVars(projectName, instance.Port)
    }
    printEnvVars(key, envVars)
}
```

Single-service path (line 70-78) gets the same branch. No change to `printEnvVars`.

## Data Flow

```
┌──────────────────┐      ┌─────────────────────┐
│  any callsite    │ ───▶ │  services.LookupAny │
└──────────────────┘      └──────────┬──────────┘
                                     │
                          ┌──────────┴──────────┐
                          │  binaryRegistry?    │ yes ──▶ KindBinary, *binSvc, nil, nil
                          └──────────┬──────────┘
                                     │ no
                          ┌──────────┴──────────┐
                          │  registry?          │ yes ──▶ KindDocker, nil, *docSvc, nil
                          └──────────┬──────────┘
                                     │ no
                                     ▼
                            KindUnknown, nil, nil, err
```

No I/O, no state, no goroutine concerns. Two map reads and an `fmt.Errorf` on the failure path.

## Error Handling

| Failure | Where caught | Behavior |
|---|---|---|
| Name not in either registry | `LookupAny` | Returns `KindUnknown` + error matching the existing `Lookup` text. Callers either propagate (with their own context wrapping) or print `ui.Subtle` and continue, preserving today's UX. |
| `Available()` lists a name that `LookupAny` then fails to find | Theoretically impossible; both consult the same maps. If it happens (e.g. concurrent registry mutation, which the codebase doesn't currently do), the wizard skips silently and continues — same fail-soft posture as the existing inline loop. |
| Caller forgets to check `kind` and dereferences a nil `binSvc` or `docSvc` | Caller bug — would panic. Mitigated by always pairing `LookupAny` with a `switch kind` block in the spec's example code. Tests cover the convention. |

No new failure modes are introduced. The bugs being fixed all manifest as "silent skip" or "wrong error text"; after this PR, the same code paths produce correct output.

## Testing Strategy

### Unit tests

**`internal/services/lookup_test.go`** — 4 cases:

- `TestLookupAny_BinaryService` — `LookupAny("mail")` returns `(KindBinary, *Mailpit, nil, nil)`.
- `TestLookupAny_DockerService` — `LookupAny("mysql")` returns `(KindDocker, nil, *MySQL, nil)`.
- `TestLookupAny_Unknown` — `LookupAny("mongodb")` returns `(KindUnknown, nil, nil, err)`; assert error text contains `unknown service "mongodb"` and `available:`.
- `TestLookupAny_BinaryWinsOnCollision` — temporarily seed both registries with the same key (use `t.Cleanup` to restore), assert binary wins.

**`cmd/install_test.go`** — extend with 3 cases:

- `parseWith("service[s3]")` succeeds.
- `parseWith("service[mail]")` succeeds.
- `parseWith("service[mongodb]")` errors with text containing `unknown service`.

**`cmd/setup_test.go`** — new tests for `buildServiceOptions()`:

- All known service names appear in the returned slice with their `DisplayName()` as the label.
- Both binary services (`s3`, `mail`) and Docker services (`mysql`, `postgres`, `redis`) are present.

**`cmd/doctor_test.go` (or wherever doctor tests live)** — 1 test:

- After registering a binary service in a temp registry, the doctor check output does not contain a "lookup_error" or "registry may be out of date" entry for it.

If `cmd/doctor.go` does not currently have a tested entry-point, defer this test to the integration / E2E level rather than introduce one ad hoc.

**`internal/commands/service/env_test.go`** — new file, 3 cases:

- All-services path with a registry containing only Docker services prints all of them.
- All-services path with a registry containing only binary services prints all of them.
- Single-service path: `pv service:env mail` returns the correct `MAIL_*` map.

### Integration tests

None required. The fixed callsites are not currently covered by integration tests at the daemon level; the unit tests cover the behavior changes.

### E2E

No new E2E phase. The existing `install.sh` exercises `pv setup` and would catch a regression that breaks the wizard for Docker services. Adding a fixture that selects mail in the wizard is a separate effort.

### Explicitly NOT tested

- Multi-version binary services (none exist; YAGNI).
- Concurrent registration into either registry from multiple `init()`s (would be a programmer error, not a runtime input).

## Verification Items (before implementation starts)

1. Confirm `service.RunAdd([]string{name})` (single-arg form) routes through `resolveKind` correctly for binary services. If only the two-arg form is supported, pass `[]string{name, "latest"}` for binary in `setup.go`'s post-wizard loop.

Confirmed by inspection on 2026-04-15:

- `cmd/setup_test.go` does not exist — create it.
- `internal/commands/service/env_test.go` does not exist — create it.
- `cmd/doctor_test.go` exists with a `newDoctorCmd()` scaffold and a `TestDoctor_EmptyHome` pattern using `t.TempDir()` + `t.Setenv("HOME", ...)`. Reuse the scaffold; add the new test alongside the existing ones.

## Deferred

- **`internal/commands/service/stop.go:65-68` `applyFallbacksToLinkedProjects` issue.** Different bug shape (rewrites linked projects' `.env` files for binary services that are still supervised). Tracked as a separate follow-up PR.
- **`ReadyCheck` sum-type tightening** (rejecting invalid `{}` and `{TCPPort: x, HTTPEndpoint: y}` states at construction time). Type-design improvement noted in the mailpit PR review; touches both `RustFS` and `Mailpit`. Separate PR.
- **`TestBuildSupervisorProcess_Mailpit`** — closes the per-service supervisor wiring test gap noted in the mailpit PR review. Separate PR.
- **E2E coverage that selects mail in `setup.sh` wizard fixtures.** Separate effort.
- **Refactor of `internal/commands/service/dispatch.go`'s `resolveKind` to delegate to `LookupAny`.** Considered; rejected because `resolveKind` enforces a no-collision invariant that `LookupAny` doesn't, and collapsing them would weaken the dispatcher's contract.
