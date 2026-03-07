# CLAUDE.md

Instructions for working in this codebase. See `README.md` for architecture overview.

## What is pv

`pv` is a local dev server manager powered by FrankenPHP. Go + cobra CLI. Manages PHP versions, serves projects under `.test` domains with HTTPS, runs containerized backing services via Colima/Docker.

## Build & test

```bash
go build -o pv .              # build
go test ./...                  # all tests
go test ./internal/registry/   # one package
go test ./cmd/ -run TestLink   # pattern match
```

Build version is set via `go build -ldflags "-X github.com/prvious/pv/cmd.version=1.0.0"` — defaults to `"dev"`.

## Command conventions

- **Colon-namespaced**: tool/service/daemon commands use `tool:action` format (e.g., `mago:install`, `service:add`, `daemon:enable`). Core commands (`link`, `start`, `stop`) are plain.
- **All commands register on `rootCmd`** — cobra requires a flat `cmd/` directory. No subdirectories.
- **Always use `RunE`** (not `Run`) so errors propagate.
- **Command files are named `<tool>_<action>.go`** (e.g., `mago_install.go`, `service_add.go`).

## Tool command rules

Every managed tool (php, mago, composer, colima) follows a strict five-command pattern. When adding a new tool, create all five:

| Command | What it does | Where logic lives |
|---------|-------------|-------------------|
| `:download` | Fetches binary to private storage | `internal/binaries/` or `internal/phpenv/` |
| `:path` | Exposes/unexposes from PATH (supports `--remove`) | `internal/tools/` |
| `:install` | Orchestrates `:download` then `tools.Expose()` | `cmd/` — delegates only |
| `:update` | Redownloads, re-exposes if `tools.IsExposed()` | `cmd/` + `internal/` |
| `:uninstall` | Unexposes + removes binary files | `cmd/` + `internal/tools/` |

**Hard rules:**
1. `:install` MUST delegate to `:download` RunE — never inline download logic in `cmd/`.
2. Download logic lives in `internal/binaries/` or `internal/phpenv/`, never in `cmd/`.
3. Exposure logic lives in `internal/tools/` — use `tools.Expose()` / `tools.Unexpose()`.
4. `:update` uses `tools.IsExposed()` (not `AutoExpose`) to decide re-exposure — handles manually-exposed tools correctly.
5. New tools must be registered in `internal/tools/tool.go`'s `All` map with correct `ExposureType` and `AutoExpose`.

## Orchestrator commands

`install`, `update`, and `uninstall` are thin orchestrators. They call per-tool `:install`/`:update`/`:uninstall` RunE functions. They MUST NOT contain download, exposure, or cleanup logic — that belongs in the per-tool commands.

- `pv update` self-updates the pv binary first (via `syscall.Exec` re-exec with `--no-self-update`), then delegates to each tool's `:update`.
- `pv restart` delegates to `daemon:restart` in daemon mode, otherwise reloads config via admin API.

## Binary storage rules

- `~/.pv/bin/` — user PATH. **Only** shims and symlinks go here. Never place real binaries.
- `~/.pv/internal/bin/` — private storage. Real binaries (mago, composer.phar, colima) live here.
- `~/.pv/php/{ver}/` — versioned PHP binaries (php, frankenphp) live here.
- Use `config.InternalBinDir()` for private storage paths, `config.BinDir()` for PATH entries.

## UI rules

### Stack overview

The CLI uses a layered Charm stack:
- **fang** (`charm.land/fang/v2`) — wraps Cobra. Handles help pages, usage text, error display (with `ERROR` badge), version flag, and command spacing. Configured in `cmd/root.go` via `fang.Execute()`.
- **huh** (`charm.land/huh/v2`) — interactive forms (multi-select, text input, confirm). Used for `setup` wizard and any future interactive prompts.
- **lipgloss** (`charm.land/lipgloss/v2`) — low-level styling. Used inside `internal/ui/` helpers. Never import v1 (`github.com/charmbracelet/lipgloss`).
- **`internal/ui/`** — spinners, progress bars, status output (✓/✗), tables, trees. All user-facing status output goes through these helpers.

### What fang handles (do NOT reimplement)

- **Help/usage text** — fang styles it. Never set `Long` to replicate usage info. Put usage examples in the `Example` field (fang syntax-highlights them).
- **Error display** — fang shows errors with a styled `ERROR` badge. Never manually print errors and `os.Exit(1)`. Return `error` from `RunE` and let fang handle it.
- **`SilenceUsage` / `SilenceErrors`** — fang sets these globally. Never set them on individual commands.
- **Spacing/padding** — fang manages whitespace around help and error output. Don't add `fmt.Fprintln(os.Stderr)` for visual spacing around errors.
- **Version flag** — provided via `fang.WithVersion()`. Don't add a manual `--version` flag.

### What `internal/ui/` handles (always use these)

- **Long operations**: `ui.Step(label, fn)` — spinner, then `✓ result` or `✗ error`.
- **Downloads**: `ui.StepProgress(label, fn)` — progress bar with percentage.
- **Multi-step commands**: `ui.Header(version)` at start, `ui.Footer(start, docsURL)` at end.
- **Lists/tables**: `ui.Table(headers, rows)` or `ui.Tree(items)`.
- **One-liners**: `ui.Success(text)`, `ui.Fail(text)`, `ui.Subtle(text)`.
- All output goes to `os.Stderr` (stdout is reserved for machine-readable output like `pv env`).

### Error handling pattern

- **Simple errors**: return `fmt.Errorf(...)` — fang displays it with styled `ERROR` badge.
- **After `ui.Step` / `ui.StepProgress`**: these already print `✗` on failure and return `ui.ErrAlreadyPrinted`. The custom fang error handler in `cmd/root.go` skips re-display for this sentinel.
- **Never use the sandwich pattern**: don't do `fmt.Fprintln` + `ui.Fail()` + `cmd.SilenceUsage = true` + `return ErrAlreadyPrinted`. Just return the error.

### Interactive forms

- Use **huh** (`charm.land/huh/v2`) for any interactive user input (multi-select, text fields, confirmations).
- Never use raw `fmt.Scan` / `bufio.Scanner` for interactive input.

### Hard don'ts

1. **Errors**: always `return fmt.Errorf(...)` — fang displays them. Never `fmt.Print` an error manually.
2. **Status output**: use `ui.*` helpers (`ui.Success`, `ui.Fail`, `ui.Subtle`, `ui.Step`, etc.) — never raw `fmt.Print*` for new code. Legacy uses remain in older commands.
3. Never import lipgloss v1 (`github.com/charmbracelet/lipgloss`). Always use `charm.land/lipgloss/v2`.
4. Never set `SilenceUsage` or `SilenceErrors` on commands — fang owns this.
5. Never add `--version` flags — fang provides this.
6. Put usage examples in `Example:` field, not `Long:` — fang syntax-highlights `Example`.
7. Don't add `fmt.Fprintln(os.Stderr)` for blank-line spacing around errors — fang handles spacing.

## Import cycle: phpenv ↔ tools

`phpenv` and `tools` cannot import each other. This is resolved via callback:
- `phpenv.ExposeFunc` is a `func(name string) error` variable
- `phpenv/shim.go` init() wires it to `tools.Expose()`
- When adding new cross-package dependencies, use the same callback pattern — don't create import cycles.

## Testing conventions

- **Filesystem isolation**: always use `t.Setenv("HOME", t.TempDir())` — never touch the real home dir.
- **Cmd tests**: build fresh cobra command trees per test to avoid state leaking.
- **Registry**: in-memory + explicit save. `Load()` → mutate → `Save()`.
- **E2E tests**: live in `scripts/e2e/`, run on GitHub Actions (macOS). Source `scripts/e2e/helpers.sh`. Use these for anything needing real binaries, network, DNS, or HTTPS. Add new phases to `.github/workflows/e2e.yml`.

## Multi-version PHP

- Main FrankenPHP serves on :443/:80, proxies non-global versions via `reverse_proxy`.
- Secondary FrankenPHP per version on high port: `8000 + major*100 + minor*10` (8.3 → 8830).
- Version resolution order: `.pv-php` file → `composer.json` require.php → global default.

## Services

- Each backing service (mysql, postgres, redis, mail, s3) implements `services.Service` interface.
- Services run as Docker containers via Colima. Container operations go through `container.Engine`.
- Service commands use `service:action` format. New services need: implementation in `internal/services/`, command in `cmd/service_*.go`.
