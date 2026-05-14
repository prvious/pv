# Repository Instructions

## Project

- `pv` is a Go Cobra CLI for local PHP dev: FrankenPHP, versioned PHP, Composer/Mago, `.test` HTTPS, and native supervised services. No Docker or VM layer.
- Use Go for repo logic. Do not add Python/Ruby/Node/etc. dependencies; test fakes that need binaries should be small Go `main` packages under `internal/.../testdata/` and built with `go build`.
- When working on Go code, always activate the repo-local `golang-pro` and `modern-go` skills first.
- This project is still in Prototyping stages. we do NOT care about backwards compatibility. we do NOT have any users. for the people testing simply unistall and installing again when new version comes out is the only way to safely update

## Commands

- Build: `go build -o pv .`
- Build with a version: `go build -ldflags "-X github.com/prvious/pv/cmd.version=1.0.0"`
- Focused tests: `go test ./internal/registry/` or `go test ./cmd/ -run TestLink`
- Before handing off Go changes: `gofmt -w .`, `go vet ./...`, `go build ./...`, `go test ./...`

## Architecture

- Entry point is `main.go` -> `cmd.Execute()`; `cmd/root.go` wraps the Cobra root with Fang.
- `cmd/` holds core/orchestrator commands plus thin registration shims. Group commands live in `internal/commands/<group>/` with `Register(parent *cobra.Command)` and exported `Run*` wrappers for orchestrators.
- Tool/service commands use `tool:action`; core commands (`link`, `start`, `stop`, etc.) are plain.
- Current service groups are first-class commands: `postgres:*` (`pg:*` alias), `mysql:*`, `redis:*`, `rustfs:*` (`s3:*` alias), and `mailpit:*` (`mail:*` alias). Add new services the same way; do not introduce a generic `service:*` group.
- Lifecycle implementation belongs in `internal/{phpenv,binaries,tools,postgres,mysql,redis,rustfs,mailpit,...}`, not in Cobra wrappers. `install`, `update`, and `uninstall` delegate to per-tool/service `Run*` helpers.
- `pv.yml` is the project contract; `pv init` generates it. Do not reintroduce hidden service/env setup based on `.env` hints.

## Tools And Storage

- Managed tools follow `:download`, `:path`, `:install`, `:update`, `:uninstall`; `:install` delegates to `:download`, and exposure uses `internal/tools`.
- Real binaries never go in `~/.pv/bin/`; that directory is only shims and symlinks. Use `config.InternalBinDir()` for private binaries and `config.BinDir()` for PATH entries.
- PHP installs currently live under `~/.pv/php/{ver}/`; use `config.PhpVersionDir()` and `config.PortForVersion()` instead of hardcoding paths or port math.

## CLI UI

- Fang owns help, errors, spacing, and `--version`; production commands should use `RunE`, return errors, and not set `SilenceUsage`/`SilenceErrors`.
- New user-facing status output should use `internal/ui` helpers and stderr. Use stdout only for commands intentionally meant for piping, such as `pv env` and logs.
- Use `charm.land/huh/v2` for prompts and `charm.land/lipgloss/v2` for styling; never import `github.com/charmbracelet/lipgloss`.

## Testing

- Tests that touch pv state must isolate HOME with `t.Setenv("HOME", t.TempDir())`.
- Do not use `t.Parallel()` in tests that call `t.Setenv` or mutate pv/Cobra globals.
- Cobra tests should build a fresh command tree per test; do not mutate the package-level `rootCmd`.
- Registry changes are in-memory until `Save()`; tests usually `Load()` -> mutate -> `Save()`.
- E2E lives in `scripts/e2e/` and `.github/workflows/e2e.yml` on macOS. Use it for real binaries, network, DNS, HTTPS, or sudo behavior.
- Source `scripts/e2e/helpers.sh` in E2E scripts; captured Lipgloss output needs `strip_ansi` before assertions.

## CI And Releases

- Manual `build-artifacts.yml` dispatches are expensive; pass `skip_*` for unaffected families. The full run builds FrankenPHP/PHP CLI 8.3/8.4/8.5, Postgres 17/18, MySQL 8.0/8.4/9.7, Mailpit, RustFS, and Redis.
- If the affected artifact family is ambiguous, ask first; never run all artifact jobs "just to be safe".
- `release-pv.yml` publishes the `pv` binary on `v*` tags via GoReleaser. The rolling `artifacts` release is produced only by a full `build-artifacts.yml` run on `main`.
