# Repository Instructions

## Working rules

- Keep changes simple. Avoid adding dependencies, wrappers, abstractions, prompts, or styling that do not clearly pay for themselves. Ask before removing existing dependencies or established patterns.
- Use Go for repository logic. Do not add Python/Ruby/Node/etc. dependencies.
- When working on Go code, activate the repo-local `golang-pro` and `modern-go` skills first.
- Before each commit. you must always run "go-simplifer" skill on the code
- Prefer explicit, reviewable configuration over hidden inference or magic.
- Keep commands scriptable: return errors, keep stdout for pipeable output, and write human status to stderr.
- Treat generated state, downloaded tools, logs, and user-authored config as separate concerns.
- Do not preserve backwards compatibility unless the project is past GA or a task explicitly asks for it.
- For rewrite work, read `docs/rewrite/01-prd.md` and `docs/rewrite/02-architecture.md` first.
- If `legacy/prototype/` exists, treat it as reference-only unless a task explicitly targets it.
- Avoid carrying forward dependencies or patterns from old code by default; re-justify them in the current design. Ask before removing anything already in use.

## Testing

- Always try to add or update tests for changed behavior.
- Prefer focused tests while iterating; run the full check before handing off Go changes.
- Tests that touch pv state must isolate `HOME` with `t.Setenv("HOME", t.TempDir())`.
- Do not use `t.Parallel()` in tests that call `t.Setenv` or mutate global command/state.
- Before handing off Go changes, run: `gofmt -w .`, `go vet ./...`, `go build ./...`, `go test ./...`.

## Safety

- Never run expensive artifact workflows unless the user explicitly asks.
- If an affected artifact family is ambiguous, ask instead of running everything.
- Do not reintroduce hidden service/env setup based on `.env` hints.
