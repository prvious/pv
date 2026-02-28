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
  phpenv/                     # PHP version management
    phpenv.go                 # InstalledVersions, IsInstalled, SetGlobal, Remove
    install.go                # Download FrankenPHP from prvious/pv releases + PHP CLI from static-php.dev
    resolve.go                # ResolveVersion: .pv-php → composer.json → global default
    available.go              # AvailableVersions from GitHub releases
    shim.go                   # WriteShims: creates ~/.pv/bin/php shim script
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
├── bin/           # symlinks to active global version + Mago, Composer
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

## Key patterns

- **Test isolation**: Tests use `t.Setenv("HOME", t.TempDir())` so filesystem ops go to a temp dir.
- **Cmd tests**: Build fresh cobra command trees per test to avoid state leaking.
- **Registry is in-memory + explicit save**: `Load()` → mutate → `Save()`.
- **Commands use `RunE`** (not `Run`) so errors propagate.
- **Version resolution**: `.pv-php` file → `composer.json` require.php → global default.
- **Caddy site config**: `GenerateSiteConfig(project, globalPHP)` — empty globalPHP = single-version mode.
