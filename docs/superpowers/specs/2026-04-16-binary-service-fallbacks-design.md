# Binary Service Fallback Semantics

**Date:** 2026-04-16 (revised 2026-04-17)
**Status:** Approved

## Problem

After the rustfs and mailpit binary-service migrations, the `.env` fallback system (`applyFallbacksToLinkedProjects`) has three bugs:

1. **False positive (stop.go:65-68):** `pv service:stop` (no args) applies fallbacks to all registered services, including binary services it didn't actually stop. Binary services are skipped in the stop loop (line 38-42, "managed by the daemon") but NOT skipped in the fallback loop. Result: linked Laravel projects have `MAIL_MAILER=smtp → log` rewritten even though mail is still supervised and listening.

2. **Missing fallback + unbind on binary removal (remove.go / destroy.go binary paths):** `pv service:remove mail` and `pv service:destroy mail` permanently delete the binary but do NOT apply `.env` fallbacks to linked projects, and do NOT call `reg.UnbindService(name)` to clear the `ProjectServices.Mail` flag. The Docker `service:remove` path does both (remove.go:116-118). Result: linked projects keep `MAIL_MAILER=smtp` after mail is gone (next mail send fails), and `ProjectServices.Mail` stays set, leaking into `ProjectsUsingService`, `service:list`, and future fallback prompts.

3. **Missing fallback on Docker destroy (destroy.go:128-130):** Docker `service:destroy` calls `reg.UnbindService(svcName)` but does NOT call `applyFallbacksToLinkedProjects` first — unlike Docker `service:remove` which does both. Result: `pv service:destroy mysql` unbinds the project but leaves `DB_CONNECTION=mysql` in the linked `.env` instead of falling back to `sqlite`. This is a pre-existing Docker bug, not introduced by the binary migrations.

## Correct semantics

| Operation | Service gone? | Fallback? | Unbind? | Rationale |
|---|---|---|---|---|
| `pv service:stop` (no args) | Docker yes, Binary no | Docker yes, Binary **no** | No | Binary still supervised |
| `pv service:stop <name>` (named) | Yes (both kinds) | No | No | Transient — may come back |
| `pv service:remove <name>` | Yes (both kinds) | **Yes** | **Yes** | Permanent removal |
| `pv service:destroy <name>` | Yes (both kinds) | **Yes** | **Yes** | Permanent removal + data |

## Goals

- Fix the false positive: skip binary services in `stop.go`'s no-args fallback loop.
- Fix binary remove/destroy: add `applyFallbacksToLinkedProjects`, `reg.UnbindService`, and `reg.Save` to the binary paths so they match the Docker `service:remove` pattern.
- Fix Docker destroy: add `applyFallbacksToLinkedProjects` before the existing `reg.UnbindService` call so destroy matches remove.

## Non-goals

- Do not change Docker stop behavior (line 135-137 in `stop.go`). Docker `service:stop` currently applies fallbacks; changing that is a bigger semantic shift not requested by the user.
- Do not modify `applyFallbacksToLinkedProjects` itself — it's kind-agnostic and already works for binary service names (via `laravel.FallbackMapping`).
- Do not add fallback to the named binary `service:stop` path — user confirmed transient stops should not trigger fallback.

## Changes

Four surgical edits across three files. No new functions, no interface changes.

### Edit 1: `internal/commands/service/stop.go:65-68`

Skip binary services in the post-stop fallback loop.

Before:
```go
			// Apply fallbacks for each stopped service.
			for key := range reg.ListServices() {
				svcName, _ := services.ParseServiceKey(key)
				applyFallbacksToLinkedProjects(reg, svcName)
			}
```

After:
```go
			// Apply fallbacks for each stopped service.
			for key, inst := range reg.ListServices() {
				if inst.Kind == "binary" {
					continue // binary services were not stopped above; no fallback needed.
				}
				svcName, _ := services.ParseServiceKey(key)
				applyFallbacksToLinkedProjects(reg, svcName)
			}
```

### Edit 2: `internal/commands/service/remove.go` binary path

Add fallback + unbind after the daemon signal, before the success message. Mirrors the Docker path at remove.go:115-118 (`applyFallbacksToLinkedProjects` → `reg.UnbindService`).

Before (lines 67-72):
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
			// Apply env fallbacks and unbind from linked projects — the binary
			// is permanently gone. Mirrors the Docker path at remove.go:115-118.
			applyFallbacksToLinkedProjects(reg, name)
			reg.UnbindService(name)
			if err := reg.Save(); err != nil {
				return fmt.Errorf("cannot save registry: %w", err)
			}
			ui.Success(fmt.Sprintf("%s removed (data preserved)", binSvc.DisplayName()))
```

Note: `reg.Save()` is needed because `UnbindService` mutates the in-memory registry but does not persist — the existing `reg.Save()` at line 44-46 was called BEFORE the binary cleanup, so the unbind mutation must be saved separately. The Docker path avoids this because its unbind happens AFTER `reg.RemoveService` + `reg.Save` at lines 120-125, which saves both the removal and the unbind in one call. For the binary path, the service was already removed and saved at lines 41-46, so a second `reg.Save()` is required for the unbind.

### Edit 3: `internal/commands/service/destroy.go` binary path

Same pattern as Edit 2 — add fallback + unbind after the daemon signal.

Before (lines 71-76):
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
			// Apply env fallbacks and unbind from linked projects — the binary
			// is permanently gone. Mirrors the Docker path pattern.
			applyFallbacksToLinkedProjects(reg, name)
			reg.UnbindService(name)
			if err := reg.Save(); err != nil {
				return fmt.Errorf("cannot save registry: %w", err)
			}
			ui.Success(fmt.Sprintf("%s destroyed (binary + data gone)", binSvc.DisplayName()))
```

### Edit 4: `internal/commands/service/destroy.go` Docker path (~line 128)

Add `applyFallbacksToLinkedProjects` before the existing `reg.UnbindService` so Docker destroy matches Docker remove.

Before (lines 128-130):
```go
		// Unbind from all projects.
		projects := reg.ProjectsUsingService(svcName)
		reg.UnbindService(svcName)
```

After:
```go
		// Apply fallbacks and unbind from all projects.
		applyFallbacksToLinkedProjects(reg, svcName)
		projects := reg.ProjectsUsingService(svcName)
		reg.UnbindService(svcName)
```

## Testing

### New test: mail fallback integration (hooks_test.go)

Mirror the existing Redis integration test for mail. Register a mail binary service, set up a linked Laravel project with `MAIL_MAILER=smtp`, call `applyFallbacksToLinkedProjects(reg, "mail")`, assert `MAIL_MAILER=log`. This pins the `FallbackMapping("mail")` wiring that the binary remove/destroy paths now depend on.

### New test: stop-all skips binary fallback (hooks_test.go)

Test the control-flow guard directly: create a registry with both a Docker service and a binary service, each with a linked project whose `.env` has service-specific values. Simulate the stop-all fallback loop (iterate `reg.ListServices()`, skip `Kind == "binary"`, apply fallbacks for the rest). Assert the Docker service's project got fallback-rewritten but the binary service's project did NOT.

This tests the exact loop logic from `stop.go:65-68` in isolation without needing a Docker engine. The test extracts the loop body into a helper-like pattern using the same `inst.Kind == "binary"` check the production code uses.

### New test: unbind clears project binding (hooks_test.go)

Register a mail binary service, bind a project to it (`ProjectServices.Mail = true`), call `reg.UnbindService("mail")`, assert `ProjectServices.Mail == false`. Pins the unbind wiring that binary remove/destroy now depend on.

### Existing tests

- `TestApplyFallbacksToLinkedProjects_Integration` (Redis) — unaffected, continues to pass.
- `TestFallbackMapping_Mail` / `TestFallbackMapping_S3` in `internal/laravel/env_test.go` — unaffected.

## Verification items

1. `laravel.FallbackMapping("mail")` returns rules for `MAIL_MAILER` → `log`. Confirmed by `internal/laravel/env_test.go:125-134`.
2. `remove.go` binary path: `name := binSvc.Name()` is in scope at the insertion point (line 37). Confirmed.
3. `destroy.go` binary path: `name := binSvc.Name()` is in scope at the insertion point (line 38). Confirmed.
4. `destroy.go` Docker path: `svcName` is in scope at the insertion point (assigned at line 94 via `services.ParseServiceKey`). Confirmed.
5. `reg.UnbindService(name)` does not call `reg.Save()` internally — the caller must save. Confirmed by reading `internal/registry/registry.go`.
