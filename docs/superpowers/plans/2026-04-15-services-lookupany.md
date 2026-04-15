# `services.LookupAny` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `services.LookupAny` (a tagged-union helper that resolves a service name across both Docker and binary registries) and fix the six callsites that currently consult only the Docker registry and silently mishandle binary services (s3, mail).

**Architecture:** Purely additive helper plus six small kind-branched callsite fixes. No interface changes, no supervisor changes. Mirrors the existing private `resolveKind` lookup-order convention (binary first, Docker second).

**Tech Stack:** Go, existing `services` package, existing `cobra` command tree.

**Spec:** `docs/superpowers/specs/2026-04-15-services-lookupany-design.md`

**Branch:** `feat/services-lookupany` (already created). Each task ends with a commit; six commits total.

---

## File Structure

| Path | Action | Responsibility |
|------|--------|---------------|
| `internal/services/lookup.go` | Create | `Kind` enum + `LookupAny` function (the new public helper) |
| `internal/services/lookup_test.go` | Create | Unit tests pinning lookup-order, both-kinds, and unknown-name behavior |
| `cmd/install.go` | Modify | Replace `Lookup` validation in `parseWith` with `LookupAny` |
| `cmd/install_test.go` | Modify | Add 3 cases covering `--with=service[s3|mail|mongodb]` |
| `cmd/setup.go` | Modify | Extract `buildServiceOptions()` helper using `LookupAny`; route binary kind in post-wizard provisioning loop |
| `cmd/setup_test.go` | Create | Test `buildServiceOptions()` — verifies all binary + Docker services appear |
| `cmd/doctor.go` | Modify | Skip binary services in the per-service Docker check loop |
| `cmd/doctor_test.go` | Modify | Add a test verifying binary services don't produce "lookup_error" output |
| `internal/commands/service/env.go` | Modify | Branch on `kind` in both the all-services loop and the single-service path |
| `internal/commands/service/env_test.go` | Create | 3 cases: Docker-only registry, binary-only registry, single-service for both kinds |

---

## Task 1: Pre-flight verification

**Files:**
- None modified. Verification only.

The spec lists one verification item that must be confirmed before coding starts. It has already been spot-checked by the plan author, but the implementer should re-confirm in case the codebase moved.

- [ ] **Step 1: Confirm `service.RunAdd` accepts a single-arg slice for binary services**

Read `internal/commands/service/add.go` lines 23-62. Confirm:
- `addCmd.Args` is `cobra.RangeArgs(1, 2)` (accepts 1 or 2 args).
- The `kindBinary` branch at line 51-52 calls `addBinary(cmd.Context(), reg, binSvc)` with no `args[1]` reference.
- The `kindDocker` branch at line 53-58 only consults `args[1]` if `len(args) > 1`.

If those conditions hold, `service.RunAdd([]string{name})` (single-arg) is the correct call for binary services in `setup.go`'s post-wizard loop. Continue to Task 2.

If `addCmd.Args` has changed or the binary branch now requires a version arg, pass `[]string{name, "latest"}` instead in Task 6 — adjust the implementation code accordingly.

---

## Task 2: `services.LookupAny` + unit tests

**Files:**
- Create: `internal/services/lookup.go`
- Create: `internal/services/lookup_test.go`

Pure addition. TDD: tests first, fail, implement, pass, commit.

- [ ] **Step 1: Write the failing tests**

Create `internal/services/lookup_test.go`:

```go
package services

import (
	"strings"
	"testing"
)

func TestLookupAny_BinaryService(t *testing.T) {
	kind, binSvc, docSvc, err := LookupAny("mail")
	if err != nil {
		t.Fatalf("LookupAny(\"mail\") error = %v", err)
	}
	if kind != KindBinary {
		t.Errorf("kind = %v, want KindBinary", kind)
	}
	if binSvc == nil {
		t.Error("binSvc is nil; want non-nil for binary kind")
	}
	if docSvc != nil {
		t.Errorf("docSvc = %#v, want nil for binary kind", docSvc)
	}
}

func TestLookupAny_DockerService(t *testing.T) {
	kind, binSvc, docSvc, err := LookupAny("mysql")
	if err != nil {
		t.Fatalf("LookupAny(\"mysql\") error = %v", err)
	}
	if kind != KindDocker {
		t.Errorf("kind = %v, want KindDocker", kind)
	}
	if docSvc == nil {
		t.Error("docSvc is nil; want non-nil for docker kind")
	}
	if binSvc != nil {
		t.Errorf("binSvc = %#v, want nil for docker kind", binSvc)
	}
}

func TestLookupAny_Unknown(t *testing.T) {
	kind, binSvc, docSvc, err := LookupAny("mongodb")
	if err == nil {
		t.Fatal("LookupAny(\"mongodb\") error = nil; want non-nil for unknown name")
	}
	if kind != KindUnknown {
		t.Errorf("kind = %v, want KindUnknown", kind)
	}
	if binSvc != nil || docSvc != nil {
		t.Errorf("binSvc=%v docSvc=%v; want both nil", binSvc, docSvc)
	}
	if !strings.Contains(err.Error(), `unknown service "mongodb"`) {
		t.Errorf("error %q missing expected text", err)
	}
	if !strings.Contains(err.Error(), "available:") {
		t.Errorf("error %q missing available list", err)
	}
}

func TestLookupAny_BinaryWinsOnCollision(t *testing.T) {
	// Pin the lookup-order invariant by temporarily seeding both registries
	// with the same key. Restore via t.Cleanup so other tests are unaffected.
	const key = "collisiontest"

	// Stash any pre-existing entries (defensive — there should be none).
	prevBin, hadBin := binaryRegistry[key]
	prevDoc, hadDoc := registry[key]
	t.Cleanup(func() {
		if hadBin {
			binaryRegistry[key] = prevBin
		} else {
			delete(binaryRegistry, key)
		}
		if hadDoc {
			registry[key] = prevDoc
		} else {
			delete(registry, key)
		}
	})

	binaryRegistry[key] = &Mailpit{} // any BinaryService will do
	registry[key] = &MySQL{}         // any Service will do

	kind, binSvc, docSvc, err := LookupAny(key)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if kind != KindBinary {
		t.Errorf("kind = %v, want KindBinary (binary should win on collision)", kind)
	}
	if binSvc == nil {
		t.Error("binSvc is nil; want non-nil")
	}
	if docSvc != nil {
		t.Errorf("docSvc = %#v, want nil (binary won)", docSvc)
	}
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
go test ./internal/services/ -run LookupAny -v
```

Expected: FAIL — `undefined: LookupAny`, `undefined: KindBinary`, `undefined: KindDocker`, `undefined: KindUnknown`.

- [ ] **Step 3: Create the implementation**

Create `internal/services/lookup.go`:

```go
package services

import (
	"fmt"
	"strings"
)

// Kind classifies which registry a service name resolves to.
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

- [ ] **Step 4: Run tests to verify they pass**

```bash
gofmt -w internal/services/
go vet ./internal/services/
go test ./internal/services/ -v
```

Expected: PASS for all four `TestLookupAny_*` plus all pre-existing tests in the package.

- [ ] **Step 5: Commit**

```bash
git add internal/services/lookup.go internal/services/lookup_test.go
git commit -m "Add services.LookupAny helper

Resolves a service name across both Docker and binary registries,
returning a tagged union (Kind, BinaryService, Service, error). Binary
wins on a hypothetical collision — matching the order used by the
service:* command dispatcher's private resolveKind helper.

Closes the gap that left several callsites only consulting the Docker
registry and silently mishandling binary services (s3, mail). Per-callsite
fixes follow in subsequent commits."
```

---

## Task 3: Wire `cmd/setup.go` — extract `buildServiceOptions` + branch the post-wizard loop

**Files:**
- Modify: `cmd/setup.go`
- Create: `cmd/setup_test.go`

Two changes in `cmd/setup.go`:
1. Replace the inline service-options loop (lines 69-75) with a call to a new `buildServiceOptions()` helper.
2. Branch on `kind` in the post-wizard provisioning loop (lines 251-262).

- [ ] **Step 1: Write the failing test for `buildServiceOptions`**

Create `cmd/setup_test.go`:

```go
package cmd

import (
	"testing"
)

func TestBuildServiceOptions_IncludesBothKinds(t *testing.T) {
	opts := buildServiceOptions()
	if len(opts) == 0 {
		t.Fatal("buildServiceOptions() returned empty; want at least one option")
	}

	// Pin specific names from each registry. mysql is Docker; mail and s3 are binary.
	want := []string{"mysql", "postgres", "redis", "mail", "s3"}
	for _, name := range want {
		found := false
		for _, opt := range opts {
			if opt.value == name {
				found = true
				if opt.label == "" {
					t.Errorf("option %q has empty label", name)
				}
				break
			}
		}
		if !found {
			t.Errorf("buildServiceOptions() missing %q", name)
		}
	}
}

func TestBuildServiceOptions_LabelsUseDisplayName(t *testing.T) {
	opts := buildServiceOptions()
	for _, opt := range opts {
		// DisplayName for mail should be "Mail (Mailpit)" — not "mail".
		if opt.value == "mail" && opt.label != "Mail (Mailpit)" {
			t.Errorf("mail label = %q, want %q", opt.label, "Mail (Mailpit)")
		}
		// DisplayName for s3 should be "S3 Storage (RustFS)".
		if opt.value == "s3" && opt.label != "S3 Storage (RustFS)" {
			t.Errorf("s3 label = %q, want %q", opt.label, "S3 Storage (RustFS)")
		}
	}
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
go test ./cmd/ -run TestBuildServiceOptions -v
```

Expected: FAIL — `undefined: buildServiceOptions`.

- [ ] **Step 3: Extract the helper in `cmd/setup.go`**

In `cmd/setup.go`, replace the inline loop at lines 68-75:

Before:
```go
		// Service options.
		var svcOpts []selectOption
		for _, name := range services.Available() {
			svc, _ := services.Lookup(name)
			if svc != nil {
				svcOpts = append(svcOpts, selectOption{label: svc.DisplayName(), value: name})
			}
		}
```

After:
```go
		// Service options.
		svcOpts := buildServiceOptions()
```

Add the helper function at the bottom of `cmd/setup.go` (place it after the `setupCmd` definition; if a natural insertion point is unclear, put it just above the closing of the file):

```go
// buildServiceOptions returns the wizard's service multi-select options.
// Both Docker and binary services are listed using their DisplayName so that
// binary-only services (mail, s3) are visible in the picker.
func buildServiceOptions() []selectOption {
	names := services.Available()
	out := make([]selectOption, 0, len(names))
	for _, name := range names {
		kind, binSvc, docSvc, err := services.LookupAny(name)
		if err != nil {
			// Available() is the union of both registries; LookupAny over
			// the same names should never miss. If it does, skip silently
			// rather than break the wizard.
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

- [ ] **Step 4: Update the post-wizard provisioning loop**

In `cmd/setup.go`, replace the loop at lines 249-263:

Before:
```go
		// Spin up selected services.
		if len(selectedServices) > 0 {
			fmt.Fprintln(os.Stderr)
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
		}
```

After:
```go
		// Spin up selected services.
		if len(selectedServices) > 0 {
			fmt.Fprintln(os.Stderr)
			for _, name := range selectedServices {
				kind, _, docSvc, lookupErr := services.LookupAny(name)
				if lookupErr != nil {
					ui.Fail(fmt.Sprintf("Service %s: %v", name, lookupErr))
					continue
				}
				var svcArgs []string
				switch kind {
				case services.KindBinary:
					// Binary services are unversioned; service:add ignores any
					// explicit version (verified in Task 1).
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
		}
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
gofmt -w cmd/
go vet ./cmd/
go test ./cmd/ -run TestBuildServiceOptions -v
go test ./cmd/ -v
```

Expected: PASS for both `TestBuildServiceOptions_*` tests, plus all pre-existing `cmd/` tests.

- [ ] **Step 6: Commit**

```bash
git add cmd/setup.go cmd/setup_test.go
git commit -m "setup.go: list binary services in wizard and provision them

Extracts buildServiceOptions() so the wizard's service multi-select
includes both Docker and binary services (mail, s3). The post-wizard
provisioning loop now branches on kind and calls service:add with the
appropriate arg form for each kind — the binary branch uses the
single-arg form which routes through resolveKind correctly.

Fixes two of the six silent failures called out in the mailpit PR review."
```

---

## Task 4: Wire `internal/commands/service/env.go` — extract dispatch helper + branch on kind

**Files:**
- Modify: `internal/commands/service/env.go`
- Create: `internal/commands/service/env_test.go`

Both call sites in `env.go` need the same kind-branch shape (calling the right `EnvVars` signature for each kind). To avoid duplication and gain a real test surface, extract the dispatch into a small helper `envVarsFor` that **the production code itself uses** — so the tests pin the production behavior, not a parallel implementation.

- [ ] **Step 1: Write the failing tests**

Create `internal/commands/service/env_test.go`:

```go
package service

import (
	"strings"
	"testing"
)

func TestEnvVarsFor_BinaryService(t *testing.T) {
	got, err := envVarsFor("mail", "anyproject", 0)
	if err != nil {
		t.Fatalf("envVarsFor(\"mail\") error = %v", err)
	}
	if got["MAIL_MAILER"] != "smtp" {
		t.Errorf("MAIL_MAILER = %q, want smtp", got["MAIL_MAILER"])
	}
	if got["MAIL_HOST"] != "127.0.0.1" {
		t.Errorf("MAIL_HOST = %q, want 127.0.0.1", got["MAIL_HOST"])
	}
}

func TestEnvVarsFor_DockerService(t *testing.T) {
	// MySQL.EnvVars takes (projectName, port) and uses both. Pass a non-default
	// port to verify the port arg is consulted — a regression that always
	// passed 0 in the docker branch would silently set DB_PORT=0.
	got, err := envVarsFor("mysql", "anyproject", 3306)
	if err != nil {
		t.Fatalf("envVarsFor(\"mysql\") error = %v", err)
	}
	if got["DB_PORT"] != "3306" {
		t.Errorf("DB_PORT = %q, want 3306", got["DB_PORT"])
	}
}

func TestEnvVarsFor_Unknown(t *testing.T) {
	_, err := envVarsFor("mongodb", "anyproject", 0)
	if err == nil {
		t.Fatal("expected error for unknown service")
	}
	if !strings.Contains(err.Error(), `unknown service "mongodb"`) {
		t.Errorf("error %q missing expected text", err)
	}
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
go test ./internal/commands/service/ -run TestEnvVarsFor -v
```

Expected: FAIL — `undefined: envVarsFor`.

- [ ] **Step 3: Add the `envVarsFor` helper in `env.go`**

In `internal/commands/service/env.go`, add the helper function (place it just above `printEnvVars` at the bottom of the file):

```go
// envVarsFor resolves a service name across both registries and returns the
// .env keys/values it injects into a linked project. Used by both the
// all-services and single-service code paths in envCmd so the per-kind
// dispatch lives in one place.
//
// Binary services have EnvVars(projectName) — port is fixed by the binary
// itself. Docker services have EnvVars(projectName, port) — port comes from
// the registry.ServiceInstance.
func envVarsFor(svcName, projectName string, port int) (map[string]string, error) {
	kind, binSvc, docSvc, err := services.LookupAny(svcName)
	if err != nil {
		return nil, err
	}
	switch kind {
	case services.KindBinary:
		return binSvc.EnvVars(projectName), nil
	case services.KindDocker:
		return docSvc.EnvVars(projectName, port), nil
	}
	return nil, fmt.Errorf("unexpected kind %v for %q", kind, svcName)
}
```

- [ ] **Step 4: Update `env.go` — all-services loop**

In `internal/commands/service/env.go`, replace lines 43-52:

Before:
```go
			fmt.Fprintln(os.Stderr)
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
			fmt.Fprintln(os.Stderr)
			for key, instance := range svcs {
				svcName, _ := services.ParseServiceKey(key)
				envVars, err := envVarsFor(svcName, projectName, instance.Port)
				if err != nil {
					ui.Subtle(fmt.Sprintf("Skipping unknown service %q", svcName))
					continue
				}
				printEnvVars(key, envVars)
			}
```

- [ ] **Step 5: Update `env.go` — single-service path**

In `internal/commands/service/env.go`, replace lines 70-78:

Before:
```go
		svcName, _ := services.ParseServiceKey(key)
		svc, err := services.Lookup(svcName)
		if err != nil {
			return err
		}

		envVars := svc.EnvVars(projectName, instance.Port)
		fmt.Fprintln(os.Stderr)
		printEnvVars(key, envVars)
```

After:
```go
		svcName, _ := services.ParseServiceKey(key)
		envVars, err := envVarsFor(svcName, projectName, instance.Port)
		if err != nil {
			return err
		}

		fmt.Fprintln(os.Stderr)
		printEnvVars(key, envVars)
```

- [ ] **Step 6: Run tests + build**

```bash
gofmt -w internal/commands/service/
go vet ./internal/commands/service/
go test ./internal/commands/service/ -v
go build ./...
```

Expected: PASS for `TestEnvVarsFor_*`, plus all existing `internal/commands/service/` tests. Build clean.

- [ ] **Step 7: Commit**

```bash
git add internal/commands/service/env.go internal/commands/service/env_test.go
git commit -m "service:env: print env vars for binary services correctly

Extracts an envVarsFor helper that resolves the service name via
LookupAny and dispatches to the right EnvVars signature for each kind
(BinaryService.EnvVars(projectName) vs Service.EnvVars(projectName,
port)). Both call sites in envCmd now go through it, eliminating the
duplicated branching and giving us a real test surface.

Previously \"pv service:env mail\" hard-errored and \"pv service:env\"
with no args printed \"Skipping unknown service\" for each registered
binary service.

Fixes two of the six silent failures from the mailpit PR review."
```

---

## Task 5: Wire `cmd/doctor.go` — skip binary services in the per-service Docker check loop

**Files:**
- Modify: `cmd/doctor.go`
- Modify: `cmd/doctor_test.go`

The doctor loop (around line 616) iterates every registered service and asks Docker if its container is running. For binary services there's no container; the existing `lookup_error` branch wrongly reports them as "unknown service type — registry may be out of date."

Per the spec's design decision (option B): skip binary services entirely. They have first-class observability via `pv service:list` / `pv service:status` / `pv service:logs`. Doctor's job is cross-cutting infra health.

- [ ] **Step 1: Write the failing test**

Add to `cmd/doctor_test.go` (the file exists; append the new test function at the bottom):

```go
// TestDoctor_SkipsBinaryServices verifies that registering a binary service
// does NOT add a "lookup_error" / "unknown service type" entry to doctor's
// output. Binary services have their own observability and are intentionally
// skipped by the doctor's Docker-container check.
func TestDoctor_SkipsBinaryServices(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := config.EnsureDirs(); err != nil {
		t.Fatalf("EnsureDirs() error = %v", err)
	}

	// Register a binary service in the registry on disk.
	reg, err := registry.Load()
	if err != nil {
		t.Fatalf("registry.Load() error = %v", err)
	}
	tru := true
	reg.Services["mail"] = &registry.ServiceInstance{
		Kind:    "binary",
		Port:    1025,
		Enabled: &tru,
	}
	if err := reg.Save(); err != nil {
		t.Fatalf("reg.Save() error = %v", err)
	}

	// Capture stderr output from doctor.
	r, w, pipeErr := os.Pipe()
	if pipeErr != nil {
		t.Fatalf("os.Pipe() error = %v", pipeErr)
	}
	prevStderr := os.Stderr
	os.Stderr = w

	cmd := newDoctorCmd()
	cmd.SetArgs([]string{"doctor"})
	_ = cmd.Execute()

	w.Close()
	os.Stderr = prevStderr

	buf := make([]byte, 64*1024)
	n, _ := r.Read(buf)
	output := string(buf[:n])

	if strings.Contains(output, "lookup_error") {
		t.Errorf("doctor output should not contain \"lookup_error\" for binary service; got:\n%s", output)
	}
	if strings.Contains(output, "unknown service type") {
		t.Errorf("doctor output should not contain \"unknown service type\" for binary service; got:\n%s", output)
	}
	if strings.Contains(output, "registry may be out of date") {
		t.Errorf("doctor output should not contain \"registry may be out of date\" for binary service; got:\n%s", output)
	}
}
```

If `cmd/doctor_test.go` does not already import `strings`, add it to the import block. The other imports (`os`, `path/filepath`, `testing`, `config`, `registry`, `cobra`) are already present from the existing tests. Confirm via `head -15 cmd/doctor_test.go`.

- [ ] **Step 2: Run tests to verify they fail**

```bash
go test ./cmd/ -run TestDoctor_SkipsBinaryServices -v
```

Expected: FAIL — the doctor will produce a `lookup_error` or `unknown service type` entry for the registered `mail` binary service because the current code iterates every registered service through the Docker-check path.

(Note: doctor's overall command may exit non-zero due to other infra checks failing in the test env. That's fine — the assertions only inspect output text, not exit code.)

- [ ] **Step 3: Update `cmd/doctor.go`**

Find the loop around line 616. Add the binary-skip block at the top of the loop body.

Before:
```go
	for key, svc := range svcs {
		svcName, version := services.ParseServiceKey(key)

		status := "unknown"
		if engine != nil {
			svcDef, lookupErr := services.Lookup(svcName)
			if lookupErr != nil {
				status = "lookup_error"
			} else {
```

After:
```go
	for key, svc := range svcs {
		svcName, version := services.ParseServiceKey(key)

		// Skip binary services: they have no Docker container to probe.
		// Binary supervision health is reported via `pv service:list` and
		// `pv service:status`, which read the daemon's status snapshot
		// directly. Including them here would couple doctor to the daemon's
		// internal status format and produce misleading "lookup_error"
		// output for healthy services (mail, s3).
		if kind, _, _, _ := services.LookupAny(svcName); kind == services.KindBinary {
			continue
		}

		status := "unknown"
		if engine != nil {
			svcDef, lookupErr := services.Lookup(svcName)
			if lookupErr != nil {
				status = "lookup_error"
			} else {
```

- [ ] **Step 4: Run tests + build**

```bash
gofmt -w cmd/
go vet ./cmd/
go test ./cmd/ -run TestDoctor -v
go build ./...
```

Expected: PASS for `TestDoctor_SkipsBinaryServices` plus all pre-existing `TestDoctor_*` tests.

- [ ] **Step 5: Commit**

```bash
git add cmd/doctor.go cmd/doctor_test.go
git commit -m "doctor: skip binary services in per-service Docker check

Doctor previously reported every registered binary service (mail, s3)
as \"unknown service type — registry may be out of date\" because the
loop fed every name through services.Lookup, which only consults the
Docker registry. Binary services have first-class observability via
pv service:list / pv service:status / pv service:logs; doctor's job
is cross-cutting infra health, not per-binary-service supervision
health.

Fixes one of the six silent failures from the mailpit PR review."
```

---

## Task 6: Wire `cmd/install.go` — `--with=service[…]` validation accepts binary services

**Files:**
- Modify: `cmd/install.go`
- Modify: `cmd/install_test.go`

The smallest fix: replace one `services.Lookup` call in `parseWith` with `services.LookupAny`.

- [ ] **Step 1: Write the failing tests**

Append to `cmd/install_test.go` (after the existing `TestParseWith_*` tests; before `TestInstallCmd_AlreadyInstalled`):

```go
func TestParseWith_BinaryServiceS3(t *testing.T) {
	spec, err := parseWith("service[s3]")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(spec.services) != 1 {
		t.Fatalf("expected 1 service, got %d", len(spec.services))
	}
	if spec.services[0].name != "s3" {
		t.Errorf("service[0].name = %q, want s3", spec.services[0].name)
	}
}

func TestParseWith_BinaryServiceMail(t *testing.T) {
	spec, err := parseWith("service[mail]")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(spec.services) != 1 {
		t.Fatalf("expected 1 service, got %d", len(spec.services))
	}
	if spec.services[0].name != "mail" {
		t.Errorf("service[0].name = %q, want mail", spec.services[0].name)
	}
}

func TestParseWith_UnknownServiceMongodb(t *testing.T) {
	_, err := parseWith("service[mongodb]")
	if err == nil {
		t.Fatal("expected error for unknown service")
	}
	if !strings.Contains(err.Error(), `unknown service "mongodb"`) {
		t.Errorf("error %q missing expected text", err)
	}
}
```

If `cmd/install_test.go` does not already import `strings`, add it to the import block (the existing test file uses `os`, `path/filepath`, `testing`, `cobra` — `strings` is new). Confirm via `head -10 cmd/install_test.go`.

- [ ] **Step 2: Run tests to verify they fail**

```bash
go test ./cmd/ -run TestParseWith_BinaryService -v
go test ./cmd/ -run TestParseWith_UnknownServiceMongodb -v
```

Expected: FAIL for the two binary-service tests — `services.Lookup("s3")` and `services.Lookup("mail")` return errors today, so `parseWith` rejects them. The `mongodb` test should already PASS since unknown is unknown.

- [ ] **Step 3: Update `parseWith` in `cmd/install.go`**

In `cmd/install.go`, replace the validation at line 61-63:

Before:
```go
			if _, err := services.Lookup(s.name); err != nil {
				return spec, fmt.Errorf("unknown service %q in --with (available: %s)", s.name, strings.Join(services.Available(), ", "))
			}
```

After:
```go
			if k, _, _, _ := services.LookupAny(s.name); k == services.KindUnknown {
				return spec, fmt.Errorf("unknown service %q in --with (available: %s)", s.name, strings.Join(services.Available(), ", "))
			}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
gofmt -w cmd/
go vet ./cmd/
go test ./cmd/ -run TestParseWith -v
go build ./...
```

Expected: PASS for all `TestParseWith_*` tests including the three new ones.

- [ ] **Step 5: Run full test suite**

```bash
go test ./...
```

Expected: every package passes. This is the last task — confirm the whole tree is clean before committing.

- [ ] **Step 6: Commit**

```bash
git add cmd/install.go cmd/install_test.go
git commit -m "install --with=service[…] accepts binary services

parseWith now uses services.LookupAny, so --with=service[s3] and
--with=service[mail] no longer falsely error as 'unknown service'.
Validation behavior for genuinely unknown names is unchanged.

Closes the last of the six silent failures from the mailpit PR review."
```

---

## Parallelization Guide

Linear. Each task depends on Task 2 (`LookupAny` must exist before any callsite consumes it). After Task 2, Tasks 3-6 are technically independent and could run in parallel — but they all touch test files in `cmd/`, and a parallel `gofmt -w cmd/` could race with another task's edit. Safer to keep them sequential.

Total: 6 commits, all on branch `feat/services-lookupany`. Whole branch should land in one PR.
