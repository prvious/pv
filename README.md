# pv

`pv` is being rewritten as a Laravel-first local desired-state control plane.

The active rewrite module lives at the repository root. The previous prototype
implementation has been moved to `legacy/prototype/` as a buildable reference
module.

## Active Rewrite

Read these first:

- `docs/rewrite/01-prd.md`
- `docs/rewrite/02-architecture.md`
- `docs/rewrite/03-working-on-rewrite.md`

The root command layer is intentionally minimal. Do not carry Fang or prototype
orchestration forward by default. Commands should parse intent, validate it,
write desired state when appropriate, and leave reconciliation work to
controllers.

Root verification:

```bash
gofmt -w .
go vet ./...
go build ./...
go test ./...
```

## Prototype Reference

`legacy/prototype/` is the old implementation. It remains available for
behavioral reference only.

Rules:

- Do not import prototype packages from the active rewrite module.
- Copy behavior or tests forward deliberately when they still fit the rewrite.
- Run prototype checks from `legacy/prototype/`.

Prototype verification:

```bash
cd legacy/prototype
gofmt -w .
go vet ./...
go build ./...
go test ./...
```
