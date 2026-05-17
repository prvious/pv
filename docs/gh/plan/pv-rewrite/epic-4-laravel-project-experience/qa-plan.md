# QA Plan: Epic 4 - Laravel Project Experience

## Quality Gates

| Gate | Required Evidence |
| --- | --- |
| Contract | `pv.yml` schema, parser, generator, and validation tests pass. |
| Init | `pv init` is deterministic and refuses overwrite unless forced. |
| Link | `pv link` records durable desired state and never infers from `.env`. |
| Env | Managed env writer preserves user values and backs up before mutation. |
| Setup | Setup runner uses managed PHP and stops on first failure. |
| Gateway | Route rendering is deterministic; DNS, TLS, and browser behavior use adapters. |
| Helpers | Helper commands resolve current project and declared resources. |

## Manual QA Checklist

- [ ] Run `pv init` in a Laravel fixture and review generated `pv.yml`.
- [ ] Run `pv init` again and confirm overwrite refusal.
- [ ] Run forced init and confirm output remains deterministic.
- [ ] Run `pv link` with declared services and confirm only declared env keys are
  written.
- [ ] Confirm user-authored `.env` lines are preserved.
- [ ] Run setup with a failing command and confirm fail-fast behavior.
- [ ] Render gateway routes for primary host and aliases.
- [ ] Confirm `pv open` resolves the linked app URL through adapter behavior.
- [ ] Run helper commands with missing declared resources and confirm actionable
  errors.

## Review Checklist

- [ ] No service decisions are read from `.env`.
- [ ] No command mutates `.env` except the managed env writer.
- [ ] Generated YAML ordering is stable.
- [ ] Project desired state is recorded before daemon signal.
- [ ] Gateway code does not directly mutate privileged OS state outside adapters.
- [ ] Helper commands never auto-create missing resources.
- [ ] Tests that mutate pv state isolate `HOME`.
- [ ] Tests that call `t.Setenv` do not call `t.Parallel`.
- [ ] PR description lists exact verification commands run.

## Required Verification

```bash
gofmt -w .
go vet ./...
go build ./...
go test ./...
```

If a task deliberately performs real DNS, TLS, or browser integration, document:

- hostnames used;
- certificate or trust-store action;
- files changed;
- cleanup performed;
- why adapter coverage was insufficient.
