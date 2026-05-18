# Working On The Rewrite

The repository root is the active rewrite workspace. `legacy/prototype/` is the
old implementation and should be treated as reference-only.

## Boundaries

- Active rewrite code belongs at the repository root.
- Prototype code belongs in `legacy/prototype/`.
- New rewrite code must not import prototype packages.
- Behavior and tests may be copied from the prototype only when they are
  deliberately re-justified against the rewrite architecture.

## Command Layer

The rewrite starts with the smallest scriptable command layer that supports
predictable help, useful errors, stable exit codes, clean stdout for pipeable
commands, stderr for human status, and testable command construction.

Do not add Fang by default. Add any command parser or presentation dependency
only when it removes more complexity than it introduces.

## Verification

Run root checks from the repository root:

```bash
gofmt -w .
go vet ./...
go build ./...
go test ./...
```

Run prototype checks from the prototype module:

```bash
cd legacy/prototype
gofmt -w .
go vet ./...
go build ./...
go test ./...
```
