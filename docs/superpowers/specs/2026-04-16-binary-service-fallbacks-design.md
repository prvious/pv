# Binary Service Fallback Semantics

**Date:** 2026-04-16
**Status:** Approved

## Problem

After the rustfs and mailpit binary-service migrations, the `.env` fallback system (`applyFallbacksToLinkedProjects`) has two bugs:

1. **False positive (stop.go:65-68):** `pv service:stop` (no args) applies fallbacks to all registered services, including binary services it didn't actually stop. Binary services are skipped in the stop loop (line 38-42, "managed by the daemon") but NOT skipped in the fallback loop. Result: linked Laravel projects have `MAIL_MAILER=smtp → log` rewritten even though mail is still supervised and listening.

2. **Missing fallback on removal (remove.go / destroy.go binary paths):** `pv service:remove mail` and `pv service:destroy mail` permanently delete the binary but do NOT apply `.env` fallbacks to linked projects. The Docker equivalents DO apply fallbacks. Result: linked projects keep `MAIL_MAILER=smtp` after mail is gone; the next mail send fails with no hint.

## Correct semantics

| Operation | Binary actually stopped? | Fallback? | Rationale |
|---|---|---|---|
| `pv service:stop` (no args) | No (skipped in loop) | No | Binary is still supervised |
| `pv service:stop mail` (named) | Yes (Enabled=false + daemon signal) | No | Transient — may come back via `service:start` |
| `pv service:remove mail` | Yes (binary deleted) | **Yes** | Permanent removal |
| `pv service:destroy mail` | Yes (binary + data deleted) | **Yes** | Permanent removal |

## Goals

- Fix the false positive: skip binary services in `stop.go`'s no-args fallback loop.
- Fix the missing fallback: add `applyFallbacksToLinkedProjects` calls to the binary paths of `remove.go` and `destroy.go`.

## Non-goals

- Do not change Docker stop behavior (line 135-137 in `stop.go`). Docker `service:stop` currently applies fallbacks; changing that is a bigger semantic shift not requested by the user.
- Do not modify `applyFallbacksToLinkedProjects` itself — it's kind-agnostic and already works for binary service names (via `laravel.FallbackMapping`).
- Do not add fallback to the named binary `service:stop` path — user confirmed transient stops should not trigger fallback.

## Changes

Three surgical edits across three files. No new functions, no interface changes.

### Edit 1: `internal/commands/service/stop.go:65-68`

Skip binary services in the post-stop fallback loop.

Before:
```go
for key := range reg.ListServices() {
    svcName, _ := services.ParseServiceKey(key)
    applyFallbacksToLinkedProjects(reg, svcName)
}
```

After:
```go
for key, inst := range reg.ListServices() {
    if inst.Kind == "binary" {
        continue // binary services were not stopped above; no fallback needed.
    }
    svcName, _ := services.ParseServiceKey(key)
    applyFallbacksToLinkedProjects(reg, svcName)
}
```

### Edit 2: `internal/commands/service/remove.go` binary path (~line 71)

Add fallback call before the success message.

Before:
```go
            if server.IsRunning() {
                if err := server.SignalDaemon(); err != nil {
                    ui.Subtle(fmt.Sprintf("Could not signal daemon: %v", err))
                }
            }
            ui.Success(fmt.Sprintf("%s removed (data preserved)", binSvc.DisplayName()))
```

After:
```go
            if server.IsRunning() {
                if err := server.SignalDaemon(); err != nil {
                    ui.Subtle(fmt.Sprintf("Could not signal daemon: %v", err))
                }
            }
            // Apply env fallbacks to linked projects — the binary is permanently
            // gone, so e.g. MAIL_MAILER should revert to "log".
            applyFallbacksToLinkedProjects(reg, name)
            ui.Success(fmt.Sprintf("%s removed (data preserved)", binSvc.DisplayName()))
```

### Edit 3: `internal/commands/service/destroy.go` binary path (~line 75)

Add fallback call before the success message (same pattern as remove).

Before:
```go
            if server.IsRunning() {
                if err := server.SignalDaemon(); err != nil {
                    ui.Subtle(fmt.Sprintf("Could not signal daemon: %v", err))
                }
            }
            ui.Success(fmt.Sprintf("%s destroyed (binary + data gone)", binSvc.DisplayName()))
```

After:
```go
            if server.IsRunning() {
                if err := server.SignalDaemon(); err != nil {
                    ui.Subtle(fmt.Sprintf("Could not signal daemon: %v", err))
                }
            }
            // Apply env fallbacks to linked projects — the binary is permanently
            // gone, so e.g. MAIL_MAILER should revert to "log".
            applyFallbacksToLinkedProjects(reg, name)
            ui.Success(fmt.Sprintf("%s destroyed (binary + data gone)", binSvc.DisplayName()))
```

## Testing

### New test: stop-all skips binary services

In `internal/commands/service/hooks_test.go`, add a test that:
1. Creates a registry with a binary service ("mail", Kind="binary") and a linked Laravel project whose `.env` has `MAIL_MAILER=smtp`.
2. Calls `applyFallbacksToLinkedProjects` with the same name — verifies it WOULD change `MAIL_MAILER` to `log` (the function itself works).
3. Separately, the stop-all fallback loop's `inst.Kind == "binary"` skip is a control-flow guard, not a function-level behavior. The unit test of the function itself confirms it's kind-agnostic; the skip is verified by reading the code.

The stop-all skip is hard to unit-test without running the full cobra command (needs Docker engine setup). A targeted code-review assertion is acceptable here — the fix is 2 lines (`if inst.Kind == "binary" { continue }`).

### New test: fallback works for mail service

In `hooks_test.go`, add a test mirroring the existing Redis one but for mail: register a mail binary service, set up a linked project with `MAIL_MAILER=smtp`, call `applyFallbacksToLinkedProjects(reg, "mail")`, assert `MAIL_MAILER=log`. This pins the `FallbackMapping("mail")` wiring for the remove/destroy paths.

### Existing tests

The existing `TestApplyFallbacksToLinkedProjects_Integration` (Redis) and `TestFallbackMapping_Mail` / `TestFallbackMapping_S3` in `internal/laravel/env_test.go` are unaffected and continue to pass.

## Verification items

1. Confirm `laravel.FallbackMapping("mail")` returns rules for `MAIL_MAILER` → `log`. Already confirmed by reading `internal/laravel/env_test.go:125-134`.
2. Confirm `remove.go` binary path has access to the `name` variable (service name, e.g., "mail") at the insertion point. Confirmed: `name := binSvc.Name()` at line 37.
3. Confirm `destroy.go` binary path has the same `name` variable. Confirmed: `name := binSvc.Name()` at line 38.
