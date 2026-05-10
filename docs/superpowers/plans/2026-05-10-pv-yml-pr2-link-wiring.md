# pv.yml PR 2 — `pv link` honors services, env, and aliases Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `pv link` consume the pv.yml schema landed in PR 1 — bind services declared in pv.yml (replacing the auto-detect heuristic when present), render `env:` blocks against template vars and merge into `.env`, and extend Caddy site config + TLS cert minting to cover `aliases:`. Auto-detect and Laravel env writers remain as a fallback when pv.yml declares nothing.

**Architecture:**
- Load `*ProjectConfig` once in `cmd/link.go`, plumb it onto `automation.Context`.
- Two new pipeline steps — `ApplyPvYmlServicesStep` (binds pv.yml-declared services into the registry) and `ApplyPvYmlEnvStep` (renders templates, merges into `.env`). Both are no-ops when pv.yml is absent or declares nothing in their domain.
- Modify the existing auto-detect (`DetectServicesStep`) and Laravel env writer (`laravel.DetectServicesStep`) to opt out via `ShouldRun` when their pv.yml counterpart will run.
- Caddy site templates and `GenerateTLSCertStep` learn to iterate aliases.

**Tech Stack:** Go 1.x, existing `internal/automation/` step framework, `gopkg.in/yaml.v3`, `text/template` (via `projectenv.Render`).

**Spec reference:** `docs/superpowers/specs/2026-05-10-pv-yml-explicit-config-design.md`.

**Base commit:** `7c18d25` on `main` (PR 1 merged). Branch: `feat/pvyml-link-wiring`.

---

## Compat strategy in one paragraph

pv.yml drives behavior **only in the domains it explicitly declares**, and the legacy paths fill in the rest. Specifically:

| pv.yml declares… | New behavior | Legacy behavior |
|---|---|---|
| Any service block (`postgresql:` / `mysql:` / `redis:` / `mailpit:` / `rustfs:`) | `ApplyPvYmlServicesStep` binds it | `DetectServicesStep` is **skipped** |
| Any `env:` (top-level OR per-service) | `ApplyPvYmlEnvStep` renders + merges | `laravel.DetectServicesStep` env-writer logic is **skipped** |
| `aliases:` | Caddy site block + TLS certs include them | (legacy didn't have this; additive) |
| Nothing in a given domain | (n/a) | Legacy step runs unchanged |

The `HasServices()` and `HasAnyEnv()` helper methods on `*ProjectConfig` are the decision points. Both are nil-safe so existing projects without pv.yml hit the legacy path naturally.

---

## File Structure

| File | Action | Responsibility |
|---|---|---|
| `internal/config/pvyml.go` | Modify | Add `HasServices()` and `HasAnyEnv()` methods on `*ProjectConfig`. |
| `internal/config/pvyml_test.go` | Modify | Tests for both helper methods across the nil / empty / declared matrix. |
| `internal/automation/pipeline.go` | Modify | Add `ProjectConfig *config.ProjectConfig` field to `Context`. |
| `cmd/link.go` | Modify | Call `config.FindAndLoadProjectConfig(projectPath)` and assign to `ctx.ProjectConfig`. |
| `internal/automation/steps/apply_pvyml_services.go` | Create | New step. Binds declared services into the registry, errors if a version isn't installed. |
| `internal/automation/steps/apply_pvyml_services_test.go` | Create | Tests for happy path + missing-version error + no-config-noop. |
| `internal/automation/steps/detect_services.go` | Modify | `ShouldRun` returns false when `ctx.ProjectConfig.HasServices()`. |
| `internal/automation/steps/detect_services_test.go` | Modify | Add a test confirming the skip behavior. |
| `internal/automation/steps/apply_pvyml_env.go` | Create | New step. Renders top-level + per-service env templates, merges into `.env`. |
| `internal/automation/steps/apply_pvyml_env_test.go` | Create | Tests for happy path (rendered values, MergeDotEnv produced the right keys), error propagation. |
| `internal/laravel/steps.go` | Modify | `DetectServicesStep.ShouldRun` returns false when `ctx.ProjectConfig.HasAnyEnv()`. |
| `internal/laravel/steps_test.go` *(or wherever existing tests live)* | Modify | Add a test confirming the skip behavior. |
| `internal/caddy/caddy.go` | Modify | Add `Aliases []string` to `siteData`; update all site templates to emit aliases as SANs. |
| `internal/caddy/caddy_test.go` *(or new file)* | Modify | Render-based tests confirming aliases appear in the generated Caddy block. |
| `internal/automation/steps/generate_tls_cert.go` | Modify | Mint a cert per alias in addition to the primary host. |
| `internal/automation/steps/generate_tls_cert_test.go` *(create if absent)* | Create/Modify | Test that minting iterates primary + aliases. |

Pipeline insertion order in `cmd/link.go` (`pv link`'s automation pipeline) becomes:

```
InstallPHPStep
CopyEnvStep
ComposerInstallStep
GenerateKeyStep
InstallOctaneStep
ApplyPvYmlServicesStep    ← new, before DetectServicesStep
DetectServicesStep         ← gated by ShouldRun
laravel.DetectServicesStep ← gated by ShouldRun
ApplyPvYmlEnvStep         ← new, after laravel writer; merges any pv.yml env on top
SetAppURLStep
SetViteTLSStep
GenerateTLSCertStep        ← extended for aliases
CreateDatabaseStep
RunMigrationsStep
```

(`SetAppURLStep` / `SetViteTLSStep` remain in PR 2 untouched. If pv.yml's top-level `env:` declares `APP_URL` or `VITE_DEV_SERVER_*`, `ApplyPvYmlEnvStep` writes them; `SetAppURLStep` / `SetViteTLSStep` re-write to their own values. That overlap is fine for PR 2 — PR 5 will delete the legacy steps; until then, the user-declared template values "win" because `ApplyPvYmlEnvStep` runs after both. Wait — that's actually wrong; `SetAppURLStep` runs after `ApplyPvYmlEnvStep` in the order above. Reorder so `ApplyPvYmlEnvStep` runs **last among env writers**, after `SetViteTLSStep`. That's the order in the table above.)

---

## Task 1: Plumb pv.yml into `automation.Context` + helpers

**Files:**
- Modify: `internal/config/pvyml.go`
- Modify: `internal/config/pvyml_test.go`
- Modify: `internal/automation/pipeline.go` (add field to `Context`)
- Modify: `cmd/link.go` (call `FindAndLoadProjectConfig`, populate field)

No behavior change yet — the new field is populated but no step reads it.

- [ ] **Step 1.1: Write failing test for `HasServices()`**

Append to `internal/config/pvyml_test.go`:

```go
func TestProjectConfig_HasServices(t *testing.T) {
	tests := []struct {
		name string
		cfg  *ProjectConfig
		want bool
	}{
		{"nil", nil, false},
		{"empty", &ProjectConfig{PHP: "8.4"}, false},
		{"postgres", &ProjectConfig{Postgresql: &ServiceConfig{Version: "18"}}, true},
		{"mysql", &ProjectConfig{Mysql: &ServiceConfig{Version: "8.0"}}, true},
		{"redis", &ProjectConfig{Redis: &ServiceConfig{}}, true},
		{"mailpit", &ProjectConfig{Mailpit: &ServiceConfig{}}, true},
		{"rustfs", &ProjectConfig{Rustfs: &ServiceConfig{}}, true},
		{"multiple", &ProjectConfig{Postgresql: &ServiceConfig{Version: "18"}, Redis: &ServiceConfig{}}, true},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := tt.cfg.HasServices(); got != tt.want {
				t.Errorf("HasServices() = %v, want %v", got, tt.want)
			}
		})
	}
}
```

- [ ] **Step 1.2: Run, verify FAIL**

Run: `go test ./internal/config/ -run TestProjectConfig_HasServices -v`
Expected: FAIL — `cfg.HasServices undefined`.

- [ ] **Step 1.3: Implement `HasServices()`**

Append to `internal/config/pvyml.go`:

```go
// HasServices reports whether any service block is declared in pv.yml.
// Nil-safe so it can be called on a freshly-loaded *ProjectConfig that
// may not exist for the project.
func (p *ProjectConfig) HasServices() bool {
	if p == nil {
		return false
	}
	return p.Postgresql != nil || p.Mysql != nil || p.Redis != nil ||
		p.Mailpit != nil || p.Rustfs != nil
}
```

- [ ] **Step 1.4: Verify**

Run: `go test ./internal/config/ -v`
Expected: PASS (all existing + new sub-tests).

- [ ] **Step 1.5: Failing test for `HasAnyEnv()`**

Append to `internal/config/pvyml_test.go`:

```go
func TestProjectConfig_HasAnyEnv(t *testing.T) {
	tests := []struct {
		name string
		cfg  *ProjectConfig
		want bool
	}{
		{"nil", nil, false},
		{"empty", &ProjectConfig{PHP: "8.4"}, false},
		{"top-level env", &ProjectConfig{Env: map[string]string{"APP_URL": "x"}}, true},
		{"top-level env empty map", &ProjectConfig{Env: map[string]string{}}, false},
		{"postgres env", &ProjectConfig{Postgresql: &ServiceConfig{Env: map[string]string{"DB_HOST": "x"}}}, true},
		{"postgres declared no env", &ProjectConfig{Postgresql: &ServiceConfig{Version: "18"}}, false},
		{"mysql env", &ProjectConfig{Mysql: &ServiceConfig{Env: map[string]string{"DB_HOST": "x"}}}, true},
		{"redis env", &ProjectConfig{Redis: &ServiceConfig{Env: map[string]string{"REDIS_HOST": "x"}}}, true},
		{"mailpit env", &ProjectConfig{Mailpit: &ServiceConfig{Env: map[string]string{"MAIL_HOST": "x"}}}, true},
		{"rustfs env", &ProjectConfig{Rustfs: &ServiceConfig{Env: map[string]string{"AWS_ENDPOINT": "x"}}}, true},
		{"top + service env", &ProjectConfig{Env: map[string]string{"APP_URL": "x"}, Postgresql: &ServiceConfig{Env: map[string]string{"DB_HOST": "x"}}}, true},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := tt.cfg.HasAnyEnv(); got != tt.want {
				t.Errorf("HasAnyEnv() = %v, want %v", got, tt.want)
			}
		})
	}
}
```

- [ ] **Step 1.6: Run, verify FAIL**

Run: `go test ./internal/config/ -run TestProjectConfig_HasAnyEnv -v`
Expected: FAIL — `cfg.HasAnyEnv undefined`.

- [ ] **Step 1.7: Implement `HasAnyEnv()`**

Append to `internal/config/pvyml.go`:

```go
// HasAnyEnv reports whether pv.yml declares any env keys — either the
// top-level Env map or any service's Env map. Used to decide whether
// the new pv.yml-driven env writer runs and the legacy Laravel
// writer skips.
func (p *ProjectConfig) HasAnyEnv() bool {
	if p == nil {
		return false
	}
	if len(p.Env) > 0 {
		return true
	}
	for _, svc := range []*ServiceConfig{p.Postgresql, p.Mysql, p.Redis, p.Mailpit, p.Rustfs} {
		if svc != nil && len(svc.Env) > 0 {
			return true
		}
	}
	return false
}
```

- [ ] **Step 1.8: Verify**

Run: `go test ./internal/config/ -v`
Expected: PASS.

- [ ] **Step 1.9: Failing test for `Context.ProjectConfig` plumbing**

Append to a new test file `internal/automation/context_test.go`:

```go
package automation

import (
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestContext_HoldsProjectConfig(t *testing.T) {
	cfg := &config.ProjectConfig{PHP: "8.4"}
	ctx := &Context{ProjectConfig: cfg}
	if ctx.ProjectConfig != cfg {
		t.Errorf("Context.ProjectConfig not preserved")
	}
}

func TestContext_NilProjectConfigOK(t *testing.T) {
	ctx := &Context{}
	if ctx.ProjectConfig.HasServices() {
		t.Errorf("nil ProjectConfig should report no services")
	}
	if ctx.ProjectConfig.HasAnyEnv() {
		t.Errorf("nil ProjectConfig should report no env")
	}
}
```

- [ ] **Step 1.10: Run, verify FAIL**

Run: `go test ./internal/automation/ -run TestContext -v`
Expected: FAIL — `unknown field ProjectConfig in struct literal of type automation.Context`.

- [ ] **Step 1.11: Add `ProjectConfig` field to `Context`**

In `internal/automation/pipeline.go`, find the `Context` struct (around lines 16–27) and add a field. The struct should look like:

```go
type Context struct {
	ProjectPath   string
	ProjectName   string
	ProjectType   string
	PHPVersion    string
	GlobalPHP     string
	TLD           string
	Registry      *registry.Registry
	Settings      *config.Settings
	Env           map[string]string
	DBCreated     bool
	ProjectConfig *config.ProjectConfig
}
```

(Keep all existing fields in their existing order; append `ProjectConfig` last.)

If `config` isn't already imported in `pipeline.go`, add the import (alphabetical within external group). Verify with:
```
grep -n '"github.com/prvious/pv/internal/config"' internal/automation/pipeline.go
```
If absent, add to the import block.

- [ ] **Step 1.12: Verify**

Run: `go test ./internal/automation/ -v`
Expected: PASS for both new tests AND every existing test in the package.

- [ ] **Step 1.13: Plumb `FindAndLoadProjectConfig` in `cmd/link.go`**

Read `cmd/link.go` around lines 109–119 where the `automation.Context` is built. **Before** the `ctx := &automation.Context{...}` literal, add:

```go
projectCfg, err := config.FindAndLoadProjectConfig(projectPath)
if err != nil {
	return fmt.Errorf("read pv.yml: %w", err)
}
```

Then in the `Context` literal, add the field:

```go
ProjectConfig: projectCfg,
```

(The other fields stay as they are.)

`config` is almost certainly already imported in `cmd/link.go` (it's used for `config.LoadSettings`). Verify:
```
grep -n '"github.com/prvious/pv/internal/config"' cmd/link.go
```
If absent, add it.

- [ ] **Step 1.14: Verify**

Run from repo root:
```
go vet ./...
go build ./...
go test ./...
```
Expected: all clean. No behavior change — the plumbed field isn't read by any step yet.

- [ ] **Step 1.15: gofmt**

```
gofmt -w internal/config/ internal/automation/ cmd/
```

- [ ] **Step 1.16: Commit**

```bash
git add internal/config/pvyml.go internal/config/pvyml_test.go \
        internal/automation/pipeline.go internal/automation/context_test.go \
        cmd/link.go
git commit -m "$(cat <<'EOF'
feat(automation): plumb pv.yml ProjectConfig onto Context

Adds HasServices() and HasAnyEnv() helper methods on *ProjectConfig
(both nil-safe), a ProjectConfig field on automation.Context, and a
FindAndLoadProjectConfig call early in pv link that populates it. No
step reads the field yet — that lands in subsequent tasks. The
helpers are the decision points for whether pv.yml-driven steps run
and whether legacy auto-detect / Laravel writers skip.
EOF
)"
```

---

## Task 2: `ApplyPvYmlServicesStep` + skip `DetectServicesStep` when pv.yml drives services

**Files:**
- Create: `internal/automation/steps/apply_pvyml_services.go`
- Create: `internal/automation/steps/apply_pvyml_services_test.go`
- Modify: `internal/automation/steps/detect_services.go` (`ShouldRun` skip)
- Modify: `internal/automation/steps/detect_services_test.go` (add skip test)
- Modify: `cmd/link.go` (insert new step before `DetectServicesStep`)

This task binds pv.yml-declared services into the registry using the same `bindProjectPostgres` / `bindProjectMysql` / `bindProjectService` helpers the auto-detect path already uses (visible in `internal/automation/steps/detect_services.go`).

For postgres and mysql, the step errors when the declared version isn't installed (`postgres.IsInstalled(major)` / `mysql.InstalledVersions()` membership check). For redis we use `redis.IsInstalled()`. For mailpit/rustfs the existing pipeline doesn't have a per-machine installed-check (the daemon handles that), so we bind unconditionally — matching how `DetectServicesStep` binds them today.

- [ ] **Step 2.1: Failing test — binds postgres from pv.yml**

Create `internal/automation/steps/apply_pvyml_services_test.go`. Look at the existing `detect_services_test.go` (in the same dir) for the helper pattern — it has a `stageMysqlBinary` function that creates a stub binary at the expected path so `mysql.InstalledVersions()` returns it. You'll write equivalents for postgres and redis.

```go
package steps

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
)

// stagePostgresBinary writes a stub postgres at ~/.pv/postgres/<major>/bin/postgres
// so postgres.IsInstalled(major) returns true.
func stagePostgresBinary(t *testing.T, major string) {
	t.Helper()
	bin := config.PostgresBinDir(major)
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatalf("mkdir %s: %v", bin, err)
	}
	if err := os.WriteFile(filepath.Join(bin, "postgres"), []byte{}, 0o755); err != nil {
		t.Fatalf("stage postgres: %v", err)
	}
}

// stageRedisBinary writes a stub redis-server at the path redis.IsInstalled() looks for.
func stageRedisBinary(t *testing.T) {
	t.Helper()
	bin := config.RedisBinDir()
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatalf("mkdir %s: %v", bin, err)
	}
	if err := os.WriteFile(filepath.Join(bin, "redis-server"), []byte{}, 0o755); err != nil {
		t.Fatalf("stage redis: %v", err)
	}
}

func TestApplyPvYmlServices_BindsPostgresFromConfig(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	stagePostgresBinary(t, "18")

	projDir := t.TempDir()
	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{},
		Projects: []registry.Project{{Name: "p", Path: projDir, Type: "laravel"}},
	}
	ctx := &automation.Context{
		ProjectName: "p",
		ProjectPath: projDir,
		ProjectType: "laravel",
		Registry:    reg,
		ProjectConfig: &config.ProjectConfig{
			Postgresql: &config.ServiceConfig{Version: "18"},
		},
	}

	step := &ApplyPvYmlServicesStep{}
	if !step.ShouldRun(ctx) {
		t.Fatal("ShouldRun: want true when pv.yml declares services")
	}
	if _, err := step.Run(ctx); err != nil {
		t.Fatalf("Run: %v", err)
	}
	if reg.Projects[0].Services == nil || reg.Projects[0].Services.Postgres != "18" {
		t.Errorf("Postgres binding = %+v, want major=18", reg.Projects[0].Services)
	}
}
```

(`config.PostgresBinDir` / `config.RedisBinDir` — verify these helpers exist. They should mirror `config.MysqlBinDir` used by `detect_services_test.go`. If `RedisBinDir` doesn't exist, look in `internal/redis/` for where `IsInstalled` looks and use whichever helper it does.)

- [ ] **Step 2.2: Run, verify FAIL**

Run: `go test ./internal/automation/steps/ -run TestApplyPvYmlServices_BindsPostgresFromConfig -v`
Expected: FAIL — `undefined: ApplyPvYmlServicesStep`.

- [ ] **Step 2.3: Implement `ApplyPvYmlServicesStep`**

Create `internal/automation/steps/apply_pvyml_services.go`:

```go
package steps

import (
	"fmt"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/mysql"
	"github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/redis"
)

// ApplyPvYmlServicesStep binds the services declared in a project's
// pv.yml into the registry. It runs before DetectServicesStep and,
// when active, causes DetectServicesStep to skip via its ShouldRun.
//
// For version-bearing services (postgres, mysql), errors if the
// declared version isn't installed. For single-version services
// (redis, mailpit, rustfs), binds unconditionally — matching the
// existing auto-detect behavior.
type ApplyPvYmlServicesStep struct{}

func (s *ApplyPvYmlServicesStep) Label() string    { return "Bind services from pv.yml" }
func (s *ApplyPvYmlServicesStep) Gate() string     { return "apply_pvyml_services" }
func (s *ApplyPvYmlServicesStep) Critical() bool   { return true }

func (s *ApplyPvYmlServicesStep) ShouldRun(ctx *automation.Context) bool {
	return ctx.ProjectConfig.HasServices()
}

func (s *ApplyPvYmlServicesStep) Run(ctx *automation.Context) (string, error) {
	cfg := ctx.ProjectConfig
	count := 0

	if cfg.Postgresql != nil {
		major := cfg.Postgresql.Version
		if major == "" {
			return "", fmt.Errorf("pv.yml postgresql: version is required")
		}
		if !postgres.IsInstalled(major) {
			return "", fmt.Errorf("pv.yml postgresql %q is not installed — run `pv postgres:install %s`", major, major)
		}
		if err := bindProjectPostgres(ctx.Registry, ctx.ProjectName, major); err != nil {
			return "", fmt.Errorf("bind postgres: %w", err)
		}
		count++
	}

	if cfg.Mysql != nil {
		version := cfg.Mysql.Version
		if version == "" {
			return "", fmt.Errorf("pv.yml mysql: version is required")
		}
		installed, err := mysql.InstalledVersions()
		if err != nil {
			return "", fmt.Errorf("list mysql versions: %w", err)
		}
		found := false
		for _, v := range installed {
			if v == version {
				found = true
				break
			}
		}
		if !found {
			return "", fmt.Errorf("pv.yml mysql %q is not installed — run `pv mysql:install %s`", version, version)
		}
		if err := bindProjectMysql(ctx.Registry, ctx.ProjectName, version); err != nil {
			return "", fmt.Errorf("bind mysql: %w", err)
		}
		count++
	}

	if cfg.Redis != nil {
		if !redis.IsInstalled() {
			return "", fmt.Errorf("pv.yml redis is not installed — run `pv redis:install`")
		}
		if err := bindProjectService(ctx.Registry, ctx.ProjectName, "redis", "redis"); err != nil {
			return "", fmt.Errorf("bind redis: %w", err)
		}
		count++
	}

	if cfg.Mailpit != nil {
		if err := bindProjectService(ctx.Registry, ctx.ProjectName, "mail", "mailpit"); err != nil {
			return "", fmt.Errorf("bind mailpit: %w", err)
		}
		count++
	}

	if cfg.Rustfs != nil {
		if err := bindProjectService(ctx.Registry, ctx.ProjectName, "s3", "rustfs"); err != nil {
			return "", fmt.Errorf("bind rustfs: %w", err)
		}
		count++
	}

	return fmt.Sprintf("bound %d service(s) from pv.yml", count), nil
}
```

Notes:
- `bindProjectPostgres`, `bindProjectMysql`, `bindProjectService` are package-level helpers already defined in `internal/automation/steps/detect_services.go` (lines 167–191, 147–165 per the exploration). They mutate `reg.Projects[i].Services.*` and don't return errors usually, but the signatures in the existing detect_services.go are what to match — if they don't return errors, drop the `if err := ...; err != nil` wrappers and call them directly.
- `findServiceByName` is the helper used for "mail" / "s3" lookups. The "redis"/"redis", "mail"/"mailpit", "s3"/"rustfs" key/name pairs are taken from existing `DetectServicesStep` usage.

If `bindProjectPostgres` / `bindProjectMysql` / `bindProjectService` are non-exported in `detect_services.go`, that's fine — `apply_pvyml_services.go` is in the same package and can call them directly.

- [ ] **Step 2.4: Verify the new test passes**

Run: `go test ./internal/automation/steps/ -run TestApplyPvYmlServices_BindsPostgresFromConfig -v`
Expected: PASS.

- [ ] **Step 2.5: Add more tests covering missing version, no-config-noop**

Append to `internal/automation/steps/apply_pvyml_services_test.go`:

```go
func TestApplyPvYmlServices_ErrorsWhenVersionNotInstalled(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	// Do NOT stage any postgres binary.

	projDir := t.TempDir()
	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{},
		Projects: []registry.Project{{Name: "p", Path: projDir, Type: "laravel"}},
	}
	ctx := &automation.Context{
		ProjectName: "p",
		ProjectPath: projDir,
		Registry:    reg,
		ProjectConfig: &config.ProjectConfig{
			Postgresql: &config.ServiceConfig{Version: "18"},
		},
	}
	step := &ApplyPvYmlServicesStep{}
	if _, err := step.Run(ctx); err == nil {
		t.Fatal("Run: want error when postgres not installed, got nil")
	}
}

func TestApplyPvYmlServices_ShouldRunFalseWithoutConfig(t *testing.T) {
	ctx := &automation.Context{}
	step := &ApplyPvYmlServicesStep{}
	if step.ShouldRun(ctx) {
		t.Errorf("ShouldRun: want false when ProjectConfig is nil")
	}
}

func TestApplyPvYmlServices_ShouldRunFalseWhenNoServicesDeclared(t *testing.T) {
	ctx := &automation.Context{
		ProjectConfig: &config.ProjectConfig{PHP: "8.4"},
	}
	step := &ApplyPvYmlServicesStep{}
	if step.ShouldRun(ctx) {
		t.Errorf("ShouldRun: want false when no services declared")
	}
}
```

- [ ] **Step 2.6: Verify**

Run: `go test ./internal/automation/steps/ -run TestApplyPvYmlServices -v`
Expected: all three PASS.

- [ ] **Step 2.7: Failing test — `DetectServicesStep` skips when pv.yml drives services**

Append to `internal/automation/steps/detect_services_test.go`:

```go
func TestDetectServices_SkipsWhenPvYmlDeclaresServices(t *testing.T) {
	ctx := &automation.Context{
		ProjectConfig: &config.ProjectConfig{
			Postgresql: &config.ServiceConfig{Version: "18"},
		},
	}
	step := &DetectServicesStep{}
	if step.ShouldRun(ctx) {
		t.Errorf("ShouldRun: want false when pv.yml declares services")
	}
}

func TestDetectServices_RunsWhenPvYmlEmpty(t *testing.T) {
	ctx := &automation.Context{
		ProjectConfig: &config.ProjectConfig{PHP: "8.4"},
	}
	step := &DetectServicesStep{}
	if !step.ShouldRun(ctx) {
		t.Errorf("ShouldRun: want true when pv.yml has no services")
	}
}

func TestDetectServices_RunsWhenNoPvYml(t *testing.T) {
	ctx := &automation.Context{}
	step := &DetectServicesStep{}
	if !step.ShouldRun(ctx) {
		t.Errorf("ShouldRun: want true when no pv.yml at all")
	}
}
```

- [ ] **Step 2.8: Run, verify FAIL**

Run: `go test ./internal/automation/steps/ -run TestDetectServices_SkipsWhenPvYmlDeclaresServices -v`
Expected: FAIL — `TestDetectServices_SkipsWhenPvYmlDeclaresServices` fails because the current `ShouldRun` always returns true.

- [ ] **Step 2.9: Update `DetectServicesStep.ShouldRun`**

Find `ShouldRun` in `internal/automation/steps/detect_services.go` (around line 29–31). Replace its body so it returns false when pv.yml declares services:

```go
func (s *DetectServicesStep) ShouldRun(ctx *automation.Context) bool {
	if ctx.ProjectConfig.HasServices() {
		return false
	}
	return true
}
```

- [ ] **Step 2.10: Verify**

Run: `go test ./internal/automation/steps/ -run TestDetectServices -v`
Expected: PASS for all DetectServices tests (existing + 3 new).

- [ ] **Step 2.11: Wire `ApplyPvYmlServicesStep` into the link pipeline**

In `cmd/link.go`, find where the step list is assembled (around lines 127–142 per exploration). Insert `&steps.ApplyPvYmlServicesStep{}` **immediately before** `&steps.DetectServicesStep{}`. The order should now read:

```go
allSteps := []automation.Step{
    &steps.InstallPHPStep{},
    &steps.CopyEnvStep{},
    &steps.ComposerInstallStep{},
    &steps.GenerateKeyStep{},
    &steps.InstallOctaneStep{},
    &steps.ApplyPvYmlServicesStep{},   // NEW
    &steps.DetectServicesStep{},
    &laravel.DetectServicesStep{},
    // ... rest unchanged
}
```

(Keep the surrounding entries as they appear in the current `cmd/link.go`; only insert the new line.)

- [ ] **Step 2.12: Verify the project still builds and all tests pass**

```
gofmt -w internal/automation/steps/ cmd/
go vet ./...
go build ./...
go test ./...
```
Expected: all clean.

- [ ] **Step 2.13: Commit**

```bash
git add internal/automation/steps/apply_pvyml_services.go \
        internal/automation/steps/apply_pvyml_services_test.go \
        internal/automation/steps/detect_services.go \
        internal/automation/steps/detect_services_test.go \
        cmd/link.go
git commit -m "$(cat <<'EOF'
feat(automation): bind pv.yml-declared services; skip auto-detect

ApplyPvYmlServicesStep binds postgres/mysql/redis/mailpit/rustfs
declared in pv.yml into the registry, errors when a declared
version isn't installed (with the corresponding install command in
the message). DetectServicesStep.ShouldRun now returns false when
pv.yml declares services, so the auto-detect path stays as a clean
fallback when pv.yml is absent or service-less.
EOF
)"
```

---

## Task 3: `ApplyPvYmlEnvStep` + skip `laravel.DetectServicesStep` env writer

**Files:**
- Create: `internal/automation/steps/apply_pvyml_env.go`
- Create: `internal/automation/steps/apply_pvyml_env_test.go`
- Modify: `internal/laravel/steps.go` (the `DetectServicesStep` `ShouldRun`)
- Modify: existing test file for `laravel.DetectServicesStep` (location to be confirmed during impl — likely `internal/laravel/steps_test.go`)
- Modify: `cmd/link.go` (insert new step into pipeline)

`ApplyPvYmlEnvStep` walks the top-level `env:` map plus each service's `env:` map, renders every template through `projectenv.Render` against the appropriate var producer, and merges every rendered key into the project's `.env` via `MergeDotEnv` with a `.pv-backup` backup.

For postgres and mysql, version-bearing template vars (`.version`, `.dsn`) need `ProbeVersion` results. Always probe — the binary is guaranteed installed by Task 2's check, the probe is fast, and the alternative (scanning templates for `.version` references) is more complexity for marginal speedup.

The legacy Laravel writer (`laravel.DetectServicesStep`) is gated to skip when `ctx.ProjectConfig.HasAnyEnv()`.

- [ ] **Step 3.1: Failing test — happy path top-level env**

Create `internal/automation/steps/apply_pvyml_env_test.go`:

```go
package steps

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/config"
)

func TestApplyPvYmlEnv_RendersTopLevelEnv(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	projDir := t.TempDir()
	if err := os.WriteFile(filepath.Join(projDir, ".env"), []byte("EXISTING=value\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	ctx := &automation.Context{
		ProjectName: "myapp",
		ProjectPath: projDir,
		TLD:         "test",
		ProjectConfig: &config.ProjectConfig{
			Env: map[string]string{
				"APP_URL":  "{{ .site_url }}",
				"APP_NAME": "MyApp",
			},
		},
	}
	step := &ApplyPvYmlEnvStep{}
	if !step.ShouldRun(ctx) {
		t.Fatal("ShouldRun: want true")
	}
	if _, err := step.Run(ctx); err != nil {
		t.Fatalf("Run: %v", err)
	}

	body, err := os.ReadFile(filepath.Join(projDir, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	s := string(body)
	if !strings.Contains(s, "APP_URL=https://myapp.test") {
		t.Errorf(".env missing APP_URL=https://myapp.test\n%s", s)
	}
	if !strings.Contains(s, "APP_NAME=MyApp") {
		t.Errorf(".env missing APP_NAME=MyApp\n%s", s)
	}
	if !strings.Contains(s, "EXISTING=value") {
		t.Errorf(".env clobbered existing key\n%s", s)
	}
}
```

- [ ] **Step 3.2: Run, verify FAIL**

Run: `go test ./internal/automation/steps/ -run TestApplyPvYmlEnv_RendersTopLevelEnv -v`
Expected: FAIL — `undefined: ApplyPvYmlEnvStep`.

- [ ] **Step 3.3: Implement `ApplyPvYmlEnvStep`**

Create `internal/automation/steps/apply_pvyml_env.go`:

```go
package steps

import (
	"fmt"
	"path/filepath"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/mailpit"
	"github.com/prvious/pv/internal/mysql"
	"github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/projectenv"
	"github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/rustfs"
)

// ApplyPvYmlEnvStep renders pv.yml's top-level env: and per-service
// env: templates against their respective variable maps and merges
// the rendered keys into the project's .env via MergeDotEnv.
//
// Runs when ctx.ProjectConfig.HasAnyEnv(). For version-bearing
// services (postgres/mysql), probes the installed binary to populate
// the .version / .dsn template vars; the previous step
// (ApplyPvYmlServicesStep) guarantees the binary is installed.
type ApplyPvYmlEnvStep struct{}

func (s *ApplyPvYmlEnvStep) Label() string    { return "Apply pv.yml env templates" }
func (s *ApplyPvYmlEnvStep) Gate() string     { return "apply_pvyml_env" }
func (s *ApplyPvYmlEnvStep) Critical() bool   { return true }

func (s *ApplyPvYmlEnvStep) ShouldRun(ctx *automation.Context) bool {
	return ctx.ProjectConfig.HasAnyEnv()
}

func (s *ApplyPvYmlEnvStep) Run(ctx *automation.Context) (string, error) {
	cfg := ctx.ProjectConfig
	rendered := map[string]string{}

	// Top-level env: project-level vars.
	if len(cfg.Env) > 0 {
		vars := projectenv.ProjectTemplateVars(ctx.ProjectName, ctx.TLD)
		if err := renderInto(rendered, cfg.Env, vars, "env"); err != nil {
			return "", err
		}
	}

	// postgresql.env
	if cfg.Postgresql != nil && len(cfg.Postgresql.Env) > 0 {
		full, err := postgres.ProbeVersion(cfg.Postgresql.Version)
		if err != nil {
			return "", fmt.Errorf("probe postgres version: %w", err)
		}
		vars, err := postgres.TemplateVars(cfg.Postgresql.Version, full)
		if err != nil {
			return "", fmt.Errorf("postgres template vars: %w", err)
		}
		if err := renderInto(rendered, cfg.Postgresql.Env, vars, "postgresql.env"); err != nil {
			return "", err
		}
	}

	// mysql.env
	if cfg.Mysql != nil && len(cfg.Mysql.Env) > 0 {
		full, err := mysql.ProbeVersion(cfg.Mysql.Version)
		if err != nil {
			return "", fmt.Errorf("probe mysql version: %w", err)
		}
		vars, err := mysql.TemplateVars(cfg.Mysql.Version, full)
		if err != nil {
			return "", fmt.Errorf("mysql template vars: %w", err)
		}
		if err := renderInto(rendered, cfg.Mysql.Env, vars, "mysql.env"); err != nil {
			return "", err
		}
	}

	// redis.env
	if cfg.Redis != nil && len(cfg.Redis.Env) > 0 {
		if err := renderInto(rendered, cfg.Redis.Env, redis.TemplateVars(), "redis.env"); err != nil {
			return "", err
		}
	}

	// mailpit.env
	if cfg.Mailpit != nil && len(cfg.Mailpit.Env) > 0 {
		if err := renderInto(rendered, cfg.Mailpit.Env, mailpit.TemplateVars(), "mailpit.env"); err != nil {
			return "", err
		}
	}

	// rustfs.env
	if cfg.Rustfs != nil && len(cfg.Rustfs.Env) > 0 {
		if err := renderInto(rendered, cfg.Rustfs.Env, rustfs.TemplateVars(), "rustfs.env"); err != nil {
			return "", err
		}
	}

	if len(rendered) == 0 {
		return "no env keys to write", nil
	}

	envPath := filepath.Join(ctx.ProjectPath, ".env")
	backupPath := filepath.Join(ctx.ProjectPath, ".pv-backup")
	if err := projectenv.MergeDotEnv(envPath, backupPath, rendered); err != nil {
		return "", fmt.Errorf("merge .env: %w", err)
	}
	return fmt.Sprintf("wrote %d key(s) to .env", len(rendered)), nil
}

// renderInto renders each template in src against vars and accumulates
// the result into dst. scope is used only for error messages.
func renderInto(dst, src, vars map[string]string, scope string) error {
	for key, tmpl := range src {
		out, err := projectenv.Render(tmpl, vars)
		if err != nil {
			return fmt.Errorf("%s[%s]: %w", scope, key, err)
		}
		dst[key] = out
	}
	return nil
}
```

- [ ] **Step 3.4: Verify the new test passes**

Run: `go test ./internal/automation/steps/ -run TestApplyPvYmlEnv_RendersTopLevelEnv -v`
Expected: PASS.

- [ ] **Step 3.5: Add tests for per-service env (redis — no probe needed), ShouldRun edge cases**

Append to `internal/automation/steps/apply_pvyml_env_test.go`:

```go
func TestApplyPvYmlEnv_RendersRedisEnv(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	projDir := t.TempDir()
	// No pre-existing .env — MergeDotEnv should create it.

	ctx := &automation.Context{
		ProjectName: "myapp",
		ProjectPath: projDir,
		TLD:         "test",
		ProjectConfig: &config.ProjectConfig{
			Redis: &config.ServiceConfig{
				Env: map[string]string{
					"REDIS_HOST": "{{ .host }}",
					"REDIS_PORT": "{{ .port }}",
					"REDIS_URL":  "{{ .url }}",
				},
			},
		},
	}
	step := &ApplyPvYmlEnvStep{}
	if _, err := step.Run(ctx); err != nil {
		t.Fatalf("Run: %v", err)
	}

	body, err := os.ReadFile(filepath.Join(projDir, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	s := string(body)
	for _, want := range []string{
		"REDIS_HOST=127.0.0.1",
		"REDIS_PORT=6379",
		"REDIS_URL=redis://127.0.0.1:6379",
	} {
		if !strings.Contains(s, want) {
			t.Errorf(".env missing %q\n%s", want, s)
		}
	}
}

func TestApplyPvYmlEnv_ShouldRunFalseWithoutEnv(t *testing.T) {
	ctx := &automation.Context{
		ProjectConfig: &config.ProjectConfig{PHP: "8.4"},
	}
	step := &ApplyPvYmlEnvStep{}
	if step.ShouldRun(ctx) {
		t.Errorf("ShouldRun: want false when no env declared")
	}
}

func TestApplyPvYmlEnv_ShouldRunFalseWithoutConfig(t *testing.T) {
	ctx := &automation.Context{}
	step := &ApplyPvYmlEnvStep{}
	if step.ShouldRun(ctx) {
		t.Errorf("ShouldRun: want false when ProjectConfig is nil")
	}
}

func TestApplyPvYmlEnv_ErrorsOnUnknownTemplateVar(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	projDir := t.TempDir()

	ctx := &automation.Context{
		ProjectName: "myapp",
		ProjectPath: projDir,
		TLD:         "test",
		ProjectConfig: &config.ProjectConfig{
			Env: map[string]string{
				"BAD": "{{ .nonexistent }}",
			},
		},
	}
	step := &ApplyPvYmlEnvStep{}
	if _, err := step.Run(ctx); err == nil {
		t.Fatal("Run: want error on unknown template var, got nil")
	}
}
```

- [ ] **Step 3.6: Verify all `ApplyPvYmlEnv` tests pass**

Run: `go test ./internal/automation/steps/ -run TestApplyPvYmlEnv -v`
Expected: all PASS.

- [ ] **Step 3.7: Failing test — `laravel.DetectServicesStep` skips when pv.yml drives env**

The Laravel auto-detect / env-writer step lives at `internal/laravel/steps.go` (`DetectServicesStep`). Locate its test file. If `internal/laravel/steps_test.go` exists, append; otherwise create it.

```go
package laravel

import (
	"testing"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/config"
)

func TestLaravelDetectServices_SkipsWhenPvYmlHasEnv(t *testing.T) {
	ctx := &automation.Context{
		ProjectConfig: &config.ProjectConfig{
			Env: map[string]string{"APP_URL": "{{ .site_url }}"},
		},
	}
	step := &DetectServicesStep{}
	if step.ShouldRun(ctx) {
		t.Errorf("ShouldRun: want false when pv.yml has env")
	}
}

func TestLaravelDetectServices_RunsWhenPvYmlEmpty(t *testing.T) {
	ctx := &automation.Context{
		ProjectConfig: &config.ProjectConfig{PHP: "8.4"},
	}
	step := &DetectServicesStep{}
	if !step.ShouldRun(ctx) {
		t.Errorf("ShouldRun: want true when pv.yml has no env")
	}
}
```

- [ ] **Step 3.8: Run, verify FAIL**

Run: `go test ./internal/laravel/ -run TestLaravelDetectServices_SkipsWhenPvYmlHasEnv -v`
Expected: FAIL — the current `ShouldRun` returns true unconditionally.

- [ ] **Step 3.9: Update `laravel.DetectServicesStep.ShouldRun`**

Open `internal/laravel/steps.go` and locate `DetectServicesStep.ShouldRun`. Replace its body so it returns false when pv.yml declares env:

```go
func (s *DetectServicesStep) ShouldRun(ctx *automation.Context) bool {
	if ctx.ProjectConfig.HasAnyEnv() {
		return false
	}
	return true
}
```

(If `ShouldRun` had additional conditions before, preserve them and add the pv.yml short-circuit as the first check.)

- [ ] **Step 3.10: Verify**

Run: `go test ./internal/laravel/ -v`
Expected: PASS — both new tests + every existing test green.

- [ ] **Step 3.11: Wire `ApplyPvYmlEnvStep` into the link pipeline**

In `cmd/link.go`, insert `&steps.ApplyPvYmlEnvStep{}` into the step list. Position: **after** `&laravel.DetectServicesStep{}`, **before** `&laravel.SetAppURLStep{}`. The order becomes:

```go
allSteps := []automation.Step{
    &steps.InstallPHPStep{},
    &steps.CopyEnvStep{},
    &steps.ComposerInstallStep{},
    &steps.GenerateKeyStep{},
    &steps.InstallOctaneStep{},
    &steps.ApplyPvYmlServicesStep{},
    &steps.DetectServicesStep{},
    &laravel.DetectServicesStep{},
    &steps.ApplyPvYmlEnvStep{},        // NEW
    &laravel.SetAppURLStep{},
    &laravel.SetViteTLSStep{},
    // ... rest unchanged
}
```

- [ ] **Step 3.12: Verify**

```
gofmt -w internal/automation/steps/ internal/laravel/ cmd/
go vet ./...
go build ./...
go test ./...
```
Expected: all clean, all tests pass.

- [ ] **Step 3.13: Commit**

```bash
git add internal/automation/steps/apply_pvyml_env.go \
        internal/automation/steps/apply_pvyml_env_test.go \
        internal/laravel/steps.go \
        internal/laravel/steps_test.go \
        cmd/link.go
git commit -m "$(cat <<'EOF'
feat(automation): apply pv.yml env templates; skip Laravel writer

ApplyPvYmlEnvStep renders top-level and per-service env: templates
against the matching template var producers (project-level,
postgres, mysql, redis, mailpit, rustfs) and merges all rendered
keys into the project's .env via MergeDotEnv with a .pv-backup.
laravel.DetectServicesStep.ShouldRun returns false when pv.yml
declares any env (top-level or per-service), so the legacy
Laravel-shaped env-writer skips. Auto-detect projects without
pv.yml env declarations continue on the legacy path.
EOF
)"
```

---

## Task 4: Caddy site aliases (multi-SAN site blocks)

**Files:**
- Modify: `internal/caddy/caddy.go` (add `Aliases []string` to `siteData`, update templates, plumb from `*registry.Project` or `*ProjectConfig` to data)
- Modify or Create: `internal/caddy/caddy_test.go` (verify rendered output)

The exploration confirmed that every Caddy site template hardcodes `{{.Name}}.{{.TLD}}, *.{{.Name}}.{{.TLD}}`. We add a third comma-separated entry that joins aliases. When `Aliases` is empty the new entry collapses to nothing — backwards compatible for projects without `aliases:`.

How the templates pick up aliases: extend `siteData` to carry `Aliases []string`, and add a small template helper or use Go template's range to render them inline.

- [ ] **Step 4.1: Read the current Caddy templates**

Open `internal/caddy/caddy.go`. Find the templates (lines ~26–116 per exploration). Confirm the host line shape (e.g., `{{.Name}}.{{.TLD}}, *.{{.Name}}.{{.TLD}} {`). Find the `siteData` struct (~line 156–162). Find `GenerateSiteConfig` (~line 186–225) — note how it builds the data struct and which fields come from the `registry.Project`.

This is reconnaissance only — no edits yet.

- [ ] **Step 4.2: Failing test — rendered site block includes aliases**

In `internal/caddy/caddy_test.go` (create if absent), append:

```go
package caddy

import (
	"strings"
	"testing"

	"github.com/prvious/pv/internal/registry"
)

func TestGenerateSiteConfig_IncludesAliases(t *testing.T) {
	proj := &registry.Project{
		Name:    "myapp",
		Path:    "/tmp/myapp",
		Type:    "php",
		PHP:     "8.4",
		Aliases: []string{"admin.myapp.test", "api.myapp.test"},
	}
	cfg, err := GenerateSiteConfig(proj, "8.4", "test")
	if err != nil {
		t.Fatalf("GenerateSiteConfig: %v", err)
	}
	for _, want := range []string{"myapp.test", "*.myapp.test", "admin.myapp.test", "api.myapp.test"} {
		if !strings.Contains(cfg, want) {
			t.Errorf("config missing host %q\n%s", want, cfg)
		}
	}
}

func TestGenerateSiteConfig_NoAliasesPreservesLegacy(t *testing.T) {
	proj := &registry.Project{
		Name: "myapp",
		Path: "/tmp/myapp",
		Type: "php",
		PHP:  "8.4",
	}
	cfg, err := GenerateSiteConfig(proj, "8.4", "test")
	if err != nil {
		t.Fatalf("GenerateSiteConfig: %v", err)
	}
	if !strings.Contains(cfg, "myapp.test") || !strings.Contains(cfg, "*.myapp.test") {
		t.Errorf("legacy hosts missing\n%s", cfg)
	}
}
```

(`GenerateSiteConfig`'s real signature might take additional args — adjust the call to match. Also: `registry.Project.Aliases` doesn't exist yet; this test exists to drive the field add.)

- [ ] **Step 4.3: Run, verify FAIL**

Run: `go test ./internal/caddy/ -run TestGenerateSiteConfig -v`
Expected: FAIL — either `Aliases` undefined on `registry.Project`, or the rendered output doesn't include the alias hostnames.

- [ ] **Step 4.4: Add `Aliases []string` to `registry.Project`**

Open `internal/registry/registry.go`. Find the `Project` struct. Add:

```go
Aliases []string `json:"aliases,omitempty"`
```

(Place it next to other per-project metadata fields. The JSON tag uses `omitempty` so existing registry files without aliases don't change shape on read.)

- [ ] **Step 4.5: Add `Aliases []string` to `siteData`**

In `internal/caddy/caddy.go`, find `siteData` (around line 156–162). Extend it:

```go
type siteData struct {
	Name     string
	Path     string
	RootPath string
	TLD      string
	Port     int
	Aliases  []string
}
```

- [ ] **Step 4.6: Update site templates to render aliases**

For each Caddy site template in `internal/caddy/caddy.go` (laravelOctaneTmpl, laravelTmpl, phpTmpl, staticTmpl, versionLaravelOctaneTmpl, versionLaravelTmpl, versionPhpTmpl), replace the host line. Current shape:

```
{{.Name}}.{{.TLD}}, *.{{.Name}}.{{.TLD}} {
```

Replace with:

```
{{.Name}}.{{.TLD}}, *.{{.Name}}.{{.TLD}}{{range .Aliases}}, {{.}}{{end}} {
```

This appends `, <alias>` for each alias. When `Aliases` is empty (nil or zero-length), the `{{range}}` body is skipped and the line falls back to the original two-host shape.

- [ ] **Step 4.7: Plumb aliases into `GenerateSiteConfig`**

Find `GenerateSiteConfig`. Where it builds the `siteData` literal, add:

```go
data := siteData{
    Name:    proj.Name,
    Path:    proj.Path,
    // ... existing fields ...
    Aliases: proj.Aliases,
}
```

(Exact field set depends on what's there today — preserve existing fields, just append `Aliases: proj.Aliases`.)

- [ ] **Step 4.8: Plumb aliases from pv.yml into `registry.Project` at link time**

In `cmd/link.go`, find where the project is registered or updated (around lines 93–106 per exploration). The project's `Aliases` should be set from `projectCfg.Aliases` (the field we plumbed onto Context in Task 1). Around where the project's existing fields are populated, add:

```go
if projectCfg != nil {
    proj.Aliases = projectCfg.Aliases
}
```

(Or wire through `reg.Add` / `reg.UpdateWith` if those wrap project mutation — match the existing pattern.)

- [ ] **Step 4.9: Verify**

Run: `go test ./internal/caddy/ -v`
Expected: PASS for both new tests + every existing Caddy test.

```
go test ./...
go vet ./...
go build ./...
gofmt -w internal/caddy/ internal/registry/ cmd/
```
Expected: all clean.

- [ ] **Step 4.10: Commit**

```bash
git add internal/caddy/caddy.go internal/caddy/caddy_test.go \
        internal/registry/registry.go \
        cmd/link.go
git commit -m "$(cat <<'EOF'
feat(caddy): render pv.yml aliases as additional SANs on the site block

Adds Aliases []string to registry.Project and siteData; updates every
Caddy site template (laravel / laravel-octane / php / static and
their version-aware twins) to append each alias to the host line.
Empty aliases preserve the legacy two-host shape verbatim, so
existing linked projects are unaffected. pv link reads
ctx.ProjectConfig.Aliases and stores them on the project entry.
EOF
)"
```

---

## Task 5: TLS certs per alias

**Files:**
- Modify: `internal/automation/steps/generate_tls_cert.go`
- Create or Modify: `internal/automation/steps/generate_tls_cert_test.go`

Today `GenerateTLSCertStep.Run` mints exactly one cert for `{name}.{tld}`. To serve aliases over HTTPS, we mint a cert per alias too. `certs.GenerateSiteTLS(hostname)` is the per-host minter (single source — `internal/certs/storage.go`).

- [ ] **Step 5.1: Failing test — minting iterates primary + aliases**

Create `internal/automation/steps/generate_tls_cert_test.go`:

```go
package steps

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/certs"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
)

// stageCaddyCA stages a fake Caddy CA so certs.GenerateSiteTLS finds
// the cert/key it needs to sign. Mirrors what `pv start` does for
// real but with throwaway content.
func stageCaddyCA(t *testing.T) {
	t.Helper()
	caCertPath := config.CACertPath()
	caKeyPath := config.CAKeyPath()
	for _, p := range []string{caCertPath, caKeyPath} {
		if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
			t.Fatalf("mkdir %s: %v", filepath.Dir(p), err)
		}
		if err := os.WriteFile(p, []byte("placeholder"), 0o644); err != nil {
			t.Fatalf("write %s: %v", p, err)
		}
	}
}

func TestGenerateTLSCert_MintsForAliases(t *testing.T) {
	t.Skip("Requires real CA signing; covered by e2e — leaving as documentation of intent")
	// If certs.GenerateSiteCert can be mocked or extracted, replace
	// this with a fake-minter test that asserts hostnames passed.
	_ = stageCaddyCA
	_ = certs.CertsDir
	_ = &registry.Project{}
	_ = &automation.Context{}
}
```

Reality: `certs.GenerateSiteCert` actually invokes Caddy's CA to sign — there's no clean mock seam in PR 1's exploration. Rather than over-engineer test infrastructure for PR 2, we:

1. Cover the iteration logic by unit-testing a helper function (extracted below).
2. Leave end-to-end cert minting verification to manual smoke test / future e2e.

Replace the stub above with a real test of the iteration helper. Continue:

```go
func TestExpandHostsForCertMinting(t *testing.T) {
	tests := []struct {
		name    string
		project string
		tld     string
		aliases []string
		want    []string
	}{
		{"no aliases", "myapp", "test", nil, []string{"myapp.test"}},
		{"one alias", "myapp", "test", []string{"admin.myapp.test"}, []string{"myapp.test", "admin.myapp.test"}},
		{"two aliases", "myapp", "test", []string{"a.test", "b.test"}, []string{"myapp.test", "a.test", "b.test"}},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := expandHostsForCertMinting(tt.project, tt.tld, tt.aliases)
			if len(got) != len(tt.want) {
				t.Fatalf("got %v, want %v", got, tt.want)
			}
			for i, h := range tt.want {
				if got[i] != h {
					t.Errorf("[%d] = %q, want %q", i, got[i], h)
				}
			}
		})
	}
}
```

- [ ] **Step 5.2: Run, verify FAIL**

Run: `go test ./internal/automation/steps/ -run TestExpandHostsForCertMinting -v`
Expected: FAIL — `undefined: expandHostsForCertMinting`.

- [ ] **Step 5.3: Implement the iteration + helper**

Open `internal/automation/steps/generate_tls_cert.go`. Replace its `Run` body so the primary host plus every alias is minted, factoring out a small helper:

```go
package steps

import (
	"fmt"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/certs"
)

type GenerateTLSCertStep struct{}

func (s *GenerateTLSCertStep) Label() string  { return "Generate TLS certificate(s)" }
func (s *GenerateTLSCertStep) Gate() string   { return "generate_tls_cert" }
func (s *GenerateTLSCertStep) Critical() bool { return true }

func (s *GenerateTLSCertStep) ShouldRun(ctx *automation.Context) bool {
	return true
}

func (s *GenerateTLSCertStep) Run(ctx *automation.Context) (string, error) {
	var aliases []string
	if ctx.ProjectConfig != nil {
		aliases = ctx.ProjectConfig.Aliases
	}
	hosts := expandHostsForCertMinting(ctx.ProjectName, ctx.TLD, aliases)
	for _, h := range hosts {
		if err := certs.GenerateSiteTLS(h); err != nil {
			return "", fmt.Errorf("generate cert for %s: %w", h, err)
		}
	}
	if len(hosts) == 1 {
		return fmt.Sprintf("minted 1 cert for %s", hosts[0]), nil
	}
	return fmt.Sprintf("minted %d certs (primary + %d alias)", len(hosts), len(hosts)-1), nil
}

// expandHostsForCertMinting returns the primary host followed by every
// alias, in order. Aliases are taken verbatim — pv.yml authors write
// fully-qualified hostnames (e.g., "admin.myapp.test").
func expandHostsForCertMinting(project, tld string, aliases []string) []string {
	hosts := make([]string, 0, 1+len(aliases))
	hosts = append(hosts, fmt.Sprintf("%s.%s", project, tld))
	hosts = append(hosts, aliases...)
	return hosts
}
```

(If the existing file already exports other functions/fields beyond what's shown, preserve them. The above is a full rewrite of the step's `Run` and adds the helper; existing imports may need pruning if `fmt.Sprintf` was the only use.)

- [ ] **Step 5.4: Verify the iteration test passes**

Run: `go test ./internal/automation/steps/ -run TestExpandHostsForCertMinting -v`
Expected: PASS.

- [ ] **Step 5.5: Verify everything still passes**

```
gofmt -w internal/automation/steps/
go vet ./...
go build ./...
go test ./...
```
Expected: all clean.

- [ ] **Step 5.6: Commit**

```bash
git add internal/automation/steps/generate_tls_cert.go \
        internal/automation/steps/generate_tls_cert_test.go
git commit -m "$(cat <<'EOF'
feat(certs): mint TLS cert per pv.yml alias in addition to primary

GenerateTLSCertStep now expands ctx.ProjectName.TLD + every alias
in ctx.ProjectConfig.Aliases and calls certs.GenerateSiteTLS for
each. The iteration is factored into expandHostsForCertMinting,
which is unit-tested directly (end-to-end Caddy CA signing is left
to manual / e2e verification). Projects without aliases keep the
single-cert behavior verbatim.
EOF
)"
```

---

## Task 6: Final verification sweep

**Files:** none modified.

- [ ] **Step 6.1: gofmt project-wide**

Run: `gofmt -l .`
Expected: (empty).

- [ ] **Step 6.2: go vet project-wide**

Run: `go vet ./...`
Expected: (empty).

- [ ] **Step 6.3: Full test suite**

Run: `go test ./...`
Expected: every package PASS.

- [ ] **Step 6.4: Full build**

Run: `go build ./...`
Expected: (empty).

- [ ] **Step 6.5: Smoke test the binary against a synthetic pv.yml**

```bash
go build -o /tmp/pv-pr2-smoke .
/tmp/pv-pr2-smoke --version
/tmp/pv-pr2-smoke link --help
rm /tmp/pv-pr2-smoke
```
Expected: version and help both work without error.

- [ ] **Step 6.6: Verify commit count and working tree**

Run: `git status && git log --oneline main..HEAD`
Expected: working tree clean. 5 commits on the branch (Tasks 1–5). No Task 6 commit.

---

## Self-Review (already applied)

**Spec coverage:**
- ✅ Service binding from pv.yml replaces auto-detect when pv.yml declares services — Task 2.
- ✅ Auto-detect remains as fallback when no service blocks declared — Task 2 (`DetectServicesStep.ShouldRun` short-circuits only on `HasServices()`).
- ✅ `env:` writes (top-level + per-service) flow through `MergeDotEnv` — Task 3.
- ✅ Only declared keys are written — Task 3 (`rendered` map only contains keys explicitly named in pv.yml).
- ✅ Aliases get Caddy SANs — Task 4.
- ✅ Aliases get TLS certs — Task 5.
- ✅ Service installed-check errors on missing version — Task 2.

**Placeholders:** None. Every step has concrete code or an explicit "look at existing pattern" pointer to a specific file with line numbers.

**Type consistency:**
- `HasServices()` / `HasAnyEnv()` methods used identically across Tasks 1, 2, 3.
- `ApplyPvYmlServicesStep` and `ApplyPvYmlEnvStep` are distinct types — no name collision.
- `expandHostsForCertMinting` signature stable: `(project, tld string, aliases []string) []string`.
- `siteData.Aliases` field name consistent with `ProjectConfig.Aliases` and `registry.Project.Aliases`.

**Deliberate notes for future readers:**
- `SetAppURLStep` / `SetViteTLSStep` are *not* gated by pv.yml in this PR. If a user declares `env: { APP_URL: "..." }` in pv.yml, `ApplyPvYmlEnvStep` writes the user's value first; then `SetAppURLStep` re-writes `APP_URL` to the pv-derived URL. The legacy step "wins" in this overlap, which means **users who explicitly want a custom `APP_URL` via pv.yml will see it overwritten until PR 5 deletes the legacy steps**. This is acceptable for PR 2 — the spec rollout calls for the legacy steps to die in PR 5. If users hit this in practice and complain before PR 5 lands, we can land a stop-gap that gates `SetAppURLStep` / `SetViteTLSStep` on pv.yml's top-level env containing the corresponding key.
- The smoke test in Task 6 doesn't exercise full `pv link` (which needs a real CA, services, etc.). End-to-end behavior is left to manual verification on the PR author's machine and the e2e workflow in CI.
