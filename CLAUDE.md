# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is pv

`pv` is a local development server manager powered by FrankenPHP (Caddy + embedded PHP). It replaces Docker for local dev by managing FrankenPHP instances that serve projects under `.test` domains with HTTPS. Supports multiple PHP versions simultaneously. Written in Go using cobra for CLI commands. See `plan.md` for the full vision.

## Commands

```bash
go build -o pv .              # build the binary
go test ./...                  # run all tests
go test ./internal/registry/   # run tests for one package
go test ./cmd/ -run TestLink   # run tests matching a pattern
go test ./... -v               # verbose output
```

## Architecture

```
main.go                       # entry point — calls cmd.Execute()
cmd/                          # cobra commands
  root.go                     # rootCmd, Execute()
  link.go, unlink.go, list.go # project management
  start.go, stop.go, restart.go, status.go, log.go  # server lifecycle
  install.go, update.go       # first-time setup and updates
  php.go, php_install.go, php_list.go, php_remove.go  # PHP version management
  use.go                      # switch global PHP version
internal/
  config/                     # path helpers for ~/.pv/ directory structure
    paths.go                  # PvDir, PhpDir, PhpVersionDir, PortForVersion, etc.
    settings.go               # TLD + GlobalPHP settings
  registry/                   # project registry (JSON in ~/.pv/data/registry.json)
    registry.go               # Project{Name,Path,Type,PHP}, Registry with CRUD + GroupByPHP
  tools/                      # tool abstraction layer
    tool.go                   # Tool struct, registry, Expose/Unexpose/IsExposed/ExposeAll
    shims.go                  # PHP + Composer shim scripts
  phpenv/                     # PHP version management
    phpenv.go                 # InstalledVersions, IsInstalled, SetGlobal, Remove
    install.go                # Download FrankenPHP from prvious/pv releases + PHP CLI from static-php.dev
    resolve.go                # ResolveVersion: .pv-php → composer.json → global default
    available.go              # AvailableVersions from GitHub releases
    shim.go                   # WriteShims (delegates to tools.ExposeAll)
  caddy/                      # Caddyfile generation (multi-version aware)
    caddy.go                  # GenerateSiteConfig(project, globalPHP), GenerateAllConfigs, GenerateVersionCaddyfile
  server/                     # process management
    process.go                # Start supervisor (DNS + main FP + secondary FPs), ReconfigureServer
    frankenphp.go             # StartFrankenPHP, StartVersionFrankenPHP, Reload
    dns.go                    # Embedded DNS server on port 10053
  binaries/                   # binary download (Mago, Composer)
  detection/                  # project type detection (laravel, php, static)
  setup/                      # install prerequisites, resolver, selftest
```

## Directory layout (~/.pv/)

```
~/.pv/
├── bin/                        # User PATH — shims and symlinks ONLY
│   ├── php                     # Shim (version resolution)
│   ├── composer                # Shim (wraps PHAR with PHP)
│   ├── frankenphp → ../php/{ver}/frankenphp  # Symlink
│   └── mago → ../internal/bin/mago           # Symlink
├── internal/bin/               # pv's private toolbox — never on PATH
│   ├── colima                  # Container runtime
│   ├── mago                    # Real binary
│   └── composer.phar           # Real PHAR
├── config/        # Caddyfiles + settings.json
│   ├── Caddyfile              # main process
│   ├── php-8.3.Caddyfile      # secondary process (if needed)
│   ├── sites/                 # per-project configs for main process
│   └── sites-8.3/            # per-project configs for secondary process
├── data/          # registry.json, versions.json, pv.pid
├── logs/          # caddy.log, caddy-8.3.log
└── php/           # per-version binaries
    ├── 8.3/frankenphp + php
    ├── 8.4/frankenphp + php
    └── 8.5/frankenphp + php
```

## Multi-version architecture

- **Main FrankenPHP** (global version): serves on :443/:80, handles projects using the global PHP version directly via `php_server`, and proxies non-global projects via `reverse_proxy`.
- **Secondary FrankenPHP** (per non-global version): serves on high port (8830 for 8.3, 8840 for 8.4, etc.), HTTP only, `admin off`. The main process proxies to these.
- **Port scheme**: `8000 + major*100 + minor*10` (e.g., PHP 8.3 → 8830).
- FrankenPHP binaries come from `prvious/pv` GitHub releases (format: `frankenphp-{platform}-php{version}`).

## Testing strategy

- **Unit tests** (`go test ./...`): Run locally. Use `t.Setenv("HOME", t.TempDir())` for filesystem isolation. Fake binaries (bash scripts) can stand in for real PHP when testing shims.
- **E2E tests** (`.github/workflows/e2e.yml` + `scripts/e2e/`): Run on GitHub Actions (macOS runner) to simulate real end-user flows. These tests use real PHP, real Composer, real FrankenPHP — things we can't easily run locally. **When your feature involves real binary execution, network calls, DNS, HTTPS, or anything that needs a full `pv install` environment, add an e2e script in `scripts/e2e/` and wire it into the workflow.** Each script sources `scripts/e2e/helpers.sh` for `assert_contains`, `assert_fails`, `curl_site`, etc. The workflow phases run sequentially: install → verify → fixtures → link → start → curl → shim → composer → errors → stop → lifecycle → update → verify-final.

## Tool command pattern (:download / :path / :install)

Every managed tool follows a strict three-command pattern:

- **`:download`** — Fetches the binary to private storage (`internal/bin/` or `php/{ver}/`). Contains the actual download logic.
- **`:path`** — Exposes the binary to the user's PATH (`~/.pv/bin/`) via shim or symlink. Supports `--remove` to unexpose.
- **`:install`** — Orchestrator only. Calls `:download` RunE, then `tools.Expose()`. **Never duplicates download logic.**

Rules:
1. `:install` must always delegate to `:download` — never inline download logic.
2. Download logic lives in `internal/binaries/` or `internal/phpenv/`, not in `cmd/`.
3. Exposure logic lives in `internal/tools/` — shims and symlinks are managed by `tools.Expose()` / `tools.Unexpose()`.
4. The `internal/tools/` package defines each tool's `ExposureType` (None, Symlink, Shim) and `AutoExpose` flag.
5. `tools.ExposeAll()` is the single entry point for batch-exposing all auto-exposed tools (called by `pv install` and `pv setup`).

## Key patterns

- **Test isolation**: Tests use `t.Setenv("HOME", t.TempDir())` so filesystem ops go to a temp dir.
- **Cmd tests**: Build fresh cobra command trees per test to avoid state leaking.
- **Registry is in-memory + explicit save**: `Load()` → mutate → `Save()`.
- **Commands use `RunE`** (not `Run`) so errors propagate.
- **Version resolution**: `.pv-php` file → `composer.json` require.php → global default.
- **Caddy site config**: `GenerateSiteConfig(project, globalPHP)` — empty globalPHP = single-version mode.
