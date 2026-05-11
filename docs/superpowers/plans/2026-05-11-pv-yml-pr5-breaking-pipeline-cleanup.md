# pv.yml PR 5 — Breaking: delete the legacy pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `pv.yml` mandatory and remove the entire legacy pipeline that PR 2/3 kept alive as a compat fallback. After this PR: `pv link` without a pv.yml errors with "run `pv init`" (PR 4's tool); the auto-detect step, the Laravel env writer, the 6 hardcoded setup steps, and the two env-writer steps (`SetAppURLStep` / `SetViteTLSStep`) are all gone.

**Architecture:** Surgical deletion across `internal/automation/steps/`, `internal/laravel/`, `internal/config/settings.go`, `internal/automation/pipeline.go`, `cmd/link.go`. The shared helpers used by both the auto-detect step and the pv.yml-driven step (`findServiceByName`, `bindProjectPostgres`, `bindProjectMysql`, `bindProjectService`) need to MOVE to the pv.yml-driven step's file first, since the auto-detect file is going away. Existing `cmd/link_test.go` tests need fixture pv.yml files written into their tempdirs. README gets a migration section.

**Tech Stack:** Go 1.x — pure deletion + minor refactor.

**Spec reference:** `docs/superpowers/specs/2026-05-10-pv-yml-explicit-config-design.md` (the "What's removed" table).

**Base commit:** `bde0e43` on `main`. Branch: `feat/pvyml-breaking-pipeline-cleanup`.

---

## Deletion surface (the big picture)

| File | What dies | What survives |
|---|---|---|
| `internal/automation/steps/detect_services.go` | Entire file | The 4 helpers (`findServiceByName`, `bindProjectService`, `bindProjectPostgres`, `bindProjectMysql`) MOVE to `apply_pvyml_services.go` first |
| `internal/automation/steps/detect_services_test.go` | Entire file | — |
| `internal/laravel/steps.go` | 8 step types: `DetectServicesStep`, `CopyEnvStep`, `ComposerInstallStep`, `GenerateKeyStep`, `InstallOctaneStep`, `CreateDatabaseStep`, `RunMigrationsStep`, `SetAppURLStep`, `SetViteTLSStep` (NOTE: 9 — the laravel DetectServicesStep is the 1st) | `isLaravel` helper |
| `internal/laravel/steps_test.go` | Tests for all 9 deleted step types + the HasSetup short-circuit tests | `TestIsLaravel` |
| `internal/laravel/env.go` | `SmartEnvVars`, `UpdateProjectEnvForPostgres`, `UpdateProjectEnvForMysql`, `UpdateProjectEnvForRedis` | `ApplyFallbacks`, `FallbackMapping` (called from `internal/svchooks/` on service removal) |
| `internal/laravel/env_test.go` | Tests for the deleted UpdateProjectEnvFor* functions | `TestApplyFallbacks_*` |
| `internal/laravel/artisan.go` | `KeyGenerate`, `OctaneInstall`, `Migrate` | `RunArtisan` (if used elsewhere — verify) |
| `internal/laravel/composer.go` | `ComposerInstall` | — |
| `internal/laravel/database.go` | `ResolveDatabaseName` | — |
| Various `internal/laravel/*.go` | `HasEnvExample`, `HasEnvFile`, `ReadAppKey`, `HasOctanePackage`, `HasOctaneWorker` (verify usage) | Anything still called from outside the deleted steps |
| `internal/config/settings.go` | 10 `Automation` struct fields + their defaults | The 3 surviving gate fields (`ApplyPvYmlServices`, `ApplyPvYmlEnv`, `ApplySetup`, plus whatever else stays) |
| `internal/automation/pipeline.go` | 10 cases from `LookupGate` switch | Cases for the surviving gates |
| `cmd/link.go` | 9 step instantiations from `allSteps` slice; legacy pipeline mode | New nil-check on `projectCfg` |
| `cmd/link_test.go` | Nothing (tests stay; existing tests get fixture pv.yml files written into their tempdirs) | All existing tests |
| `cmd/setup.go` | Nothing — `pv setup` is a TUI wizard, not a pipeline runner | All logic |
| `README.md` | — | New migration section |

Approx ~1100 lines of Go deleted, ~50 lines added (the pv.yml guard + test fixtures + migration doc).

---

## Task 1: Move shared helpers from `detect_services.go` to `apply_pvyml_services.go`

**Files:**
- Modify: `internal/automation/steps/detect_services.go` (helpers removed; type kept temporarily until Task 2)
- Modify: `internal/automation/steps/apply_pvyml_services.go` (helpers added)
- (Possibly modify call sites — verify there are no consumers besides `apply_pvyml_services.go` for these 4 helpers)

The 4 helpers in `detect_services.go` are used by both the auto-detect step AND by `ApplyPvYmlServicesStep`. Move them BEFORE deleting the auto-detect step so the surviving caller keeps working.

- [ ] **Step 1.1: Locate the helpers**

```bash
grep -n "^func findServiceByName\|^func bindProjectService\|^func bindProjectPostgres\|^func bindProjectMysql" internal/automation/steps/detect_services.go
```

Note the exact line ranges of each function. Read the full bodies — they may share state or have intra-file dependencies.

- [ ] **Step 1.2: Verify no other callers**

```bash
grep -rn "findServiceByName\|bindProjectService\|bindProjectPostgres\|bindProjectMysql" --include="*.go" . | grep -v _test.go | grep -v detect_services.go | grep -v apply_pvyml_services.go
```

Expected: empty. If non-empty, every match needs to keep working after the helpers move — flag and adapt.

- [ ] **Step 1.3: Move helpers**

Cut the 4 helper functions from `internal/automation/steps/detect_services.go`. Paste them at the BOTTOM of `internal/automation/steps/apply_pvyml_services.go` (below the `Run` method).

Both files are in the same package (`steps`), so the helpers remain accessible to whatever still calls them in `detect_services.go` until Task 2 deletes that file.

- [ ] **Step 1.4: Verify**

```
gofmt -w internal/automation/steps/
go vet ./internal/automation/steps/...
go build ./...
go test ./...
```

Expected: every test still passes (the detect_services_test.go tests still work because the helpers are accessible in the same package).

- [ ] **Step 1.5: Commit**

```bash
git add internal/automation/steps/detect_services.go internal/automation/steps/apply_pvyml_services.go
git commit -m "$(cat <<'EOF'
refactor(steps): move shared binding helpers to apply_pvyml_services

The four helpers (findServiceByName, bindProjectService,
bindProjectPostgres, bindProjectMysql) live in detect_services.go
today but are also used by ApplyPvYmlServicesStep. Moving them now
ahead of PR 5's deletion of the auto-detect step so the surviving
caller keeps working. No behavior change.
EOF
)"
```

---

## Task 2: Delete `steps.DetectServicesStep` (the auto-detect)

**Files:**
- Delete: `internal/automation/steps/detect_services.go`
- Delete: `internal/automation/steps/detect_services_test.go`
- Modify: `cmd/link.go` (remove `&steps.DetectServicesStep{}` from pipeline)
- Modify: `internal/automation/pipeline.go` (remove `case "detect_services":` from `LookupGate`)
- Modify: `internal/config/settings.go` (remove `DetectServices AutoMode` field + its default + its validation in `applyAutomationDefaults`)

- [ ] **Step 2.1: Delete the files**

```bash
rm internal/automation/steps/detect_services.go
rm internal/automation/steps/detect_services_test.go
```

After Task 1's helper move, `detect_services.go` contained only the `DetectServicesStep` type + methods. Deleting the file removes them.

- [ ] **Step 2.2: Remove from pipeline in `cmd/link.go`**

Find the `allSteps := []automation.Step{...}` literal. Delete the line `&steps.DetectServicesStep{},`.

- [ ] **Step 2.3: Remove `LookupGate` case in `internal/automation/pipeline.go`**

Find `func LookupGate(...)` and the switch. Delete the entire `case "detect_services":` arm. Preserve the surrounding cases verbatim.

- [ ] **Step 2.4: Remove `Automation.DetectServices` field**

In `internal/config/settings.go`:
1. Find the `Automation` struct. Delete the `DetectServices AutoMode \`yaml:"detect_services,omitempty"\`` field.
2. Find `DefaultAutomation()` (or wherever defaults are seeded). Delete the line that sets `DetectServices: AutoOn` (or whatever value).
3. Find `applyAutomationDefaults` (or equivalent). Delete the validation block for `DetectServices`.

- [ ] **Step 2.5: Verify**

```
gofmt -w .
go vet ./...
go build ./...
go test ./...
```

Expected: all clean. If any test breaks, it's referencing a deleted symbol — read the failure and update or delete the test.

- [ ] **Step 2.6: Commit**

```bash
git add -A
git status --short  # confirm deletions look right
git commit -m "$(cat <<'EOF'
refactor(steps): delete auto-detect DetectServicesStep

PR 2 wired pv.yml-driven service binding while keeping the legacy
auto-detect path as a fallback. PR 4 shipped `pv init` so users on
existing projects have a migration tool. Now that pv.yml is the
contract, the legacy heuristic-based binding is gone:

- internal/automation/steps/detect_services.go deleted (the four
  helper functions moved to apply_pvyml_services.go in the prior
  commit so ApplyPvYmlServicesStep keeps working).
- detect_services_test.go deleted.
- "detect_services" gate removed from LookupGate.
- Automation.DetectServices struct field removed (old settings.yml
  files with the field deserialize cleanly — Go ignores unknown
  fields).
EOF
)"
```

---

## Task 3: Delete `laravel.DetectServicesStep` (the Laravel env writer) + `UpdateProjectEnvFor*` + `SmartEnvVars`

**Files:**
- Modify: `internal/laravel/steps.go` (delete `DetectServicesStep` type + methods)
- Modify: `internal/laravel/env.go` (delete `SmartEnvVars`, `UpdateProjectEnvForPostgres`, `UpdateProjectEnvForMysql`, `UpdateProjectEnvForRedis`)
- Modify: `internal/laravel/steps_test.go` (delete tests for the deleted step)
- Modify: `internal/laravel/env_test.go` (delete tests for the deleted helpers)
- Modify: `cmd/link.go` (remove `&laravel.DetectServicesStep{}` from pipeline)
- Modify: `internal/automation/pipeline.go` (remove `case "update_env_on_service":` from `LookupGate`)
- Modify: `internal/config/settings.go` (remove `ServiceEnvUpdate` field + default + validation)

This deletes the LARAVEL-specific env writer that PR 2 kept gated on `HasAnyEnv()`. With pv.yml now mandatory and `ApplyPvYmlEnvStep` handling all env writes, this code is dead.

`ApplyFallbacks` and `FallbackMapping` STAY (they're called from service-removal hooks in `internal/svchooks/`, not from the pipeline).

- [ ] **Step 3.1: Find every UpdateProjectEnvFor* caller outside the step**

```bash
grep -rn "UpdateProjectEnvForPostgres\|UpdateProjectEnvForMysql\|UpdateProjectEnvForRedis" --include="*.go" . | grep -v _test.go | grep -v internal/laravel/env.go
```

Expected: only `internal/laravel/steps.go` (the `DetectServicesStep.Run` body). If there are other callers (service-install commands, etc.), they need to be updated or the functions need to be partially preserved.

Same for `SmartEnvVars`:

```bash
grep -rn "SmartEnvVars" --include="*.go" .
```

Expected: only `internal/laravel/env.go` (the function itself) and `internal/laravel/steps.go` (the caller in `DetectServicesStep.Run`). If used elsewhere, flag.

- [ ] **Step 3.2: Delete the laravel `DetectServicesStep` type + methods**

In `internal/laravel/steps.go`, find:

```go
type DetectServicesStep struct{}

func (s *DetectServicesStep) Label() string { ... }
func (s *DetectServicesStep) Gate() string { ... }
func (s *DetectServicesStep) Critical() bool { ... }
func (s *DetectServicesStep) Verbose() bool { ... }
func (s *DetectServicesStep) ShouldRun(ctx *automation.Context) bool { ... }
func (s *DetectServicesStep) Run(ctx *automation.Context) (string, error) { ... }
```

Delete the type and all 6 methods. Preserve the `// pv.yml setup: declared — ...` takeover-policy comment if it lives above another step (it was added in PR 3 above `CopyEnvStep.ShouldRun` — survives until Task 4).

- [ ] **Step 3.3: Delete the env writer functions from `internal/laravel/env.go`**

Find and delete:
- `func SmartEnvVars(...)` (about 15 lines)
- `func UpdateProjectEnvForPostgres(...)` (about 15 lines)
- `func UpdateProjectEnvForMysql(...)` (about 15 lines)
- `func UpdateProjectEnvForRedis(...)` (about 12 lines)

Keep `ApplyFallbacks` and `FallbackMapping`. Verify the file still compiles standalone.

- [ ] **Step 3.4: Delete the corresponding tests**

In `internal/laravel/steps_test.go`: find every `Test*DetectServicesStep*` and `TestLaravelDetectServices_*` test. Delete those functions.

In `internal/laravel/env_test.go`: find every `TestUpdateProjectEnvFor*` and `TestSmartEnvVars*` test. Delete. Keep `TestApplyFallbacks_*`.

- [ ] **Step 3.5: Remove from pipeline + gates**

In `cmd/link.go`, delete `&laravel.DetectServicesStep{},` from `allSteps`.

In `internal/automation/pipeline.go`, delete `case "update_env_on_service":` from `LookupGate` (or whatever the gate string is — the explorer flagged `"update_env_on_service"` mapped to `a.ServiceEnvUpdate`).

In `internal/config/settings.go`, delete:
- `ServiceEnvUpdate AutoMode` field from `Automation` struct
- Its default in `DefaultAutomation()`
- Its validation in `applyAutomationDefaults`

- [ ] **Step 3.6: Verify**

```
gofmt -w .
go vet ./...
go build ./...
go test ./...
```

Expected: clean. If test failures reference deleted symbols, those tests need deletion too — they're testing dead code.

- [ ] **Step 3.7: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
refactor(laravel): delete DetectServicesStep + UpdateProjectEnvFor* helpers

The Laravel env writer that PR 2 kept gated on HasAnyEnv() is now
dead — ApplyPvYmlEnvStep handles every env write under the explicit
pv.yml contract.

- internal/laravel/steps.go: DetectServicesStep type + 6 methods deleted.
- internal/laravel/env.go: SmartEnvVars, UpdateProjectEnvForPostgres,
  UpdateProjectEnvForMysql, UpdateProjectEnvForRedis deleted.
  ApplyFallbacks and FallbackMapping kept (still called from
  internal/svchooks/ on service removal).
- internal/laravel/steps_test.go + env_test.go: tests for the
  deleted symbols removed.
- cmd/link.go: pipeline entry removed.
- pipeline.go LookupGate: "update_env_on_service" case removed.
- settings.go: Automation.ServiceEnvUpdate field removed.
EOF
)"
```

---

## Task 4: Delete the 6 hardcoded laravel pipeline steps

**Files:**
- Modify: `internal/laravel/steps.go` — delete 6 step types
- Modify: `internal/laravel/steps_test.go` — delete corresponding tests + HasSetup short-circuit tests
- Possibly delete or modify: `internal/laravel/composer.go`, `internal/laravel/artisan.go`, `internal/laravel/database.go` (depending on whether their helpers have non-step callers)
- Modify: `cmd/link.go` — delete 6 step instantiations from `allSteps`
- Modify: `internal/automation/pipeline.go` — delete 6 gate cases
- Modify: `internal/config/settings.go` — delete 6 Automation fields + defaults

Steps deleted: `CopyEnvStep`, `ComposerInstallStep`, `GenerateKeyStep`, `InstallOctaneStep`, `CreateDatabaseStep`, `RunMigrationsStep`.

- [ ] **Step 4.1: Audit helper usage for each step**

Run these one at a time and note where the helper is called from outside the soon-to-be-deleted step:

```bash
# ComposerInstall
grep -rn "ComposerInstall\b" --include="*.go" . | grep -v _test.go
# KeyGenerate
grep -rn "KeyGenerate\b" --include="*.go" . | grep -v _test.go
# OctaneInstall
grep -rn "OctaneInstall\b" --include="*.go" . | grep -v _test.go
# Migrate
grep -rn "func Migrate\|laravel\.Migrate" --include="*.go" . | grep -v _test.go
# ResolveDatabaseName
grep -rn "ResolveDatabaseName" --include="*.go" . | grep -v _test.go
# HasEnvExample, HasEnvFile, ReadAppKey, HasOctanePackage, HasOctaneWorker
grep -rn "HasEnvExample\|HasEnvFile\|ReadAppKey\|HasOctanePackage\|HasOctaneWorker" --include="*.go" . | grep -v _test.go
```

For each helper, build a list:
- If the ONLY non-test caller is the soon-to-be-deleted step, the helper is dead — delete it.
- If there's another caller (e.g., `pv install` calls `ComposerInstall` to bootstrap), the helper survives — keep it.

Most are likely deletable. The audit takes ~5 minutes.

- [ ] **Step 4.2: Delete the 6 step types from `internal/laravel/steps.go`**

Each step is roughly:
```go
type <Name>Step struct{}

func (s *<Name>Step) Label() string  { ... }
func (s *<Name>Step) Gate() string   { ... }
func (s *<Name>Step) Critical() bool { ... }
func (s *<Name>Step) Verbose() bool  { ... }
func (s *<Name>Step) ShouldRun(ctx *automation.Context) bool { ... }
func (s *<Name>Step) Run(ctx *automation.Context) (string, error) { ... }
```

Delete all 6.

The takeover-policy comment that PR 3 added above `CopyEnvStep.ShouldRun` goes with the step it documents.

- [ ] **Step 4.3: Delete the dead helpers**

For every helper the Step 4.1 audit determined is dead, delete it. Likely all of:
- `internal/laravel/composer.go` — `ComposerInstall` function (and maybe delete the whole file if it's the only function)
- `internal/laravel/artisan.go` — `KeyGenerate`, `OctaneInstall`, `Migrate` (preserve `RunArtisan` if used elsewhere)
- `internal/laravel/database.go` — `ResolveDatabaseName` (preserve if used elsewhere; likely deletable)
- Helpers in `env.go` or `compositor.go` or wherever: `HasEnvExample`, `HasEnvFile`, `ReadAppKey`, `HasOctanePackage`, `HasOctaneWorker`

After deletion, run `go build ./...` after each chunk so you catch missed dependencies early.

- [ ] **Step 4.4: Delete the corresponding tests**

In `internal/laravel/steps_test.go`, delete every test function whose name starts with:
- `TestCopyEnvStep_*`
- `TestComposerInstallStep_*`
- `TestGenerateKeyStep_*`
- `TestInstallOctaneStep_*`
- `TestCreateDatabaseStep_*`
- `TestRunMigrationsStep_*`

Also delete the HasSetup short-circuit tests added in PR 3:
- `TestCopyEnvStep_SkipsWhenSetupDeclared`
- `TestGenerateKeyStep_SkipsWhenSetupDeclared`
- `TestInstallOctaneStep_SkipsWhenSetupDeclared`
- `TestComposerInstallStep_SkipsWhenSetupDeclared`
- `TestCreateDatabaseStep_SkipsWhenSetupDeclared`
- `TestRunMigrationsStep_SkipsWhenSetupDeclared`

In `internal/laravel/artisan_test.go`, `composer_test.go`, etc. (if they exist): delete tests for the helpers that just got deleted.

- [ ] **Step 4.5: Remove from pipeline + gates + settings**

In `cmd/link.go`, delete the 6 step entries:
```go
&laravel.CopyEnvStep{},
&laravel.ComposerInstallStep{},
&laravel.GenerateKeyStep{},
&laravel.InstallOctaneStep{},
&laravel.CreateDatabaseStep{},
&laravel.RunMigrationsStep{},
```

In `internal/automation/pipeline.go LookupGate`, delete the 6 cases:
```go
case "copy_env":         return a.CopyEnv
case "composer_install": return a.ComposerInstall
case "generate_key":     return a.GenerateKey
case "install_octane":   return a.InstallOctane
case "create_database":  return a.CreateDatabase
case "run_migrations":   return a.RunMigrations
```

(Verify exact gate strings by reading the file — they may differ slightly.)

In `internal/config/settings.go`, delete the 6 `Automation` fields + their defaults + their validation:
- `CopyEnv`
- `ComposerInstall`
- `GenerateKey`
- `InstallOctane`
- `CreateDatabase`
- `RunMigrations`

- [ ] **Step 4.6: Verify**

```
gofmt -w .
go vet ./...
go build ./...
go test ./...
```

If `go test ./...` fails for a test that lives in `internal/laravel/` and references a deleted helper, delete that test too — it's testing dead code.

- [ ] **Step 4.7: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
refactor(laravel): delete 6 hardcoded pipeline steps

PR 3's setup: runner replaced these in practice. PR 4's pv init
generates a default setup: block per project type so existing users
have a migration tool. With pv.yml mandatory after PR 5, these
steps are dead:

- CopyEnvStep, ComposerInstallStep, GenerateKeyStep,
  InstallOctaneStep, CreateDatabaseStep, RunMigrationsStep types
  deleted from internal/laravel/steps.go.
- Dead helpers cleaned up: ComposerInstall, KeyGenerate,
  OctaneInstall, Migrate, ResolveDatabaseName, HasEnvExample,
  HasEnvFile, ReadAppKey, HasOctanePackage, HasOctaneWorker —
  audited for non-step callers first; anything still used elsewhere
  survives.
- Corresponding tests removed (including the PR 3 HasSetup
  short-circuit tests, which become tautological once the steps
  themselves are gone).
- cmd/link.go pipeline shrinks by 6 entries.
- pipeline.go LookupGate loses 6 cases.
- settings.go Automation struct loses 6 fields + defaults.
EOF
)"
```

---

## Task 5: Delete `SetAppURLStep` + `SetViteTLSStep`

**Files:**
- Modify: `internal/laravel/steps.go` — delete 2 step types
- Modify: `internal/laravel/steps_test.go` — delete corresponding tests
- Modify: `cmd/link.go` — delete 2 entries from `allSteps`
- Modify: `internal/automation/pipeline.go` — delete 2 cases from `LookupGate`
- Modify: `internal/config/settings.go` — delete 2 `Automation` fields

Users get the equivalent behavior via pv.yml top-level `env:`:
```yaml
env:
  APP_URL: "{{ .site_url }}"
  VITE_DEV_SERVER_KEY: "{{ .tls_key_path }}"
  VITE_DEV_SERVER_CERT: "{{ .tls_cert_path }}"
```

PR 4's `pv init` already generates `APP_URL: "{{ .site_url }}"` for Laravel projects. Vite TLS users add the two lines manually (the comment block PR 4 emitted in unknown/static templates hints at this).

- [ ] **Step 5.1: Delete the 2 step types**

In `internal/laravel/steps.go`, find and delete:
- `SetAppURLStep` (type + 6 methods)
- `SetViteTLSStep` (type + 6 methods)

- [ ] **Step 5.2: Delete the corresponding tests**

In `internal/laravel/steps_test.go`, delete every test function whose name starts with:
- `TestSetAppURLStep_*`
- `TestSetViteTLSStep_*`

- [ ] **Step 5.3: Remove from pipeline + gates + settings**

In `cmd/link.go`, delete:
```go
&laravel.SetAppURLStep{},
&laravel.SetViteTLSStep{},
```

In `internal/automation/pipeline.go LookupGate`, delete:
```go
case "set_app_url":  return a.SetAppURL
case "set_vite_tls": return a.SetViteTLS
```

In `internal/config/settings.go`, delete `SetAppURL` and `SetViteTLS` fields from `Automation` + their defaults + validation.

- [ ] **Step 5.4: Verify**

```
gofmt -w .
go vet ./...
go build ./...
go test ./...
```

Expected: clean.

- [ ] **Step 5.5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
refactor(laravel): delete SetAppURLStep + SetViteTLSStep

Users get the equivalent behavior via pv.yml's top-level env: block
with template values — APP_URL: "{{ .site_url }}" and
VITE_DEV_SERVER_{KEY,CERT}: "{{ .tls_{key,cert}_path }}". pv init
emits APP_URL for Laravel projects already; Vite TLS users add the
two lines per pv.yml's documented project-level template vars.

- internal/laravel/steps.go: SetAppURLStep + SetViteTLSStep deleted.
- steps_test.go: tests removed.
- cmd/link.go: 2 pipeline entries removed.
- pipeline.go: 2 LookupGate cases removed.
- settings.go: SetAppURL + SetViteTLS Automation fields removed.
EOF
)"
```

---

## Task 6: Require pv.yml in `pv link` + update existing tests

**Files:**
- Modify: `cmd/link.go` — add nil-check on `projectCfg`
- Modify: `cmd/link_test.go` — every test that calls `pv link` against a tempdir now needs a fixture pv.yml in that dir

The current `cmd/link.go` does:
```go
projectCfg, err := config.FindAndLoadProjectConfig(absPath)
if err != nil {
    return fmt.Errorf("cannot read pv.yml: %w", err)
}
```

`FindAndLoadProjectConfig` returns `(nil, nil)` when no pv.yml exists. PR 5 makes that an error.

- [ ] **Step 6.1: Add the guard**

In `cmd/link.go`, right after the `FindAndLoadProjectConfig` call (which the implementer hoisted in PR 2's review fix), add:

```go
projectCfg, err := config.FindAndLoadProjectConfig(absPath)
if err != nil {
    return fmt.Errorf("cannot read pv.yml: %w", err)
}
if projectCfg == nil {
    return fmt.Errorf("no pv.yml found at %s. Run `pv init` to generate one.", absPath)
}
```

Note: `fmt.Errorf("%s", projectCfg)` works because fang renders the error nicely. Don't use `ui.Fail` here — fang's error renderer is the right output channel.

- [ ] **Step 6.2: Add a test helper for fixture pv.yml**

In `cmd/link_test.go`, just after `writeDefaultSettings`, add:

```go
// writePvYml drops a minimal pv.yml into projDir so cmd/link's
// pv.yml-required guard is satisfied. Tests that want a specific
// pv.yml shape should write their own.
func writePvYml(t *testing.T, projDir string) {
	t.Helper()
	body := `php: "8.4"
`
	if err := os.WriteFile(filepath.Join(projDir, "pv.yml"), []byte(body), 0o644); err != nil {
		t.Fatalf("write pv.yml: %v", err)
	}
}
```

- [ ] **Step 6.3: Update every existing cmd/link test**

For every test in `cmd/link_test.go` that calls `pv link <projDir>` and currently doesn't write a pv.yml: add `writePvYml(t, projDir)` right after `projDir := t.TempDir()` (and after any other fixture writes that test does).

There are likely 4–6 tests. Examples from the explorer:
- `TestLink_ExplicitPathAndName` — writes nothing today; add `writePvYml(t, projDir)` after `projDir := t.TempDir()`.
- `TestLink_RelinkPreservesServices` — same.
- `TestLink_RelinkOverwritesStaleAliases` — this test already writes its own pv.yml; verify it does, otherwise add.

Exceptions (tests that test error paths and SHOULDN'T have a pv.yml):
- `TestLink_NonExistentPath` — uses `/does/not/exist`; no projDir to write to. No change.
- `TestLink_FileNotDir` — writes a file, not a dir. No change.

Add one NEW test that proves the guard:

```go
func TestLink_RefusesWithoutPvYml(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	writeDefaultSettings(t)

	projDir := t.TempDir()
	// Intentionally NO pv.yml.

	cmd := newLinkCmd()
	cmd.SetArgs([]string{"link", projDir})
	err := cmd.Execute()
	if err == nil {
		t.Fatal("Execute: want error when no pv.yml, got nil")
	}
	if !strings.Contains(err.Error(), "pv init") {
		t.Errorf("err = %v; want it to suggest `pv init`", err)
	}
}
```

- [ ] **Step 6.4: Verify**

```
go test ./cmd/ -v 2>&1 | tail -50
go test ./... -count=1 2>&1 | tail -10
gofmt -l .
go vet ./...
go build ./...
```

Every cmd/link test should still pass (with fixture pv.yml writes added) plus the new `TestLink_RefusesWithoutPvYml`.

- [ ] **Step 6.5: Commit**

```bash
git add cmd/link.go cmd/link_test.go
git commit -m "$(cat <<'EOF'
feat(link): require pv.yml — error with `pv init` hint when missing

The final cut in the pv.yml redesign. pv link without a pv.yml now
errors with:

  no pv.yml found at <path>. Run `pv init` to generate one.

cmd/link_test.go: existing tests get a writePvYml(t, projDir) helper
call so the guard doesn't trip them. New TestLink_RefusesWithoutPvYml
pins the guard's behavior. Error-path tests (non-existent path,
file-not-dir) are untouched.
EOF
)"
```

---

## Task 7: Migration guide in README

**Files:**
- Modify: `README.md`

Add a "Migration" section so existing users know how to move to the new model.

- [ ] **Step 7.1: Read the existing README structure**

```bash
head -80 README.md
grep -n "^##" README.md | head -20
```

Find a sensible place for the migration section — probably after a Getting Started / Usage section, before any contributor docs.

- [ ] **Step 7.2: Add the migration section**

Insert this section into README.md at the chosen location:

```markdown
## Migrating from pre-pv.yml versions

If you used pv before this release, your projects worked via auto-detection plus a hardcoded setup pipeline. Both are gone. **`pv.yml` is now the project's contract with pv.**

To migrate an existing linked project:

```bash
cd /path/to/your/project
pv init               # generates pv.yml with sensible defaults for the detected type
# review the generated file — adjust services, env, setup as needed
git add pv.yml && git commit -m "Add pv.yml"
pv link               # relinks with the new contract
```

`pv init` writes a `pv.yml` with:

- The project's PHP version (from `composer.json` `require.php` when parseable, otherwise your global default)
- A `postgresql:` or `mysql:` block when the matching engine is installed (use `--mysql` to prefer mysql when both are installed)
- A `setup:` block with `cp .env.example .env`, `composer install`, `php artisan key:generate`, and `php artisan migrate` for Laravel projects (just `composer install` for generic PHP)
- A commented-out `aliases:` line you can uncomment to add extra hostnames

Common adjustments after `pv init`:

- **No database**: remove the `postgresql:` / `mysql:` block and the `pv <engine>:db:create` + `php artisan migrate` lines from `setup:`.
- **Custom migrate command**: replace `php artisan migrate` with whatever your team uses (e.g., `php artisan x-migrate` for multi-database setups).
- **Custom env keys**: add to the top-level `env:` block or per-service `env:` map. Values can be plain strings or templates like `{{ .site_url }}`, `{{ .host }}`, `{{ .port }}`, etc. See the spec at `docs/superpowers/specs/2026-05-10-pv-yml-explicit-config-design.md` for the full template variable reference.
- **Aliases**: uncomment the `aliases:` line and add hostnames for sites that should serve from the same project (each alias gets its own TLS cert).

## What's no longer automatic

The following used to happen invisibly during `pv link`. After this release, they happen only if you ask for them in `pv.yml`:

- **Service binding from `.env` hints** (e.g., `DB_CONNECTION=pgsql` no longer auto-binds postgres) — declare the service in `pv.yml` instead.
- **Laravel-shaped env writes** (`DB_HOST`, `DB_PORT`, `CACHE_STORE`, `SESSION_DRIVER`, etc.) — declare the keys in `pv.yml`'s `env:` blocks; values come from template variables.
- **`.env.example` → `.env` copy** — put `cp .env.example .env` in `setup:`.
- **Composer install, key:generate, migrate, Octane install** — put each in `setup:`.
- **Database creation** — call `pv postgres:db:create <name>` (or mysql) from `setup:`.
- **`APP_URL` and Vite TLS env vars** — declare in pv.yml's top-level `env:`:

  ```yaml
  env:
    APP_URL: "{{ .site_url }}"
    VITE_DEV_SERVER_KEY: "{{ .tls_key_path }}"
    VITE_DEV_SERVER_CERT: "{{ .tls_cert_path }}"
  ```

The trade-off: a one-time `pv init` per project in exchange for never seeing a mystery `.env` write again.
```

- [ ] **Step 7.3: Verify**

```
# No build/test impact, just verify the markdown is valid.
grep -c "^##" README.md  # should have grown by 2 (the two new sections)
```

- [ ] **Step 7.4: Commit**

```bash
git add README.md
git commit -m "$(cat <<'EOF'
docs(readme): migration guide for the pv.yml redesign

Two new sections: "Migrating from pre-pv.yml versions" walks
existing users through the pv init → review → commit → pv link
flow; "What's no longer automatic" enumerates the behaviors that
moved from invisible-pipeline to user-declared. References the
spec doc for the full template variable reference.
EOF
)"
```

---

## Task 8: Final verification + open PR

**Files:** none modified.

- [ ] **Step 8.1: Project-wide gates**

```
gofmt -l .
go vet ./...
go test ./... -count=1
go build ./...
```
Expected: every gate clean.

- [ ] **Step 8.2: Smoke test the binary against the migration story**

```bash
go build -o /tmp/pv-pr5-smoke .

# 1. pv link without pv.yml errors with init hint
tmp=$(mktemp -d)
/tmp/pv-pr5-smoke link "$tmp" 2>&1 | head -5
echo "(^ should mention 'pv init')"

# 2. pv init writes a pv.yml
echo '{"require":{"laravel/framework":"^11.0"}}' > "$tmp/composer.json"
/tmp/pv-pr5-smoke init "$tmp"
cat "$tmp/pv.yml"
echo "---"

# 3. After init, pv link should not error on the pv.yml-missing branch.
#    (It may still fail later for environmental reasons; that's outside PR 5's concern.)
/tmp/pv-pr5-smoke link "$tmp" 2>&1 | head -20 || true
echo "(^ no longer the 'no pv.yml' error)"

rm -rf "$tmp" /tmp/pv-pr5-smoke
```

- [ ] **Step 8.3: Confirm commit count + line counts**

```
git log --oneline main..HEAD
git diff --stat main..HEAD | tail -3
```

Expected: 7 commits (one plan + Tasks 1-7 implementation = 8 — but the plan is on this branch as the first commit so 1 plan + 7 implementation = 8). Diff stat should show net deletions in the thousand-line range.

- [ ] **Step 8.4: Open the PR**

```bash
git push -u origin feat/pvyml-breaking-pipeline-cleanup
gh pr create --title "feat: pv.yml is required — delete legacy pipeline (PR 5/6) BREAKING" --body "$(cat <<'PRBODY'
## Summary

PR 5 of 6 in the pv.yml redesign — the **breaking change**. See [spec](docs/superpowers/specs/2026-05-10-pv-yml-explicit-config-design.md) and [plan](docs/superpowers/plans/2026-05-11-pv-yml-pr5-breaking-pipeline-cleanup.md). After this PR `pv.yml` is required; the legacy pipeline that PRs 2/3 kept alive as a compat fallback is deleted; `pv link` without a pv.yml errors with `Run \`pv init\` to generate one.`

Deleted:
- **Auto-detect** — `steps.DetectServicesStep` and `laravel.DetectServicesStep` (the env writer). PR 2's `ApplyPvYmlServicesStep` and `ApplyPvYmlEnvStep` are now the sole binding + env path.
- **6 hardcoded laravel pipeline steps** — `CopyEnvStep`, `ComposerInstallStep`, `GenerateKeyStep`, `InstallOctaneStep`, `CreateDatabaseStep`, `RunMigrationsStep`. PR 3's `setup:` runner replaces them; users declare the commands they want in pv.yml.
- **Env writer steps** — `SetAppURLStep` and `SetViteTLSStep`. Users now declare `APP_URL: "{{ .site_url }}"` etc. in pv.yml's top-level `env:`.
- **Dead helpers** — `SmartEnvVars`, `UpdateProjectEnvForPostgres/Mysql/Redis`, `ComposerInstall`, `KeyGenerate`, `OctaneInstall`, `Migrate`, `ResolveDatabaseName`, and the file-existence checks tied exclusively to the deleted steps.
- **10 `Automation` struct fields** (`DetectServices`, `ServiceEnvUpdate`, `CopyEnv`, `ComposerInstall`, `GenerateKey`, `InstallOctane`, `CreateDatabase`, `RunMigrations`, `SetAppURL`, `SetViteTLS`) and their `LookupGate` cases. Old `~/.pv/settings.yml` files that carry these keys deserialize cleanly (Go ignores unknown fields).

Survived:
- `ApplyFallbacks` and `FallbackMapping` in `internal/laravel/env.go` — still called from `internal/svchooks/` on service removal.
- The 4 shared binding helpers (`findServiceByName`, `bindProjectService`, `bindProjectPostgres`, `bindProjectMysql`) — moved to `apply_pvyml_services.go` in the first commit before the auto-detect file was deleted.

7 commits, net ~1100-line deletion. README gains a "Migrating from pre-pv.yml versions" section pointing at `pv init`.

## Migration story (in the README under the new section)

```bash
cd /path/to/your/project
pv init               # detects type; writes pv.yml with sensible defaults
git add pv.yml && git commit -m "Add pv.yml"
pv link               # relinks with the new contract
```

## Test plan

- [ ] `go test ./...` — every package passes; new \`TestLink_RefusesWithoutPvYml\` pins the guard; tests that previously linked a tempdir now write fixture pv.yml files via the \`writePvYml(t, dir)\` helper
- [ ] `gofmt -l .` clean
- [ ] `go vet ./...` clean
- [ ] `go build ./...` clean
- [ ] Smoke test: \`pv link\` on a project without pv.yml errors with the \`pv init\` hint; \`pv init\` writes pv.yml; subsequent \`pv link\` passes the guard
- [ ] Old settings.yml files (with deprecated Automation fields like \`detect_services\`) still load cleanly — Go's yaml.Unmarshal ignores unknown fields

## What's left

After this lands, only PR 6 remains: managed-key tracking so removing a key from pv.yml removes it from \`.env\` on next link. That closes the dead-var gap and is the rollout's last polish.
PRBODY
)"
```

(The PR command is illustrative — adapt the body wording to match what just got built.)

---

## Self-Review (already applied)

**Spec coverage:**
- ✅ Auto-detect deleted — Tasks 2 + 3.
- ✅ 6 hardcoded steps deleted — Task 4.
- ✅ `SetAppURLStep` + `SetViteTLSStep` deleted — Task 5.
- ✅ `cmd/setup.go` checked — no changes needed (TUI wizard, not a pipeline runner).
- ✅ `pv link` requires pv.yml — Task 6.
- ✅ README migration guide — Task 7.

**Placeholders:** None. Every step has the file paths, the lines to look for, and the verification commands.

**Type consistency:**
- The 4 shared helpers (`findServiceByName`, `bindProjectService`, `bindProjectPostgres`, `bindProjectMysql`) — used identically before and after the move. Task 1 just relocates them.
- The `writePvYml(t, projDir)` test helper — single signature used across all updated tests.

**Deliberate notes for future readers:**
- **Old settings.yml files with deprecated fields**: yaml.Unmarshal silently ignores unknown fields, so users carrying `~/.pv/settings.yml` from a pre-PR-5 install boot up fine. No explicit migration error. README's "What's no longer automatic" section covers the change.
- **`ApplyFallbacks` / `FallbackMapping` survive** because they're called from `internal/svchooks/` when a service is removed from a project. That's a separate code path from the pipeline; it stays intact.
- **`internal/svchooks/`** — verify during implementation that nothing in svchooks calls a soon-to-be-deleted helper. If it does (likely doesn't, but possible), update svchooks accordingly.

## Open questions

1. **Should we delete the `internal/laravel/` package entirely** if nothing's left after Task 5? Unlikely — `isLaravel`, `ApplyFallbacks`, `FallbackMapping`, and possibly `RunArtisan` survive. But if the package becomes 30 lines of helpers, consider folding into `internal/projectenv/` or similar. Decide during implementation.
2. **Should old settings.yml fields error or warn?** Current plan: silently ignore (Go's yaml.Unmarshal default). Alternative: emit `ui.Subtle("settings.yml contains obsolete fields: %v — they're ignored")`. Defer; users who keep old settings files for years probably hit other migration issues first.
