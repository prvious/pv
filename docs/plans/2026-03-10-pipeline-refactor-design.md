# Link Pipeline Refactor — Design

**Date:** 2026-03-10
**Status:** Approved

## Problem

The `pv link` command mixes inline logic, conditional Laravel-only pipelines, and mid-function mutations. Operations like PHP install, Caddyfile generation, TLS certs, and service detection are hardcoded in `cmd/link.go` instead of flowing through the automation pipeline. The pipeline itself is Laravel-specific, making it impossible to reuse for future frameworks.

## Solution

Make **everything** a Step. The pipeline becomes the single organizing principle for all link-time operations. Steps declare their own applicability via `ShouldRun()`, whether they're critical, and their automation gate. The link command becomes a thin orchestrator.

## Step Interface

```go
type Step interface {
    Label() string
    Gate() string                     // every step has an automation gate
    Critical() bool                   // true = failure aborts pipeline
    ShouldRun(ctx *Context) bool      // step decides its own applicability
    Run(ctx *Context) (string, error)
}
```

All gates map to `AutoMode` values in settings. The setup wizard shows a curated subset. Hidden gates (e.g. `generate_caddyfile`) are configurable via hand-editing `pv.yml` but not exposed in the wizard UI.

## Context

Mutable state object that flows through the pipeline. Steps mutate it freely. The pipeline saves the registry once at the end.

```go
type Context struct {
    ProjectPath string
    ProjectName string
    ProjectType string
    PHPVersion  string
    GlobalPHP   string

    TLD      string
    Settings *config.Settings
    Registry *registry.Registry
    Env      map[string]string

    DBCreated bool
}
```

## Pipeline

```go
func RunPipeline(steps []Step, ctx *Context) error {
    for _, step := range steps {
        if !step.ShouldRun(ctx) { continue }

        mode := LookupGate(&ctx.Settings.Automation, step.Gate())
        // gate logic (AutoOff skip, AutoAsk prompt, AutoOn proceed)

        err := ui.Step(step.Label(), func() (string, error) {
            return step.Run(ctx)
        })
        if err != nil && step.Critical() {
            return err
        }
    }
    return ctx.Registry.Save()
}
```

## Step Order

| # | Step | Gate | Critical | ShouldRun | Location |
|---|------|------|----------|-----------|----------|
| 1 | Install PHP version | `install_php_version` | yes | version != global && !installed | `internal/automation/steps/` |
| 2 | Copy .env | `copy_env` | no | has .env.example && no .env | `internal/laravel/` |
| 3 | Composer install | `composer_install` | no | has composer.json && no vendor/ | `internal/laravel/` |
| 4 | Generate app key | `generate_key` | no | Laravel && APP_KEY empty | `internal/laravel/` |
| 5 | Install Octane | `install_octane` | no | Laravel && octane pkg && no worker | `internal/laravel/` |
| 6 | Generate site config | `generate_site_config` | yes | always | `internal/automation/steps/` |
| 7 | Generate Caddyfile | `generate_caddyfile` | yes | always | `internal/automation/steps/` |
| 8 | Generate TLS cert | `generate_tls_cert` | no | always | `internal/automation/steps/` |
| 9 | Detect & bind services | `detect_services` | no | always | `internal/automation/steps/` |
| 10 | Configure service env | `update_env_on_service` | no | Laravel && services bound | `internal/laravel/` |
| 11 | Set APP_URL | `set_app_url` | no | Laravel && has .env | `internal/laravel/` |
| 12 | Create database | `create_database` | no | Laravel && DB service bound | `internal/laravel/` |
| 13 | Run migrations | `run_migrations` | no | Laravel && DB created | `internal/laravel/` |

### Wizard-visible gates

`install_php_version`, `copy_env`, `composer_install`, `generate_key`, `set_app_url`, `install_octane`, `create_database`, `run_migrations`, `update_env_on_service`, `service_fallback`

### Hidden gates (not in wizard, hand-editable)

`generate_site_config`, `generate_caddyfile`, `generate_tls_cert`, `detect_services`

## Link Command

Becomes thin:

1. Resolve path, name, check duplicates
2. Detect project type
3. Resolve PHP version
4. Build context
5. Register project in registry
6. Build full step list, run single pipeline
7. Print success output
8. Reload/restart server if needed
9. Watch project

The `if projectType == "laravel"` blocks disappear. Each step's `ShouldRun` handles framework checks internally.

## Key Decisions

- **All steps have gates (option C)**: Uniform interface, hidden gates for infrastructure steps. Power users can override via `pv.yml`.
- **Mutable context, save once (option 2)**: Steps mutate context freely. Pipeline saves registry at the end. No per-step filesystem writes.
- **Critical steps**: `Critical() bool` on the interface. Pipeline aborts on critical step failure, continues on best-effort failure.
- **Octane re-detection**: Owned by `InstallOctaneStep.Run()` which mutates `ctx.ProjectType` after installing.

## Files to Change

| File | Change |
|------|--------|
| `internal/automation/pipeline.go` | Update `Step` interface (add `Critical`), update `RunPipeline`, add terminal save, add `Context.GlobalPHP` |
| `internal/automation/steps/` | New package: universal steps (PHP install, Caddyfile, TLS, services) |
| `internal/laravel/steps.go` | Add `Critical() bool` to existing steps, move Octane re-detection into step |
| `internal/config/settings.go` | Add hidden gate fields to `Automation` struct |
| `cmd/setup_tui.go` | No change to visible items (hidden gates stay hidden) |
| `cmd/link.go` | Gut inline logic, build single step list, run one pipeline |
| `cmd/link_test.go` | Update tests for new flow |
