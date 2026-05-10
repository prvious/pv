# Drop the `services` namespace; promote rustfs and mailpit Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Retire `internal/services/` and `internal/svchooks/`. Give rustfs and mailpit self-contained `internal/<tool>/` packages mirroring redis/postgres/mysql. Remove the dead `registry.ServiceInstance.Kind` field. Zero user-visible change.

**Architecture:** Strict mirror of redis/pg/mysql packaging — no shared `BinaryService` interface, no polymorphic registry. Cross-cutting callers (`server/manager.go`, `caddy/caddy.go`, `laravel/env.go`, `cmd/{install,setup,update}.go`) become explicit per-tool calls / switches. Stable cross-cutting helpers (`ReadDotEnv`, `SanitizeProjectName`, `MergeDotEnv`) move into a new single-purpose package `internal/projectenv/`. `WebRoute` moves to `internal/caddy/` (its only consumer); `ReadyCheck`/`HTTPReady`/`TCPReady` move to `internal/supervisor/` (their natural consumer).

**Tech Stack:** Go 1.22+, cobra/CLI, no new external dependencies.

**Spec:** `docs/superpowers/specs/2026-05-10-drop-services-namespace-design.md`

---

## Notes for the implementer

- After every code-modifying step, run `gofmt -w .` before checking compilation. The CLAUDE.md style rules apply.
- Keep imports alphabetically ordered within each group (stdlib, then external) — `gofmt` does not sort imports.
- "Verify build" means: `go build ./...` returns 0; "verify tests" means `go test ./...` returns 0.
- When moving a function, also move its tests to the same destination package in the same task.
- Use `git mv` when moving a single file 1:1 to preserve history; use `Write` + delete when you must split a file.
- Each task ends with a commit. Use the message style from recent history (e.g. `refactor(services): move ReadDotEnv to projectenv`). No "co-authored" trailers.

---

## File map (where things end up)

| Current | New | Notes |
|---|---|---|
| `internal/services/dotenv.go` | `internal/projectenv/dotenv.go` | `ReadDotEnv`, `MergeDotEnv` |
| `internal/services/dotenv_test.go` | `internal/projectenv/dotenv_test.go` | unchanged tests, retargeted package |
| `internal/services/service.go` (`SanitizeProjectName`) | `internal/projectenv/sanitize.go` | one function + its tests |
| `internal/services/service.go` (`ServiceKey`/`ParseServiceKey`) | `internal/registry/keys.go` | versioned-key parsing |
| `internal/services/service.go` (`WebRoute`) | `internal/caddy/webroute.go` | caddy is the only consumer |
| `internal/services/service.go` (`Available`/`Lookup`) | (deleted) | polymorphic enumeration is gone |
| `internal/services/binary.go` (`ReadyCheck`, `HTTPReady`, `TCPReady`) | `internal/supervisor/readycheck.go` | produces `func(ctx) error` |
| `internal/services/binary.go` (`BinaryService` interface, `binaryRegistry`, `LookupBinary`, `AllBinary`) | (deleted) | no shared interface |
| `internal/services/rustfs.go` | `internal/rustfs/service.go` (subset) | constants + supervisor process |
| `internal/services/mailpit.go` | `internal/mailpit/service.go` (subset) | constants + supervisor process |
| `internal/svchooks/install.go` (rustfs path) | `internal/rustfs/install.go` | `Install()` (no `BinaryService` arg) |
| `internal/svchooks/install.go` (mailpit path) | `internal/mailpit/install.go` | `Install()` |
| `internal/svchooks/update.go` | `internal/{rustfs,mailpit}/update.go` | `Update()` |
| `internal/svchooks/uninstall.go` | `internal/{rustfs,mailpit}/uninstall.go` | `Uninstall(deleteData bool)` |
| `internal/svchooks/enable.go` | `internal/{rustfs,mailpit}/enable.go` | `SetEnabled(bool)` |
| `internal/svchooks/restart.go` | `internal/{rustfs,mailpit}/restart.go` | `Restart()` |
| `internal/svchooks/status.go` | `internal/{rustfs,mailpit}/status.go` | `PrintStatus()` |
| `internal/svchooks/logs.go` | `internal/{rustfs,mailpit}/logs.go` | `TailLog(ctx, follow)` |
| `internal/svchooks/wait.go` | `internal/{rustfs,mailpit}/wait.go` | `WaitStopped(timeout)` |
| `internal/svchooks/svchooks.go` (`UpdateLinkedProjectsEnvBinary`) | `internal/{rustfs,mailpit}/env.go` | per-tool env-update helper |
| `internal/svchooks/svchooks.go` (`BindBinaryServiceToAllProjects`) | `internal/{rustfs,mailpit}/bind.go` | per-tool retroactive bind |
| `internal/svchooks/svchooks.go` (`ApplyFallbacksToLinkedProjects`) | `internal/{rustfs,mailpit}/fallback.go` | per-tool fallback hook |
| `internal/server/binary_service.go` | (deleted) | replaced by per-tool `BuildSupervisorProcess()` |
| `internal/server/binary_service_test.go` (ready-check tests) | `internal/supervisor/readycheck_test.go` | tests follow the helpers |
| `internal/laravel/env.go` (`UpdateProjectEnvForBinaryService`) | (deleted) | replaced by per-tool calls |

After all tasks, `internal/services/` and `internal/svchooks/` are empty and removed.

---

## Task 1: Create `internal/projectenv/`; move `ReadDotEnv` and `MergeDotEnv`

**Files:**
- Create: `internal/projectenv/dotenv.go` (content of `internal/services/dotenv.go`, package renamed)
- Create: `internal/projectenv/dotenv_test.go` (content of `internal/services/dotenv_test.go`, package renamed)
- Modify: `cmd/link.go` — `services.ReadDotEnv` → `projectenv.ReadDotEnv`
- Modify: `internal/automation/steps/detect_services.go` — `services.ReadDotEnv` → `projectenv.ReadDotEnv`, `services.SanitizeProjectName` stays calling `services.SanitizeProjectName` for now (Task 2 moves it)
- Modify: `internal/commands/postgres/install.go` — `services.ReadDotEnv` → `projectenv.ReadDotEnv`
- Modify: `internal/commands/mysql/install.go` — `services.ReadDotEnv` → `projectenv.ReadDotEnv`
- Modify: `internal/laravel/env.go` — replace `services.ReadDotEnv` and `services.MergeDotEnv` with `projectenv.ReadDotEnv`/`projectenv.MergeDotEnv`
- Modify: `internal/laravel/steps.go` — replace 3 `services.MergeDotEnv` calls with `projectenv.MergeDotEnv`
- Modify: `internal/redis/database_test.go` — `services.MergeDotEnv` → `projectenv.MergeDotEnv`
- Modify: `internal/services/dotenv.go` — delete (file removed at end of task)
- Modify: `internal/services/dotenv_test.go` — delete

- [ ] **Step 1.1: Create `internal/projectenv/dotenv.go` with the existing helpers**

```go
// internal/projectenv/dotenv.go
package projectenv

import (
	"os"
	"strings"
)

// ReadDotEnv reads a .env file into a map of key=value pairs.
func ReadDotEnv(path string) (map[string]string, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	result := make(map[string]string)
	for _, line := range strings.Split(string(data), "\n") {
		line = strings.TrimSpace(line)
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		parts := strings.SplitN(line, "=", 2)
		if len(parts) == 2 {
			result[parts[0]] = parts[1]
		}
	}
	return result, nil
}

// MergeDotEnv reads an existing .env file, replaces matching keys in-place,
// appends new keys, and writes the result. Creates a backup at backupPath.
func MergeDotEnv(envPath, backupPath string, newVars map[string]string) error {
	existing, err := os.ReadFile(envPath)
	if err != nil && !os.IsNotExist(err) {
		return err
	}

	if err == nil && backupPath != "" {
		if err := os.WriteFile(backupPath, existing, 0644); err != nil {
			return err
		}
	}

	replaced := make(map[string]bool)
	var lines []string

	if len(existing) > 0 {
		for _, line := range strings.Split(string(existing), "\n") {
			trimmed := strings.TrimSpace(line)
			if trimmed != "" && !strings.HasPrefix(trimmed, "#") {
				parts := strings.SplitN(trimmed, "=", 2)
				if len(parts) == 2 {
					key := parts[0]
					if val, ok := newVars[key]; ok {
						lines = append(lines, key+"="+val)
						replaced[key] = true
						continue
					}
				}
			}
			lines = append(lines, line)
		}
	}

	for key, val := range newVars {
		if !replaced[key] {
			lines = append(lines, key+"="+val)
		}
	}

	content := strings.Join(lines, "\n")
	if !strings.HasSuffix(content, "\n") {
		content += "\n"
	}

	return os.WriteFile(envPath, []byte(content), 0644)
}
```

- [ ] **Step 1.2: Create `internal/projectenv/dotenv_test.go`**

Copy `internal/services/dotenv_test.go` verbatim, change `package services` → `package projectenv`, and update internal references (`ReadDotEnv` / `MergeDotEnv` are unqualified within the package — should compile unchanged after the rename).

- [ ] **Step 1.3: Run the projectenv tests**

```bash
go test ./internal/projectenv/
```
Expected: PASS (all tests previously in `services/dotenv_test.go`).

- [ ] **Step 1.4: Update each caller (one-shot find/replace)**

In each of these files, swap the import `"github.com/prvious/pv/internal/services"` for `"github.com/prvious/pv/internal/projectenv"` (keeping `services` import only if other `services.*` calls remain in the file), and replace `services.ReadDotEnv(` → `projectenv.ReadDotEnv(`, `services.MergeDotEnv(` → `projectenv.MergeDotEnv(`:

- `cmd/link.go` (1 call site at link.go:122)
- `internal/automation/steps/detect_services.go` (1 site at line 35)
- `internal/commands/postgres/install.go` (1 site at line 88)
- `internal/commands/mysql/install.go` (1 site at line 83)
- `internal/laravel/env.go` (sites at lines 69, 83, 99, 119, 139, 157)
- `internal/laravel/steps.go` (3 sites at lines 111, 144, 253)
- `internal/redis/database_test.go` (1 site at line 19)

Several of these still need the `services` import for other calls (e.g., `laravel/env.go` still references `services.BinaryService` until later tasks). Keep both imports temporarily where required.

- [ ] **Step 1.5: Delete the old files**

```bash
rm internal/services/dotenv.go internal/services/dotenv_test.go
```

- [ ] **Step 1.6: Verify build + tests**

```bash
gofmt -w . && go vet ./... && go build ./... && go test ./...
```
Expected: all green. If `internal/services/` shows an unused-import warning for `os`/`strings`, remove the unused imports from the remaining files in that package (likely none, since dotenv.go was the only file using them).

- [ ] **Step 1.7: Commit**

```bash
git add -A
git commit -m "refactor(services): extract ReadDotEnv/MergeDotEnv into projectenv"
```

---

## Task 2: Move `SanitizeProjectName` to `internal/projectenv/`

**Files:**
- Create: `internal/projectenv/sanitize.go`
- Create: `internal/projectenv/sanitize_test.go` (split out of `internal/services/service_test.go`)
- Modify: `internal/automation/steps/detect_services.go` — `services.SanitizeProjectName` → `projectenv.SanitizeProjectName`
- Modify: `internal/postgres/envvars.go` — update doc comment that references `services.SanitizeProjectName` (non-functional)
- Modify: `internal/mysql/envvars.go` — update doc comment that references `services.SanitizeProjectName` (non-functional)
- Modify: `internal/services/service.go` — remove `SanitizeProjectName` and the `safeIdentifier` regex
- Modify: `internal/services/service_test.go` — remove `SanitizeProjectName` tests (kept tests for `ServiceKey`/`ParseServiceKey` for now, deleted in Task 3)

- [ ] **Step 2.1: Create `internal/projectenv/sanitize.go`**

```go
// internal/projectenv/sanitize.go
package projectenv

import (
	"regexp"
	"strings"
)

var safeIdentifier = regexp.MustCompile(`[^a-zA-Z0-9_]`)

// SanitizeProjectName converts a directory name to a database-safe identifier.
// Only alphanumeric characters and underscores are kept; everything else is stripped.
func SanitizeProjectName(name string) string {
	name = strings.ReplaceAll(name, "-", "_")
	return safeIdentifier.ReplaceAllString(name, "")
}
```

- [ ] **Step 2.2: Create `internal/projectenv/sanitize_test.go`**

Look at `internal/services/service_test.go` and identify the test functions that exercise `SanitizeProjectName`. Copy those test functions verbatim into `internal/projectenv/sanitize_test.go`, change `package services` → `package projectenv`, and remove any references not related to sanitize. Leave the other tests (`ServiceKey`/`ParseServiceKey`) in `services/service_test.go` for Task 3 to handle.

- [ ] **Step 2.3: Run the projectenv tests**

```bash
go test ./internal/projectenv/ -run TestSanitize -v
```
Expected: PASS.

- [ ] **Step 2.4: Update the one production caller**

In `internal/automation/steps/detect_services.go` line 45, replace `services.SanitizeProjectName(ctx.ProjectName)` with `projectenv.SanitizeProjectName(ctx.ProjectName)`. Add `"github.com/prvious/pv/internal/projectenv"` to imports if not already present (Task 1 likely added it).

- [ ] **Step 2.5: Update doc comments**

Edit `internal/postgres/envvars.go` line 6 and `internal/mysql/envvars.go` line 7: replace `services.SanitizeProjectName` with `projectenv.SanitizeProjectName` in the comment text. No code changes — these comments document the contract.

- [ ] **Step 2.6: Remove `SanitizeProjectName` from `services`**

In `internal/services/service.go`, delete the `safeIdentifier` regex, the `SanitizeProjectName` function, and the `regexp` import. The file should still contain `WebRoute`, `Available`, `Lookup`, `ServiceKey`, `ParseServiceKey` — those move in later tasks.

In `internal/services/service_test.go`, remove the test functions covering `SanitizeProjectName` (already copied to projectenv in Step 2.2).

- [ ] **Step 2.7: Verify build + tests**

```bash
gofmt -w . && go vet ./... && go build ./... && go test ./...
```
Expected: all green.

- [ ] **Step 2.8: Commit**

```bash
git add -A
git commit -m "refactor(services): move SanitizeProjectName to projectenv"
```

---

## Task 3: Move `ServiceKey` / `ParseServiceKey` to `internal/registry/`

**Files:**
- Create: `internal/registry/keys.go`
- Create: `internal/registry/keys_test.go` (port the tests from `internal/services/service_test.go`)
- Modify: `internal/services/service.go` — remove `ServiceKey`, `ParseServiceKey`
- Modify: `internal/services/service_test.go` — remove their tests
- Modify: any callers (grep first to confirm scope)

- [ ] **Step 3.1: Confirm no external callers (or list them)**

```bash
grep -rn "services\.\(ServiceKey\|ParseServiceKey\)" --include="*.go" .
```
Record any hits. (If none, the helpers are internal to `services` and the move is purely a relocation; if hits exist, each is a callsite update in this task.)

- [ ] **Step 3.2: Create `internal/registry/keys.go`**

```go
// internal/registry/keys.go
package registry

import "strings"

// ServiceKey returns the registry key for a service instance.
// For versioned services: "mysql:8.0.32". For unversioned: "redis".
func ServiceKey(name, version string) string {
	if version == "" || version == "latest" {
		return name
	}
	return name + ":" + version
}

// ParseServiceKey splits a registry key into service name and version.
// For "mysql:8.4" returns ("mysql", "8.4"). For "redis" returns ("redis", "latest").
func ParseServiceKey(key string) (name, version string) {
	if idx := strings.Index(key, ":"); idx > 0 {
		return key[:idx], key[idx+1:]
	}
	return key, "latest"
}
```

- [ ] **Step 3.3: Port the tests**

In `internal/services/service_test.go`, find the test functions covering `ServiceKey` / `ParseServiceKey`. Copy them verbatim to `internal/registry/keys_test.go`, change `package services` → `package registry`, and remove from the source file.

- [ ] **Step 3.4: Update callers (if any from Step 3.1)**

For each callsite from Step 3.1, replace `services.ServiceKey(...)` → `registry.ServiceKey(...)` and `services.ParseServiceKey(...)` → `registry.ParseServiceKey(...)`. Add the `internal/registry` import if missing.

- [ ] **Step 3.5: Remove from `services`**

In `internal/services/service.go`, delete `ServiceKey` and `ParseServiceKey`. Remove the unused `strings` import if no other function in the file needs it.

- [ ] **Step 3.6: Verify build + tests**

```bash
gofmt -w . && go vet ./... && go build ./... && go test ./...
```
Expected: all green.

- [ ] **Step 3.7: Commit**

```bash
git add -A
git commit -m "refactor(services): move ServiceKey/ParseServiceKey to registry"
```

---

## Task 4: Move `WebRoute` to `internal/caddy/`

**Files:**
- Create: `internal/caddy/webroute.go`
- Modify: `internal/services/service.go` — delete the `WebRoute` type
- Modify: `internal/services/{rustfs,mailpit}.go` — change `[]WebRoute` to `[]caddy.WebRoute` in `WebRoutes()`. (Both files acquire a `caddy` import.)
- Modify: any other callers — grep below

- [ ] **Step 4.1: Confirm scope**

```bash
grep -rn "services\.WebRoute\b" --include="*.go" .
grep -rn "WebRoute" --include="*.go" internal/services/ internal/caddy/ internal/svchooks/
```
Record callsites. Expected: rustfs.go, mailpit.go, service.go (defining), svchooks/install.go (in `PrintConnectionDetails`), caddy/caddy.go (the lookup at line ~371 uses `binSvc.WebRoutes()` which returns `[]services.WebRoute`).

- [ ] **Step 4.2: Create `internal/caddy/webroute.go`**

```go
// internal/caddy/webroute.go
package caddy

// WebRoute maps a subdomain under pv.{tld} to a local port.
// For example, {Subdomain: "s3", Port: 9001} routes s3.pv.test → 127.0.0.1:9001.
type WebRoute struct {
	Subdomain string
	Port      int
}
```

- [ ] **Step 4.3: Update the two service files**

In `internal/services/rustfs.go`, replace the function signature and body so it returns `[]caddy.WebRoute`:

```go
import (
	"time"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/caddy"
)

func (r *RustFS) WebRoutes() []caddy.WebRoute {
	return []caddy.WebRoute{
		{Subdomain: "s3", Port: 9001},
		{Subdomain: "s3-api", Port: 9000},
	}
}
```

Do the same in `internal/services/mailpit.go`:

```go
func (m *Mailpit) WebRoutes() []caddy.WebRoute {
	return []caddy.WebRoute{
		{Subdomain: "mail", Port: 8025},
	}
}
```

- [ ] **Step 4.4: Update the `BinaryService` interface**

In `internal/services/binary.go`, change the interface method:

```go
WebRoutes() []caddy.WebRoute
```

Add `"github.com/prvious/pv/internal/caddy"` to the imports.

**Watch for:** if this introduces a `caddy → services → caddy` import cycle, fix by moving the `caddy.WebRoute` import (caddy/caddy.go uses `services.LookupBinary` today). Verify with `go build ./...` after Step 4.5.

- [ ] **Step 4.5: Update `svchooks/install.go` (`PrintConnectionDetails`)**

The function iterates `svc.WebRoutes()`. Since the slice element type changed, no code change is needed inside the loop body — the field accesses (`route.Subdomain`, `route.Port`) are identical on `caddy.WebRoute`.

- [ ] **Step 4.6: Update `caddy/caddy.go`**

In `internal/caddy/caddy.go` around line 371 the code is:
```go
binSvc, ok := services.LookupBinary(svcName)
if !ok {
    continue
}
routes := binSvc.WebRoutes()
```
The returned slice is now `[]caddy.WebRoute` — same package, so no qualifier needed inside `caddy/caddy.go`. No code change required.

- [ ] **Step 4.7: Remove the original type**

In `internal/services/service.go`, delete the `WebRoute` struct definition.

- [ ] **Step 4.8: Verify**

```bash
gofmt -w . && go vet ./... && go build ./... && go test ./...
```

If `go build` reports an import cycle (`caddy → services` and `services → caddy`), check whether `caddy/caddy.go` still imports `services` for the `LookupBinary` call. It does — and that's fine: the cycle is `services → caddy` (for `WebRoute`) and `caddy → services` (for `LookupBinary`). **This will fail.** Fix path: skip Step 4.4 — leave the `BinaryService` interface signature as `WebRoutes() []caddy.WebRoute` only after `services.LookupBinary` is no longer called from `caddy/caddy.go`. Defer the interface signature change to Task 9 (which removes the `caddy → services` edge).

So **revise Step 4.4**: keep the local type in `services` for now (rename to `webRoute` lowercase to make it clearly internal? No — leave the struct in place but **also** add `caddy.WebRoute` as the canonical type. Have the interface return `[]caddy.WebRoute`. Make `services.WebRoute` a type alias: `type WebRoute = caddy.WebRoute`. This breaks the cycle because the alias only resolves at usage sites). Use this two-step approach:

```go
// internal/services/service.go (transitional)
package services

import "github.com/prvious/pv/internal/caddy"

// WebRoute is an alias for caddy.WebRoute kept during the services-package wind-down.
// Removed entirely in Task 12.
type WebRoute = caddy.WebRoute
```

The cycle still exists structurally (`services` imports `caddy`, `caddy` imports `services`). Confirm by running `go build ./...`.

If still cyclic, abandon Step 4 in this position and **defer the entire WebRoute move to Task 9** where `caddy → services` is removed first. In that case: revert all `WebRoute → caddy.WebRoute` changes in this task, leave `services.WebRoute` defined as it is, and skip to Task 5. Note this in the commit message and revisit during Task 9.

- [ ] **Step 4.9: Commit (only if Step 4.8 passes; otherwise revert and note in Task 9)**

```bash
git add -A
git commit -m "refactor(services): extract WebRoute to caddy package"
```

---

## Task 5: Move `ReadyCheck` / `HTTPReady` / `TCPReady` to `internal/supervisor/`

**Files:**
- Create: `internal/supervisor/readycheck.go`
- Create: `internal/supervisor/readycheck_test.go` (port from `internal/server/binary_service_test.go`)
- Modify: `internal/services/binary.go` — keep type aliases until Task 12 (cycle: `services → supervisor → services`? No — supervisor doesn't import services today. Verify.)
- Modify: `internal/services/{rustfs,mailpit}.go` — change `services.ReadyCheck` return → `supervisor.ReadyCheck`
- Modify: `internal/server/binary_service.go` — `services.ReadyCheck` references → `supervisor.ReadyCheck`

- [ ] **Step 5.1: Confirm supervisor does not import services**

```bash
grep -n "internal/services" internal/supervisor/*.go
```
Expected: no matches. (Supervisor is below services in the layering.) If matches exist, abort Task 5 and revisit.

- [ ] **Step 5.2: Create `internal/supervisor/readycheck.go`**

```go
// internal/supervisor/readycheck.go
package supervisor

import (
	"context"
	"fmt"
	"net"
	"net/http"
	"time"
)

// ReadyCheck describes how a supervisor verifies that a binary service has
// finished starting and is ready to accept requests. Construct via TCPReady
// or HTTPReady — the unexported fields prevent constructing invalid states
// (zero-value or both-set) from outside this package.
type ReadyCheck struct {
	tcpPort      int
	httpEndpoint string
	Timeout      time.Duration
}

// TCPReady returns a ReadyCheck that probes 127.0.0.1:port via TCP Dial.
func TCPReady(port int, timeout time.Duration) ReadyCheck {
	return ReadyCheck{tcpPort: port, Timeout: timeout}
}

// HTTPReady returns a ReadyCheck that GETs the given URL and expects a 2xx.
func HTTPReady(url string, timeout time.Duration) ReadyCheck {
	return ReadyCheck{httpEndpoint: url, Timeout: timeout}
}

// TCPPort returns the TCP probe port, or 0 if this is an HTTP check.
func (r ReadyCheck) TCPPort() int { return r.tcpPort }

// HTTPEndpoint returns the HTTP probe URL, or "" if this is a TCP check.
func (r ReadyCheck) HTTPEndpoint() string { return r.httpEndpoint }

// BuildReadyFunc returns a func(ctx) error appropriate to the ReadyCheck variant.
// The ReadyCheck must specify exactly one of TCPPort or HTTPEndpoint.
func BuildReadyFunc(rc ReadyCheck) (func(context.Context) error, error) {
	httpSet := rc.HTTPEndpoint() != ""
	tcpSet := rc.TCPPort() > 0
	switch {
	case httpSet && tcpSet:
		return nil, fmt.Errorf("invalid ReadyCheck: both TCPPort and HTTPEndpoint set; specify exactly one")
	case httpSet:
		client := &http.Client{Timeout: 2 * time.Second}
		url := rc.HTTPEndpoint()
		return func(ctx context.Context) error {
			req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
			if err != nil {
				return err
			}
			resp, err := client.Do(req)
			if err != nil {
				return err
			}
			defer resp.Body.Close()
			if resp.StatusCode >= 200 && resp.StatusCode < 300 {
				return nil
			}
			return fmt.Errorf("HTTP %s returned %d", url, resp.StatusCode)
		}, nil
	case tcpSet:
		addr := fmt.Sprintf("127.0.0.1:%d", rc.TCPPort())
		return func(ctx context.Context) error {
			d := net.Dialer{Timeout: 500 * time.Millisecond}
			c, err := d.DialContext(ctx, "tcp", addr)
			if err != nil {
				return err
			}
			c.Close()
			return nil
		}, nil
	default:
		return nil, fmt.Errorf("invalid ReadyCheck: must set exactly one of TCPPort or HTTPEndpoint")
	}
}
```

(`BuildReadyFunc` is the public version of `internal/server/binary_service.go`'s `buildReadyFunc`, exported because per-tool packages — rustfs, mailpit — will call it from outside `supervisor`.)

- [ ] **Step 5.3: Port the tests**

Copy the relevant tests from `internal/server/binary_service_test.go` (the ones starting with `TestBuildReadyFunc_…` per the lines we noted: 138, 148, 158) into `internal/supervisor/readycheck_test.go`, change `package server` → `package supervisor`, and rename `buildReadyFunc` → `BuildReadyFunc`. Replace `services.ReadyCheck{}` → `ReadyCheck{}`, `services.TCPReady` → `TCPReady`, `services.HTTPReady` → `HTTPReady`.

- [ ] **Step 5.4: Run the supervisor tests**

```bash
go test ./internal/supervisor/ -v
```
Expected: PASS.

- [ ] **Step 5.5: Switch `internal/services/binary.go` to use `supervisor.ReadyCheck` via alias**

Replace the local `ReadyCheck` struct + `TCPReady`/`HTTPReady` constructors with type aliases pointing at supervisor. The interface returns the supervisor type:

```go
package services

import (
	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/caddy"
	"github.com/prvious/pv/internal/supervisor"
)

// Aliases retained during the services-package wind-down. Removed in Task 12.
type ReadyCheck = supervisor.ReadyCheck

var TCPReady = supervisor.TCPReady
var HTTPReady = supervisor.HTTPReady

// BinaryService is the contract for services that run as native binaries
// supervised by the pv daemon. (...) Removed in Task 12.
type BinaryService interface {
	Name() string
	DisplayName() string
	Binary() binaries.Binary
	Args(dataDir string) []string
	Env() []string
	Port() int
	ConsolePort() int
	WebRoutes() []caddy.WebRoute
	EnvVars(projectName string) map[string]string
	ReadyCheck() ReadyCheck
}

var binaryRegistry = map[string]BinaryService{}

func LookupBinary(name string) (BinaryService, bool) {
	svc, ok := binaryRegistry[name]
	return svc, ok
}

func AllBinary() map[string]BinaryService {
	out := make(map[string]BinaryService, len(binaryRegistry))
	for k, v := range binaryRegistry {
		out[k] = v
	}
	return out
}
```

Delete the `time` and `net`/`net/http` imports from `binary.go` since they no longer appear in the file.

- [ ] **Step 5.6: Update `internal/server/binary_service.go`**

Replace `buildReadyFunc` body with a delegation to `supervisor.BuildReadyFunc`, and update the type reference:

```go
func buildReadyFunc(rc supervisor.ReadyCheck) (func(context.Context) error, error) {
	return supervisor.BuildReadyFunc(rc)
}
```

In `buildSupervisorProcess`, the line `rc := svc.ReadyCheck()` already returns the aliased type — no change needed. The local `buildReadyFunc` shim can stay (used inside this file only) or be inlined. Inline by replacing the call site:

```go
ready, err := supervisor.BuildReadyFunc(rc)
```

…and delete the now-unused `buildReadyFunc` function. Also remove the `net`, `net/http`, and `context` imports if no other code in the file uses them.

- [ ] **Step 5.7: Update `internal/server/binary_service_test.go`**

The ready-check tests now live in `internal/supervisor/`. Delete those tests from `binary_service_test.go`. Keep any other tests that exercise `buildSupervisorProcess` (path resolution, data dir creation).

- [ ] **Step 5.8: Verify build + tests**

```bash
gofmt -w . && go vet ./... && go build ./... && go test ./...
```
Expected: all green.

- [ ] **Step 5.9: Commit**

```bash
git add -A
git commit -m "refactor(services): move ReadyCheck/HTTPReady/TCPReady to supervisor"
```

---

## Task 6: Create `internal/rustfs/` per-tool package

**Files:**
- Create: `internal/rustfs/service.go` — constants, args/env builders, `BuildSupervisorProcess`, `WebRoutes`, `EnvVars`
- Create: `internal/rustfs/install.go` — `Install()` (ported from `svchooks/install.go`, no `BinaryService` arg)
- Create: `internal/rustfs/update.go` — `Update()`
- Create: `internal/rustfs/uninstall.go` — `Uninstall(deleteData bool)` plus `WaitStopped(timeout)` helper, `requireEntry()` helper
- Create: `internal/rustfs/enable.go` — `SetEnabled(enabled bool)` and `Restart()`
- Create: `internal/rustfs/status.go` — `PrintStatus()`
- Create: `internal/rustfs/logs.go` — `TailLog(ctx, follow bool)`
- Create: `internal/rustfs/env.go` — `UpdateLinkedProjectsEnv()` (ported from `UpdateLinkedProjectsEnvBinary` for the `s3` case)
- Create: `internal/rustfs/bind.go` — `BindToAllProjects()` (ported from `BindBinaryServiceToAllProjects` for `s3`)
- Create: `internal/rustfs/fallback.go` — `ApplyFallbacksToLinkedProjects()` (ported for `s3`)
- Create: `internal/rustfs/service_test.go` — port from `internal/services/rustfs_test.go`
- Create: `internal/rustfs/lifecycle_test.go` — port the rustfs cases from `internal/svchooks/lifecycle_test.go`

This task is the bulk of the work but is mechanical — extract the `s3`-specific paths from svchooks into the new package, with no `BinaryService` argument. The redis package (`internal/redis/`) is the structural template; mirror its file layout.

- [ ] **Step 6.1: Create `internal/rustfs/service.go`**

```go
// internal/rustfs/service.go
package rustfs

import (
	"fmt"
	"os"
	"path/filepath"
	"time"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/caddy"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/supervisor"
)

const (
	displayName = "S3 Storage (RustFS)"
	serviceKey  = "s3" // registry key + binding key — DO NOT change without a registry migration
	port        = 9000
	consolePort = 9001
)

// Binary returns the rustfs binary descriptor.
func Binary() binaries.Binary { return binaries.Rustfs }

// Port is the primary service port (S3 API).
func Port() int { return port }

// ConsolePort is the admin UI port.
func ConsolePort() int { return consolePort }

// DisplayName returns the human-readable name.
func DisplayName() string { return displayName }

// ServiceKey returns the registry key.
func ServiceKey() string { return serviceKey }

// WebRoutes maps subdomains for the caddy reverse proxy.
func WebRoutes() []caddy.WebRoute {
	return []caddy.WebRoute{
		{Subdomain: "s3", Port: consolePort},
		{Subdomain: "s3-api", Port: port},
	}
}

// EnvVars returns the env vars injected into a linked project's .env.
func EnvVars(projectName string) map[string]string {
	return map[string]string{
		"AWS_ACCESS_KEY_ID":           "rstfsadmin",
		"AWS_SECRET_ACCESS_KEY":       "rstfsadmin",
		"AWS_DEFAULT_REGION":          "us-east-1",
		"AWS_BUCKET":                  projectName,
		"AWS_ENDPOINT":                "http://127.0.0.1:9000",
		"AWS_USE_PATH_STYLE_ENDPOINT": "true",
	}
}

// BuildSupervisorProcess returns the supervisor.Process for rustfs.
// Mirrors postgres.BuildSupervisorProcess / mysql.BuildSupervisorProcess.
func BuildSupervisorProcess() (supervisor.Process, error) {
	binPath := filepath.Join(config.InternalBinDir(), Binary().Name)

	dataDir := config.ServiceDataDir(serviceKey, "latest")
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		return supervisor.Process{}, fmt.Errorf("create data dir %s: %w", dataDir, err)
	}

	logFile := filepath.Join(config.PvDir(), "logs", Binary().Name+".log")
	if err := os.MkdirAll(filepath.Dir(logFile), 0o755); err != nil {
		return supervisor.Process{}, fmt.Errorf("create log dir: %w", err)
	}

	rc := supervisor.TCPReady(port, 30*time.Second)
	ready, err := supervisor.BuildReadyFunc(rc)
	if err != nil {
		return supervisor.Process{}, fmt.Errorf("rustfs: %w", err)
	}

	args := []string{
		"server", dataDir,
		"--address", fmt.Sprintf(":%d", port),
		"--console-enable",
		"--console-address", fmt.Sprintf(":%d", consolePort),
	}
	env := []string{
		"RUSTFS_ACCESS_KEY=rstfsadmin",
		"RUSTFS_SECRET_KEY=rstfsadmin",
	}

	return supervisor.Process{
		Name:         Binary().Name,
		Binary:       binPath,
		Args:         args,
		Env:          env,
		LogFile:      logFile,
		Ready:        ready,
		ReadyTimeout: rc.Timeout,
	}, nil
}
```

- [ ] **Step 6.2: Create `internal/rustfs/wait.go`**

```go
// internal/rustfs/wait.go
package rustfs

import (
	"fmt"
	"time"

	"github.com/prvious/pv/internal/server"
)

// WaitStopped polls daemon-status.json until rustfs is no longer running,
// or until timeout. Used before destructive on-disk operations during
// uninstall and between the disable/enable halves of a restart.
func WaitStopped(timeout time.Duration) error {
	binaryName := Binary().Name
	deadline := time.Now().Add(timeout)
	for time.Now().Before(deadline) {
		snap, err := server.ReadDaemonStatus()
		if err != nil {
			return nil
		}
		st, ok := snap.Supervised[binaryName]
		if !ok || !st.Running {
			return nil
		}
		time.Sleep(200 * time.Millisecond)
	}
	return fmt.Errorf("%s did not stop within %s", DisplayName(), timeout)
}
```

- [ ] **Step 6.3: Create `internal/rustfs/install.go`**

Port `svchooks.Install()` (and `PrintConnectionDetails`) into rustfs-only form. Remove the `BinaryService` argument; replace `svc.Name()` → `serviceKey`, `svc.Binary()` → `Binary()`, etc. Drop the `inst.Kind` check (Task 13 strips Kind everywhere; this task should not write `Kind: "binary"` either).

```go
// internal/rustfs/install.go
package rustfs

import (
	"fmt"
	"net/http"
	"os"
	"time"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/caddy"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
)

// Install downloads the rustfs binary, registers it, retroactively binds
// it to existing Laravel projects, writes their .env vars, and signals
// the daemon to reconcile. Idempotent on already-registered.
func Install() error {
	reg, err := registry.Load()
	if err != nil {
		return fmt.Errorf("cannot load registry: %w", err)
	}
	if _, exists := reg.Services[serviceKey]; exists {
		ui.Success(fmt.Sprintf("%s is already added", DisplayName()))
		return nil
	}

	client := &http.Client{Timeout: 60 * time.Second}

	latest, err := binaries.FetchLatestVersion(client, Binary())
	if err != nil {
		return fmt.Errorf("cannot resolve latest %s version: %w", Binary().DisplayName, err)
	}
	if err := ui.Step(fmt.Sprintf("Downloading %s %s...", Binary().DisplayName, latest), func() (string, error) {
		if err := binaries.InstallBinary(client, Binary(), latest); err != nil {
			return "", err
		}
		return fmt.Sprintf("Installed %s %s", Binary().DisplayName, latest), nil
	}); err != nil {
		return err
	}

	vs, err := binaries.LoadVersions()
	if err != nil {
		return fmt.Errorf("cannot load versions state: %w", err)
	}
	vs.Set(Binary().Name, latest)
	if err := vs.Save(); err != nil {
		return fmt.Errorf("cannot save versions state: %w", err)
	}

	enabled := true
	inst := &registry.ServiceInstance{
		Port:        Port(),
		ConsolePort: ConsolePort(),
		Enabled:     &enabled,
	}
	if err := reg.AddService(serviceKey, inst); err != nil {
		return err
	}
	if err := reg.Save(); err != nil {
		return fmt.Errorf("cannot save registry: %w", err)
	}

	if err := BindToAllProjects(reg); err != nil {
		return fmt.Errorf("cannot bind service to projects: %w", err)
	}
	if err := reg.Save(); err != nil {
		return fmt.Errorf("cannot save registry after binding: %w", err)
	}

	UpdateLinkedProjectsEnv(reg)

	if err := caddy.GenerateServiceSiteConfigs(reg); err != nil {
		ui.Subtle(fmt.Sprintf("Could not generate service site config: %v", err))
	}

	if server.IsRunning() {
		if err := server.SignalDaemon(); err != nil {
			ui.Subtle(fmt.Sprintf("Could not signal daemon: %v", err))
		}
		ui.Success(fmt.Sprintf("%s registered and running on :%d", DisplayName(), Port()))
	} else {
		ui.Success(fmt.Sprintf("%s registered on :%d", DisplayName(), Port()))
		ui.Subtle("daemon not running — service will start on next `pv start`")
	}

	printConnectionDetails()
	return nil
}

func printConnectionDetails() {
	fmt.Fprintln(os.Stderr)
	fmt.Fprintf(os.Stderr, "    %s  127.0.0.1\n", ui.Muted.Render("Host"))
	fmt.Fprintf(os.Stderr, "    %s  %d\n", ui.Muted.Render("Port"), Port())
	settings, _ := config.LoadSettings()
	if settings != nil {
		for _, route := range WebRoutes() {
			fmt.Fprintf(os.Stderr, "    %s  https://%s.pv.%s\n",
				ui.Muted.Render(route.Subdomain), route.Subdomain, settings.Defaults.TLD)
		}
	}
	fmt.Fprintln(os.Stderr)
}
```

- [ ] **Step 6.4: Create `internal/rustfs/update.go`**

```go
// internal/rustfs/update.go
package rustfs

import (
	"fmt"
	"net/http"
	"time"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
)

// Update re-downloads the rustfs binary to the latest upstream version.
// On success the daemon is signaled so the supervisor restarts the process.
func Update() error {
	reg, err := registry.Load()
	if err != nil {
		return fmt.Errorf("cannot load registry: %w", err)
	}
	if _, ok := reg.Services[serviceKey]; !ok {
		return fmt.Errorf("%s not registered (run `pv rustfs:install` first)", serviceKey)
	}

	client := &http.Client{Timeout: 60 * time.Second}

	latest, err := binaries.FetchLatestVersion(client, Binary())
	if err != nil {
		return fmt.Errorf("cannot resolve latest %s version: %w", Binary().DisplayName, err)
	}
	if err := ui.Step(fmt.Sprintf("Updating %s to %s...", Binary().DisplayName, latest), func() (string, error) {
		if err := binaries.InstallBinary(client, Binary(), latest); err != nil {
			return "", err
		}
		return fmt.Sprintf("Installed %s %s", Binary().DisplayName, latest), nil
	}); err != nil {
		return err
	}

	vs, err := binaries.LoadVersions()
	if err != nil {
		return fmt.Errorf("cannot load versions state: %w", err)
	}
	vs.Set(Binary().Name, latest)
	if err := vs.Save(); err != nil {
		return fmt.Errorf("cannot save versions state: %w", err)
	}

	if server.IsRunning() {
		if err := server.SignalDaemon(); err != nil {
			return fmt.Errorf(
				"%s binary updated to %s, but the daemon is still running the previous version (run `pv restart`): %w",
				DisplayName(), latest, err,
			)
		}
	}
	ui.Success(fmt.Sprintf("%s updated to %s", DisplayName(), latest))
	_ = reg
	return nil
}
```

- [ ] **Step 6.5: Create `internal/rustfs/enable.go`**

```go
// internal/rustfs/enable.go
package rustfs

import (
	"fmt"
	"time"

	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
)

// SetEnabled flips the registry Enabled flag and signals the daemon.
func SetEnabled(enabled bool) error {
	reg, err := registry.Load()
	if err != nil {
		return fmt.Errorf("cannot load registry: %w", err)
	}
	inst, ok := reg.Services[serviceKey]
	if !ok {
		return fmt.Errorf("%s not registered (run `pv rustfs:install` first)", serviceKey)
	}
	flag := enabled
	inst.Enabled = &flag
	if err := reg.Save(); err != nil {
		return fmt.Errorf("cannot save registry: %w", err)
	}

	verb := "enabled"
	if !enabled {
		verb = "disabled"
	}

	if !server.IsRunning() {
		ui.Success(fmt.Sprintf("%s %s", DisplayName(), verb))
		if enabled {
			ui.Subtle("daemon not running — service will start on next `pv start`")
		}
		return nil
	}

	if err := server.SignalDaemon(); err != nil {
		ui.Subtle("Run `pv restart` to load the change.")
		return fmt.Errorf("%s %s in registry, but could not signal daemon: %w", DisplayName(), verb, err)
	}
	ui.Success(fmt.Sprintf("%s %s; daemon reconciled", DisplayName(), verb))
	return nil
}

// Restart toggles disabled, waits for the supervisor to confirm exit, then re-enables.
func Restart() error {
	if err := SetEnabled(false); err != nil {
		return err
	}
	if server.IsRunning() {
		if err := WaitStopped(30 * time.Second); err != nil {
			return fmt.Errorf("waiting for %s to stop: %w", DisplayName(), err)
		}
	}
	return SetEnabled(true)
}
```

- [ ] **Step 6.6: Create `internal/rustfs/uninstall.go`**

Port `svchooks.Uninstall` for rustfs only. Remove the `inst.Kind != "binary"` check (deferred to Task 13). The `requireEntry` helper inlines into the function.

```go
// internal/rustfs/uninstall.go
package rustfs

import (
	"fmt"
	"os"
	"path/filepath"
	"time"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/caddy"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
)

// Uninstall stops, unregisters, removes the rustfs binary, and (when
// deleteData is true) wipes the data directory. Linked Laravel projects
// are unbound and their .env files get fallback values applied.
func Uninstall(deleteData bool) error {
	reg, err := registry.Load()
	if err != nil {
		return fmt.Errorf("cannot load registry: %w", err)
	}
	inst, ok := reg.Services[serviceKey]
	if !ok {
		return fmt.Errorf("%s not registered", serviceKey)
	}

	disabled := false
	inst.Enabled = &disabled
	if err := reg.Save(); err != nil {
		return fmt.Errorf("cannot save registry: %w", err)
	}
	if server.IsRunning() {
		if err := server.SignalDaemon(); err != nil {
			return fmt.Errorf("could not signal daemon to stop %s: %w", serviceKey, err)
		}
		if err := WaitStopped(30 * time.Second); err != nil {
			ui.Subtle(fmt.Sprintf("Could not confirm %s stopped: %v (continuing)", DisplayName(), err))
		}
	}

	binPath := filepath.Join(config.InternalBinDir(), Binary().Name)
	if err := os.Remove(binPath); err != nil && !os.IsNotExist(err) {
		ui.Subtle(fmt.Sprintf("Could not remove %s: %v (file left behind)", binPath, err))
	}
	if vs, vsErr := binaries.LoadVersions(); vsErr != nil {
		ui.Subtle(fmt.Sprintf("Could not load versions file: %v (manifest may be stale)", vsErr))
	} else {
		vs.Set(Binary().Name, "")
		if err := vs.Save(); err != nil {
			ui.Subtle(fmt.Sprintf("Could not save versions file: %v", err))
		}
	}
	if deleteData {
		dataDir := config.ServiceDataDir(serviceKey, "latest")
		if err := os.RemoveAll(dataDir); err != nil {
			return fmt.Errorf("cannot delete data: %w", err)
		}
	}

	ApplyFallbacksToLinkedProjects(reg)
	reg.UnbindService(serviceKey)

	if err := reg.RemoveService(serviceKey); err != nil {
		return err
	}
	if err := reg.Save(); err != nil {
		return fmt.Errorf("cannot save registry: %w", err)
	}

	if err := caddy.GenerateServiceSiteConfigs(reg); err != nil {
		ui.Subtle(fmt.Sprintf("Could not regenerate service site config: %v", err))
	}
	if server.IsRunning() {
		if err := server.SignalDaemon(); err != nil {
			ui.Subtle(fmt.Sprintf("Could not signal daemon: %v", err))
		}
	}
	return nil
}
```

- [ ] **Step 6.7: Create `internal/rustfs/status.go`**

Port `svchooks.PrintStatus` for rustfs only. Drop the `Kind` row.

```go
// internal/rustfs/status.go
package rustfs

import (
	"fmt"
	"os"

	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
)

// PrintStatus writes the rustfs detail block to stderr.
func PrintStatus() {
	reg, err := registry.Load()
	if err != nil {
		ui.Subtle(fmt.Sprintf("Could not load registry: %v", err))
		return
	}
	inst, ok := reg.Services[serviceKey]
	enabled := true
	registered := ok
	if ok && inst.Enabled != nil {
		enabled = *inst.Enabled
	}

	runningLabel := "false"
	pid := 0
	snap, err := server.ReadDaemonStatus()
	switch {
	case err != nil && !os.IsNotExist(err):
		runningLabel = "unknown"
		ui.Subtle(fmt.Sprintf("Could not read daemon status: %v", err))
	case err == nil:
		if st, exists := snap.Supervised[Binary().Name]; exists {
			if st.Running {
				runningLabel = "true"
			}
			pid = st.PID
		}
	}

	fmt.Fprintln(os.Stderr)
	fmt.Fprintf(os.Stderr, "  %s  %s\n", ui.Muted.Render("Service"), DisplayName())
	fmt.Fprintf(os.Stderr, "  %s  %v\n", ui.Muted.Render("Registered"), registered)
	fmt.Fprintf(os.Stderr, "  %s  %v\n", ui.Muted.Render("Enabled"), enabled)
	fmt.Fprintf(os.Stderr, "  %s  %s\n", ui.Muted.Render("Running"), runningLabel)
	if pid > 0 {
		fmt.Fprintf(os.Stderr, "  %s  %d\n", ui.Muted.Render("PID"), pid)
	}
	fmt.Fprintln(os.Stderr)
}
```

Note: imports `_ = registry.Registry{}` not needed; `reg` value is used.

- [ ] **Step 6.8: Create `internal/rustfs/logs.go`**

```go
// internal/rustfs/logs.go
package rustfs

import (
	"context"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"time"

	"github.com/prvious/pv/internal/config"
)

// TailLog tails ~/.pv/logs/rustfs.log to stdout. When follow is true,
// it polls every 250ms until ctx is cancelled.
func TailLog(ctx context.Context, follow bool) error {
	logPath := filepath.Join(config.PvDir(), "logs", Binary().Name+".log")
	f, err := os.Open(logPath)
	if err != nil {
		if os.IsNotExist(err) {
			return fmt.Errorf("no log file yet (%s). Has the service run?", logPath)
		}
		return err
	}
	defer f.Close()

	if _, err := io.Copy(os.Stdout, f); err != nil {
		return err
	}
	if !follow {
		return nil
	}
	for {
		select {
		case <-ctx.Done():
			return nil
		case <-time.After(250 * time.Millisecond):
		}
		if _, err := io.Copy(os.Stdout, f); err != nil {
			if err == io.EOF {
				continue
			}
			return err
		}
	}
}
```

- [ ] **Step 6.9: Create `internal/rustfs/bind.go`**

Ports `BindBinaryServiceToAllProjects` for the s3 case.

```go
// internal/rustfs/bind.go
package rustfs

import "github.com/prvious/pv/internal/registry"

// BindToAllProjects sets Services.S3=true on every Laravel project so
// UpdateLinkedProjectsEnv can find projects that were linked before
// rustfs existed.
func BindToAllProjects(reg *registry.Registry) error {
	for i := range reg.Projects {
		p := &reg.Projects[i]
		if p.Type != "laravel" && p.Type != "laravel-octane" {
			continue
		}
		if p.Services == nil {
			p.Services = &registry.ProjectServices{}
		}
		p.Services.S3 = true
	}
	return nil
}
```

- [ ] **Step 6.10: Create `internal/rustfs/env.go`**

Ports `UpdateLinkedProjectsEnvBinary` for s3, calling a new local `updateProjectEnv` function (which replaces the `laravel.UpdateProjectEnvForBinaryService` call for the s3 case).

```go
// internal/rustfs/env.go
package rustfs

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/laravel"
	"github.com/prvious/pv/internal/projectenv"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/ui"
)

// UpdateLinkedProjectsEnv writes rustfs env vars to .env for every Laravel
// project bound to s3, gated by the user's automation settings.
func UpdateLinkedProjectsEnv(reg *registry.Registry) {
	settings, err := config.LoadSettings()
	if err != nil {
		ui.Subtle(fmt.Sprintf("Could not load settings for service env hooks: %v", err))
		return
	}
	if settings.Automation.ServiceEnvUpdate == config.AutoOff {
		return
	}

	linkedNames := reg.ProjectsUsingService(serviceKey)
	var laravelProjects []registry.Project
	for _, name := range linkedNames {
		p := reg.Find(name)
		if p != nil && (p.Type == "laravel" || p.Type == "laravel-octane") {
			laravelProjects = append(laravelProjects, *p)
		}
	}
	if len(laravelProjects) == 0 {
		return
	}

	shouldUpdate := settings.Automation.ServiceEnvUpdate == config.AutoOn
	if settings.Automation.ServiceEnvUpdate == config.AutoAsk {
		if !automation.IsInteractive() {
			return
		}
		confirmed, err := automation.ConfirmFunc(
			fmt.Sprintf("Update .env for %d linked Laravel project(s)", len(laravelProjects)),
		)
		if err != nil {
			return
		}
		shouldUpdate = confirmed
	}
	if !shouldUpdate {
		return
	}

	for _, p := range laravelProjects {
		project := reg.Find(p.Name)
		if project == nil || project.Services == nil {
			continue
		}
		if err := UpdateProjectEnv(p.Path, p.Name, project.Services); err != nil {
			ui.Subtle(fmt.Sprintf("Could not update .env for %s: %v", p.Name, err))
		} else {
			ui.Success(fmt.Sprintf("Updated .env for %s", p.Name))
		}
	}
}

// UpdateProjectEnv merges rustfs connection vars + Laravel smart vars into a
// single project's .env file. Replaces the s3 branch of
// laravel.UpdateProjectEnvForBinaryService.
func UpdateProjectEnv(projectPath, projectName string, bound *registry.ProjectServices) error {
	envPath := filepath.Join(projectPath, ".env")
	if _, err := os.Stat(envPath); os.IsNotExist(err) {
		return nil
	}
	allVars := EnvVars(projectName)
	smartVars := laravel.SmartEnvVars(bound)
	for k, v := range smartVars {
		allVars[k] = v
	}
	backupPath := envPath + ".pv-backup"
	return projectenv.MergeDotEnv(envPath, backupPath, allVars)
}
```

- [ ] **Step 6.11: Create `internal/rustfs/fallback.go`**

```go
// internal/rustfs/fallback.go
package rustfs

import (
	"fmt"
	"path/filepath"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/laravel"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/ui"
)

// ApplyFallbacksToLinkedProjects applies safe env fallbacks when rustfs
// is removed (FILESYSTEM_DISK=s3 → local).
func ApplyFallbacksToLinkedProjects(reg *registry.Registry) {
	settings, err := config.LoadSettings()
	if err != nil {
		ui.Subtle(fmt.Sprintf("Could not load settings for service fallback hooks: %v", err))
		return
	}
	if settings.Automation.ServiceFallback == config.AutoOff {
		return
	}

	projectNames := reg.ProjectsUsingService(serviceKey)
	if len(projectNames) == 0 {
		return
	}

	shouldFallback := settings.Automation.ServiceFallback == config.AutoOn
	if settings.Automation.ServiceFallback == config.AutoAsk {
		if !automation.IsInteractive() {
			return
		}
		confirmed, err := automation.ConfirmFunc(
			fmt.Sprintf("Apply env fallbacks for %s to %d project(s)", serviceKey, len(projectNames)),
		)
		if err != nil {
			return
		}
		shouldFallback = confirmed
	}
	if !shouldFallback {
		return
	}

	for _, pName := range projectNames {
		project := reg.Find(pName)
		if project == nil {
			continue
		}
		envPath := filepath.Join(project.Path, ".env")
		if err := laravel.ApplyFallbacks(envPath, serviceKey); err != nil {
			ui.Subtle(fmt.Sprintf("Could not apply fallbacks for %s: %v", pName, err))
		} else {
			ui.Success(fmt.Sprintf("Applied %s fallbacks for %s", serviceKey, pName))
		}
	}
}
```

- [ ] **Step 6.12: Port `internal/services/rustfs_test.go` → `internal/rustfs/service_test.go`**

Open `internal/services/rustfs_test.go`. The original tests assert behavior of methods on the `RustFS` struct (e.g., `(*RustFS).EnvVars(name)`, `(*RustFS).Args("/data")`, etc.). Rewrite each test to call the package-level functions instead. Examples of the rename:

- `svc := &services.RustFS{}; svc.EnvVars(...)` → `rustfs.EnvVars(...)`
- `svc.Port()` → `rustfs.Port()`
- `svc.Args("/data")` → call `rustfs.BuildSupervisorProcess()` and inspect `proc.Args`
- `svc.WebRoutes()` → `rustfs.WebRoutes()`

Place the new file at `internal/rustfs/service_test.go`, `package rustfs`. Keep test names so coverage diff is interpretable.

- [ ] **Step 6.13: Port the rustfs cases from `internal/svchooks/lifecycle_test.go`**

In `internal/svchooks/lifecycle_test.go`, find the test functions exercising the s3 path (search for `mustS3` / `"s3"`). Copy them into `internal/rustfs/lifecycle_test.go`, rewrite to call the new `rustfs.Install()`, `rustfs.SetEnabled(true/false)`, `rustfs.Update()`, `rustfs.Uninstall(...)` directly without a `BinaryService` argument. Remove any cases that exclusively exercise the `BinaryService` polymorphism (those become moot).

- [ ] **Step 6.14: Verify build + tests for the new package**

```bash
gofmt -w . && go vet ./... && go build ./... && go test ./internal/rustfs/ -v
```
Expected: PASS. The rest of the tree may still rely on `internal/services/`'s rustfs registration; that's fine — it stays in place until Task 12.

- [ ] **Step 6.15: Commit**

```bash
git add -A
git commit -m "feat(rustfs): self-contained per-tool package mirroring redis/pg/mysql"
```

---

## Task 7: Create `internal/mailpit/` per-tool package

Mirror Task 6 for mailpit. Differences from rustfs:

- `serviceKey = "mail"`
- `port = 1025`, `consolePort = 8025`
- `displayName = "Mail (Mailpit)"`
- `Binary() = binaries.Mailpit`
- `WebRoutes()` returns one route: `{Subdomain: "mail", Port: 8025}`
- `EnvVars(_)` returns the mail-specific keys (MAIL_MAILER, MAIL_HOST, MAIL_PORT, MAIL_USERNAME, MAIL_PASSWORD); the projectName parameter is unused but keep the signature for symmetry with rustfs and future per-project bucket-style keys
- Args: `"--smtp", ":1025", "--listen", ":8025", "--database", dataDir + "/mailpit.db"`
- Env: nil
- Ready check: `supervisor.HTTPReady("http://127.0.0.1:8025/livez", 30*time.Second)`
- `BindToAllProjects`: sets `p.Services.Mail = true` instead of `p.Services.S3 = true`
- `UpdateProjectEnv`/`ApplyFallbacksToLinkedProjects`: call `laravel.ApplyFallbacks(envPath, "mail")`; bind via `Services.Mail`

**Files:** the same set as Task 6 with `mailpit` substituted for `rustfs` everywhere.

- [ ] **Step 7.1 – 7.11:** Create the mailpit equivalents of `service.go`, `wait.go`, `install.go`, `update.go`, `enable.go`, `uninstall.go`, `status.go`, `logs.go`, `bind.go`, `env.go`, `fallback.go`. Use Task 6 as the template; substitute the values listed above.

- [ ] **Step 7.12: Port `internal/services/mailpit_test.go` → `internal/mailpit/service_test.go`**

Same approach as Step 6.12 but for the `Mailpit` struct → package-level functions. Note: `mailpit_test.go` includes `TestMailpit_EnvVars_Golden` which is the migration contract with the old Docker service. Keep it intact, rewriting the call to `mailpit.EnvVars("any-project")`.

- [ ] **Step 7.13: Port the mailpit cases from `internal/svchooks/lifecycle_test.go`**

Same approach as Step 6.13.

- [ ] **Step 7.14: Verify**

```bash
gofmt -w . && go vet ./... && go build ./... && go test ./internal/mailpit/ -v
```
Expected: PASS.

- [ ] **Step 7.15: Commit**

```bash
git add -A
git commit -m "feat(mailpit): self-contained per-tool package mirroring redis/pg/mysql"
```

---

## Task 8: Switch cobra wrappers to call the new packages

The existing files in `internal/commands/rustfs/` and `internal/commands/mailpit/` currently delegate to `services.LookupBinary("s3"|"mail")` + `svchooks.*`. Each one becomes a thin call into the new package.

**Files:**
- Modify (rustfs): `internal/commands/rustfs/{install,start,stop,restart,update,uninstall,status,logs}.go`
- Modify (mailpit): `internal/commands/mailpit/{install,start,stop,restart,update,uninstall,status,logs}.go`

The conversion is mechanical. Example for `internal/commands/rustfs/install.go`:

**Before:**
```go
import (
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/svchooks"
)

RunE: func(cmd *cobra.Command, args []string) error {
	reg, err := registry.Load()
	if err != nil { return fmt.Errorf("cannot load registry: %w", err) }
	svc, ok := services.LookupBinary("s3")
	if !ok { return fmt.Errorf("rustfs binary service not registered (build issue)") }
	return svchooks.Install(reg, svc)
}
```

**After:**
```go
import "github.com/prvious/pv/internal/rustfs"

RunE: func(cmd *cobra.Command, args []string) error {
	return rustfs.Install()
}
```

- [ ] **Step 8.1: Rewrite `internal/commands/rustfs/install.go`**

```go
package rustfs

import (
	pkg "github.com/prvious/pv/internal/rustfs"
	"github.com/spf13/cobra"
)

var installCmd = &cobra.Command{
	Use:     "rustfs:install",
	GroupID: "rustfs",
	Short:   "Install RustFS (S3-compatible storage) and start it",
	Long:    "Downloads the RustFS binary, registers it as a supervised service, and signals the daemon to start it.",
	Example: `pv rustfs:install
pv s3:install`,
	Args: cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		return pkg.Install()
	},
}
```

Note: the cobra wrapper package is already named `rustfs`. To avoid collision with the new `internal/rustfs` package, import it under the alias `pkg`. Keep this alias in every cobra wrapper file.

- [ ] **Step 8.2: Rewrite `internal/commands/rustfs/start.go`** → `pkg.SetEnabled(true)`
- [ ] **Step 8.3: Rewrite `internal/commands/rustfs/stop.go`** → `pkg.SetEnabled(false)`
- [ ] **Step 8.4: Rewrite `internal/commands/rustfs/restart.go`** → `pkg.Restart()`
- [ ] **Step 8.5: Rewrite `internal/commands/rustfs/update.go`** → `pkg.Update()`
- [ ] **Step 8.6: Rewrite `internal/commands/rustfs/status.go`** → `pkg.PrintStatus(); return nil`
- [ ] **Step 8.7: Rewrite `internal/commands/rustfs/logs.go`** → `pkg.TailLog(cmd.Context(), logsFollow)`
- [ ] **Step 8.8: Rewrite `internal/commands/rustfs/uninstall.go`** → `pkg.Uninstall(true)` (preserve the `--force` confirmation prompt)
- [ ] **Step 8.9: Repeat Steps 8.1–8.8 for `internal/commands/mailpit/`** with `pkg "github.com/prvious/pv/internal/mailpit"`. Note: mailpit:start / mailpit:stop / etc. all parallel rustfs.

- [ ] **Step 8.10: Verify**

```bash
gofmt -w . && go vet ./... && go build ./... && go test ./...
```
Expected: build succeeds. Some `internal/services/` types are still referenced (manager.go, caddy.go, laravel/env.go, cmd/install.go); those are still OK because Task 6/7 left the `services` registrations intact.

- [ ] **Step 8.11: Commit**

```bash
git add -A
git commit -m "refactor(commands): rustfs/mailpit cobra wrappers call new packages"
```

---

## Task 9: Update `internal/server/manager.go` and delete `binary_service.go`

**Files:**
- Modify: `internal/server/manager.go`
- Delete: `internal/server/binary_service.go`
- Modify: `internal/server/binary_service_test.go` — keep only tests not covered by `internal/supervisor/readycheck_test.go`; if everything has moved, delete this file too.

- [ ] **Step 9.1: Replace the Source 1 block in `reconcileBinaryServices`**

In `internal/server/manager.go`, find the block at line 191–206 (Source 1 — single-version binary services). Replace it with two explicit blocks:

```go
// Source 1a — rustfs.
if entry := reg.Services["s3"]; entry != nil {
	if entry.Enabled == nil || *entry.Enabled {
		proc, err := rustfs.BuildSupervisorProcess()
		if err != nil {
			startErrors = append(startErrors, fmt.Sprintf("s3: build: %v", err))
		} else {
			wanted[rustfs.Binary().Name] = proc
		}
	}
}

// Source 1b — mailpit.
if entry := reg.Services["mail"]; entry != nil {
	if entry.Enabled == nil || *entry.Enabled {
		proc, err := mailpit.BuildSupervisorProcess()
		if err != nil {
			startErrors = append(startErrors, fmt.Sprintf("mail: build: %v", err))
		} else {
			wanted[mailpit.Binary().Name] = proc
		}
	}
}
```

(The `entry.Kind != "binary"` guard from the old code is dropped — Task 13 will remove `Kind` entirely; here it's already a tautology because every `services` entry post-migration is binary.)

- [ ] **Step 9.2: Update imports in `manager.go`**

Add: `"github.com/prvious/pv/internal/mailpit"` and `"github.com/prvious/pv/internal/rustfs"`.
Remove: `"github.com/prvious/pv/internal/services"` if no longer referenced anywhere in the file.

- [ ] **Step 9.3: Delete `internal/server/binary_service.go`**

```bash
rm internal/server/binary_service.go
```

The `buildSupervisorProcess(svc services.BinaryService)` adapter is gone; rustfs and mailpit each provide their own.

- [ ] **Step 9.4: Update or delete `internal/server/binary_service_test.go`**

Inspect what tests remain (data-dir creation, log-dir creation, path resolution). If those are now covered by `internal/rustfs/service_test.go` / `internal/mailpit/service_test.go` (calling `BuildSupervisorProcess()` and asserting the returned `Process`), delete the file. Otherwise, keep only tests that exercise behavior not duplicated elsewhere, and rename them to call the rustfs/mailpit `BuildSupervisorProcess()` helpers.

- [ ] **Step 9.5: Verify**

```bash
gofmt -w . && go vet ./... && go build ./... && go test ./...
```
Expected: PASS.

- [ ] **Step 9.6: Commit**

```bash
git add -A
git commit -m "refactor(server): manager calls rustfs/mailpit directly; drop adapter"
```

---

## Task 10: Update `internal/caddy/caddy.go`

**Files:**
- Modify: `internal/caddy/caddy.go`

- [ ] **Step 10.1: Replace the `services.LookupBinary` lookup**

In `internal/caddy/caddy.go` around line 360–388 the loop currently does:

```go
for key := range reg.Services {
	svcName := key
	if idx := strings.Index(key, ":"); idx > 0 {
		svcName = key[:idx]
	}

	binSvc, ok := services.LookupBinary(svcName)
	if !ok {
		continue
	}
	routes := binSvc.WebRoutes()
	// ...
}
```

Replace with a switch on `svcName`:

```go
for key := range reg.Services {
	svcName := key
	if idx := strings.Index(key, ":"); idx > 0 {
		svcName = key[:idx]
	}

	var routes []WebRoute
	switch svcName {
	case "s3":
		routes = rustfs.WebRoutes()
	case "mail":
		routes = mailpit.WebRoutes()
	default:
		continue
	}
	// ...
}
```

- [ ] **Step 10.2: Update imports**

Add `"github.com/prvious/pv/internal/mailpit"` and `"github.com/prvious/pv/internal/rustfs"`. Remove `"github.com/prvious/pv/internal/services"` if no other usage remains in this file.

- [ ] **Step 10.3: Verify**

```bash
gofmt -w . && go vet ./... && go build ./... && go test ./...
```
Expected: PASS. Also confirms no `caddy → services` cycle remains.

- [ ] **Step 10.4: Commit**

```bash
git add -A
git commit -m "refactor(caddy): drop services.LookupBinary; switch on service name"
```

---

## Task 11: Split `laravel.UpdateProjectEnvForBinaryService`; update `cmd/link.go`

The function `UpdateProjectEnvForBinaryService` in `internal/laravel/env.go` is now redundant — `rustfs.UpdateProjectEnv` and `mailpit.UpdateProjectEnv` (added in Tasks 6 and 7) cover the same use case per-tool.

**Files:**
- Modify: `internal/laravel/env.go` — delete `UpdateProjectEnvForBinaryService`
- Modify: any callers (grep)

- [ ] **Step 11.1: Confirm scope**

```bash
grep -rn "UpdateProjectEnvForBinaryService" --include="*.go" .
```

Expected: only definition in `internal/laravel/env.go`. (The svchooks call site moved into `internal/rustfs/env.go` and `internal/mailpit/env.go` in Tasks 6/7.) If callers remain, update them by switching on the service name and dispatching to `rustfs.UpdateProjectEnv` or `mailpit.UpdateProjectEnv`.

- [ ] **Step 11.2: Delete the function**

In `internal/laravel/env.go`, remove the `UpdateProjectEnvForBinaryService` function. Remove the now-unused `services` import from this file (the remaining functions `UpdateProjectEnvFor{Postgres,Mysql,Redis}` use `projectenv.MergeDotEnv` after Task 1; `ApplyFallbacks` already does too). If the `services` import is still referenced for anything else, leave it.

- [ ] **Step 11.3: Verify**

```bash
gofmt -w . && go vet ./... && go build ./... && go test ./...
```

- [ ] **Step 11.4: Commit**

```bash
git add -A
git commit -m "refactor(laravel): drop UpdateProjectEnvForBinaryService"
```

---

## Task 12: Delete `internal/services/` and `internal/svchooks/`

By this point, every external consumer has been migrated. The remaining content of these packages — the `BinaryService` interface, the `binaryRegistry`, `Lookup`/`Available`/`AllBinary`, the `RustFS`/`Mailpit` structs, all of svchooks — is dead.

**Files:**
- Delete: `internal/services/` (entire directory)
- Delete: `internal/svchooks/` (entire directory)
- Modify: `cmd/install.go`, `cmd/setup.go`, `cmd/update.go` — remove `services.Available()` / `services.LookupBinary()` / `services.AllBinary()` calls and replace with hardcoded `binaryAddons` slice + dispatch

- [ ] **Step 12.1: Confirm no remaining imports**

```bash
grep -rn '"github.com/prvious/pv/internal/services"' --include="*.go" .
grep -rn '"github.com/prvious/pv/internal/svchooks"' --include="*.go" .
```

Expected hits at this point:
- `cmd/install.go` (parseWith — `services.LookupBinary` / `services.Available`)
- `cmd/setup.go` (`buildServiceOptions` — `services.Available` / `services.LookupBinary`)
- `cmd/update.go` (binary update loop — `services.AllBinary`)

Address each in Steps 12.2–12.4 before deleting the directory.

- [ ] **Step 12.2: Update `cmd/install.go` `parseWith`**

In `cmd/install.go`, replace the `services.LookupBinary(name)` validation (line ~56) with a call to a local helper:

```go
// cmd/install.go (helper added near installBinaryService at line 248)
var binaryAddons = []string{"s3", "mail"}

func isKnownBinaryAddon(name string) bool {
	for _, a := range binaryAddons {
		if a == name {
			return true
		}
	}
	return false
}
```

Update `parseWith`:

```go
if strings.HasPrefix(item, "service[") && strings.HasSuffix(item, "]") {
	name := item[8 : len(item)-1]
	if !isKnownBinaryAddon(name) {
		return spec, fmt.Errorf("unknown service %q in --with (available: %s)", name, strings.Join(binaryAddons, ", "))
	}
	spec.services = append(spec.services, serviceSpec{name: name})
}
```

Remove `services` from the import list.

- [ ] **Step 12.3: Update `cmd/setup.go` `buildServiceOptions`**

```go
// cmd/setup.go (replaces buildServiceOptions at line 287–298)
func buildServiceOptions() []selectOption {
	return []selectOption{
		{label: "S3 Storage (RustFS)", value: "s3"},
		{label: "Mail (Mailpit)", value: "mail"},
	}
}
```

Remove `services` from the import list.

(Display strings come from `rustfs.DisplayName()` / `mailpit.DisplayName()` if you'd rather avoid the duplication — both options are fine. Hardcoding here keeps `cmd/setup.go` from having to import the per-tool packages just for label strings.)

- [ ] **Step 12.4: Update `cmd/update.go` binary update loop**

Around line 153, replace:

```go
for name, svc := range services.AllBinary() {
	if _, registered := reg.Services[name]; !registered {
		continue
	}
	latest, err := binaries.FetchLatestVersion(client, svc.Binary())
	// ...
}
```

with explicit per-tool iteration:

```go
type binaryAddonInfo struct {
	regKey string
	bin    binaries.Binary
	label  string
}
addons := []binaryAddonInfo{
	{regKey: "s3", bin: rustfs.Binary(), label: rustfs.DisplayName()},
	{regKey: "mail", bin: mailpit.Binary(), label: mailpit.DisplayName()},
}
var binaryUpdated []string
for _, a := range addons {
	if _, registered := reg.Services[a.regKey]; !registered {
		continue
	}
	latest, err := binaries.FetchLatestVersion(client, a.bin)
	if err != nil {
		ui.Subtle(fmt.Sprintf("Skipping %s: %v", a.label, err))
		continue
	}
	if !binaries.NeedsUpdate(vs, a.bin, latest) {
		continue
	}
	current := vs.Get(a.bin.Name)
	if err := ui.Step(fmt.Sprintf("Updating %s %s -> %s", a.bin.DisplayName, current, latest), func() (string, error) {
		if err := binaries.InstallBinary(client, a.bin, latest); err != nil {
			return "", err
		}
		return fmt.Sprintf("Updated %s to %s", a.bin.DisplayName, latest), nil
	}); err != nil {
		ui.Subtle(fmt.Sprintf("Could not update %s: %v", a.label, err))
		continue
	}
	vs.Set(a.bin.Name, latest)
	binaryUpdated = append(binaryUpdated, a.regKey)
}
```

Remove `services` from the import list. Add `mailpit` and `rustfs` imports.

- [ ] **Step 12.5: Delete the directories**

```bash
rm -rf internal/services internal/svchooks
```

- [ ] **Step 12.6: Verify**

```bash
gofmt -w . && go vet ./... && go build ./... && go test ./...
```
Expected: PASS. Any leftover import cycles or symbol references will fail compilation here — fix in this same task.

- [ ] **Step 12.7: Commit**

```bash
git add -A
git commit -m "refactor: delete internal/services and internal/svchooks packages"
```

---

## Task 13: Remove the `Kind` field from `registry.ServiceInstance`

**Files:**
- Modify: `internal/registry/registry.go` — drop the `Kind` field
- Create: `internal/registry/legacy_kind_test.go` — verify legacy `"kind"` JSON parses and is dropped on next save
- Verify no remaining `.Kind` accessors

- [ ] **Step 13.1: Audit remaining Kind references**

```bash
grep -rn '\.Kind\|Kind:\s*"\|"binary"\|"docker"' --include="*.go" .
```

After Task 12, only definition lines in `registry.go` plus possibly the doc comment at line 17 should remain. Note any unexpected hits.

- [ ] **Step 13.2: Write the legacy-parse test (TDD)**

Create `internal/registry/legacy_kind_test.go`:

```go
package registry

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestServiceInstance_LegacyKindFieldIsIgnored(t *testing.T) {
	const legacy = `{
		"port": 9000,
		"console_port": 9001,
		"kind": "binary",
		"enabled": true
	}`
	var inst ServiceInstance
	if err := json.Unmarshal([]byte(legacy), &inst); err != nil {
		t.Fatalf("legacy registry should still parse: %v", err)
	}
	if inst.Port != 9000 {
		t.Errorf("Port: got %d, want 9000", inst.Port)
	}
	if inst.ConsolePort != 9001 {
		t.Errorf("ConsolePort: got %d, want 9001", inst.ConsolePort)
	}
	if inst.Enabled == nil || !*inst.Enabled {
		t.Errorf("Enabled: got %v, want non-nil true", inst.Enabled)
	}
}

func TestServiceInstance_RoundTripDropsKind(t *testing.T) {
	const legacy = `{"port":9000,"kind":"binary","enabled":true}`
	var inst ServiceInstance
	if err := json.Unmarshal([]byte(legacy), &inst); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	out, err := json.Marshal(&inst)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	if strings.Contains(string(out), `"kind"`) {
		t.Errorf("re-saved JSON still contains kind: %s", out)
	}
}
```

- [ ] **Step 13.3: Run the test — expect compile failure or PASS**

```bash
go test ./internal/registry/ -run TestServiceInstance -v
```

If it compiles and passes, the `Kind` field is already missing or already `omitempty` and undefined — proceed to Step 13.4 anyway to remove the field. If it fails to compile because `Kind` still exists in the struct, that's expected before Step 13.4.

- [ ] **Step 13.4: Remove the `Kind` field**

In `internal/registry/registry.go`:

```go
type ServiceInstance struct {
	Image       string `json:"image,omitempty"`
	Port        int    `json:"port"`
	ConsolePort int    `json:"console_port,omitempty"`
	// Enabled — nil means enabled (back-compat with pre-migration registries).
	// A non-nil false means "registered but stopped".
	Enabled *bool `json:"enabled,omitempty"`
}
```

(The `Image` field is also Docker-era residue, but the spec scopes the cleanup to `Kind` only. Leave `Image` for a follow-up.)

- [ ] **Step 13.5: Run the tests**

```bash
go test ./internal/registry/ -v
```
Expected: PASS. The legacy-parse test confirms unknown JSON keys are silently dropped by Go's `encoding/json`.

- [ ] **Step 13.6: Run the full test suite**

```bash
gofmt -w . && go vet ./... && go build ./... && go test ./...
```
Expected: PASS. If anything fails because it still references `.Kind`, that's a missed cleanup — fix in this task.

- [ ] **Step 13.7: Commit**

```bash
git add -A
git commit -m "refactor(registry): remove the dead Kind field"
```

---

## Task 14: Run end-to-end verification

This is a final verification gate, not a feature task. No code changes — just confirm the refactor is complete and behavior is unchanged.

- [ ] **Step 14.1: Confirm `services` and `svchooks` are gone**

```bash
test ! -d internal/services && test ! -d internal/svchooks && echo OK
grep -rn '"github.com/prvious/pv/internal/services"' --include="*.go" . && echo "FAIL: residual import" || echo OK
grep -rn '"github.com/prvious/pv/internal/svchooks"' --include="*.go" . && echo "FAIL: residual import" || echo OK
```
Expected: three lines of "OK".

- [ ] **Step 14.2: Confirm no `BinaryService` references remain**

```bash
grep -rn "BinaryService\|LookupBinary\|AllBinary" --include="*.go" . | grep -v "// .*"
```
Expected: no matches in non-comment code.

- [ ] **Step 14.3: Confirm no `.Kind` accessors remain**

```bash
grep -rn '\.Kind\b\|Kind:\s*"binary"\|Kind:\s*"docker"' --include="*.go" .
```
Expected: no matches.

- [ ] **Step 14.4: Run full check suite**

```bash
gofmt -w . && go vet ./... && go build ./... && go test ./...
```
Expected: PASS, no warnings.

- [ ] **Step 14.5: Smoke-test the build binary**

```bash
go build -o /tmp/pv-refactor . && /tmp/pv-refactor --help | grep -E "rustfs|mailpit|s3|mail" | head
```
Expected: rustfs:* and mailpit:* command groups appear, with their `s3:*` / `mail:*` aliases listed.

- [ ] **Step 14.6: Confirm registry-shape compatibility**

If you have a non-empty `~/.pv/registry.json`, snapshot it and re-run a benign command:

```bash
cp ~/.pv/registry.json /tmp/registry-before.json
/tmp/pv-refactor list >/dev/null
diff /tmp/registry-before.json ~/.pv/registry.json | head
```
Expected: either no diff, or only the `"kind"` field removed from existing service entries (an expected first-run rewrite).

- [ ] **Step 14.7: Dispatch CI for the affected jobs**

Per CLAUDE.md dispatch conventions, this refactor touches no artifact-build logic. The default `go build && go test ./...` plus the rustfs and mailpit e2e phases on macOS are sufficient. If the branch is pushed and a CI dispatch is desired:

```bash
gh workflow run e2e.yml --ref <branch>
```

(Manual `build-artifacts.yml` is unnecessary — no artifact-build code changed.)

- [ ] **Step 14.8: No commit needed** — verification only.

---

## Self-review notes (already addressed)

- **Spec coverage:** every section of the spec is mapped to a task above (deletes → Task 12; new packages → Tasks 1, 6, 7; type relocations → Tasks 2–5; cross-cutting callers → Tasks 9–12; Kind removal → Task 13; tests → woven into each move task).
- **Placeholder scan:** every step is concrete. Task 6/7 reference Task 6/7 templates, but Task 7's deltas-from-Task-6 are listed explicitly (no "similar to Task 6" without details).
- **Type consistency:** `BuildSupervisorProcess()` (no args) is used in all references for rustfs and mailpit, matching `redis.BuildSupervisorProcess()`. `SetEnabled(bool)` is consistent throughout. `Uninstall(deleteData bool)` is consistent. `serviceKey` (lowercase, package-private) is the registry key; `ServiceKey()` is the exported accessor.
- **Risk-aware ordering:** Task 4 (`WebRoute` move) explicitly acknowledges the potential `caddy ↔ services` cycle and provides a fall-through to defer the move to Task 9 if needed. Task 12 (the bulk delete) only runs after every external import has been removed, so the directory deletion is mechanically obvious.
