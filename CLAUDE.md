# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is pv

`pv` is a local development server manager powered by FrankenPHP (Caddy + embedded PHP). It replaces Docker for local dev by managing a single FrankenPHP instance that serves projects under `.test` domains with HTTPS. Written in Go using cobra for CLI commands. See `plan.md` for the full vision and phased development plan.

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
cmd/                          # cobra commands (root, link, unlink, list)
  root.go                     # rootCmd definition, Execute() function
internal/
  config/                     # path helpers for ~/.pv/ directory structure
    paths.go                  # PvDir, ConfigDir, SitesDir, LogsDir, DataDir, BinDir, RegistryPath, EnsureDirs
  registry/                   # project registry (JSON persistence in ~/.pv/data/registry.json)
    registry.go               # Registry struct with Add, Remove, Find, FindByPath, List, Load, Save
```

All state lives under `~/.pv/`. Path resolution goes through `config.PvDir()` which reads `os.UserHomeDir()` (i.e., `$HOME`). This is the key mechanism for test isolation — tests set `HOME` to a temp dir.

## Key patterns

- **Test isolation**: All tests that touch the filesystem use `t.Setenv("HOME", t.TempDir())` so registry reads/writes go to a temp dir instead of the real `~/.pv/`.
- **Cmd tests**: Build fresh cobra command trees per test (via helpers like `newLinkCmd()`) to avoid state leaking from package-level globals (`rootCmd`, `linkName`). The fresh command's `RunE` delegates to the real command's `RunE`.
- **Registry is in-memory + explicit save**: `Load()` reads JSON from disk, methods mutate in-memory, `Save()` writes back. Commands call `Load`, mutate, then `Save`.
- **Commands use `RunE`** (not `Run`) so errors propagate instead of calling `os.Exit`.

## Development status

Phase 1 (CLI skeleton) is complete. Future phases add project detection, Caddyfile generation, FrankenPHP lifecycle management, and first-time setup (`pv install`). See `plan.md` for details.
