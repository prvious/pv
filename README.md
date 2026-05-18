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

Tier 0 E2E is the default local-safe release gate:

```bash
go test ./test/e2e/scenarios
```

Tier 1 and Tier 2 E2E are CI-only. They require `CI=true`,
`GITHUB_ACTIONS=true`, and `RUNNER_ENVIRONMENT=github-hosted` before they run:

```bash
go test -tags=e2e_tier1 ./test/e2e/tier1
go test -tags=e2e_tier2 ./test/e2e/tier2
```

Tier 2 prints the DNS, TLS, and browser host actions it intends to perform
before enforcing the CI-only guard.

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
