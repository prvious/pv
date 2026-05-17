# QA Plan: Epic 3 - Runtime, Daemon, And Resources

## Quality Gates

| Gate | Required Evidence |
| --- | --- |
| Architecture | Commands request desired state; controllers reconcile; supervisor only manages processes. |
| Runtime | PHP and Composer use managed runtime paths and no implicit system PHP fallback. |
| Daemon | Reconcile errors are persisted as observed status and do not crash the daemon. |
| Supervisor | Package and tests prove no resource-specific behavior leaks into lifecycle code. |
| Resources | Each resource owns its flags, paths, readiness, env values, and status mapping. |
| Secrets | RustFS and other secret-like values are redacted from status and logs. |
| Tests | Focused unit tests plus narrow integration checks pass. |

## Manual QA Checklist

- [ ] Request PHP runtime desired state and confirm status shows pending or ready.
- [ ] Request Composer desired state with missing PHP and confirm blocked status
  includes a next action.
- [ ] Confirm PHP and Composer shims point at managed paths.
- [ ] Start daemon with fake or controlled resources and confirm reconcile
  status is persisted.
- [ ] Start Mailpit through the supervisor and confirm PID, ports, log path, and
  readiness are reported.
- [ ] Start a database resource in a temp state root and confirm data/log paths
  are canonical.
- [ ] Confirm Redis env values are emitted only from declared resource state.
- [ ] Confirm RustFS status does not print secret values.

## Review Checklist

- [ ] No code path infers services from `.env`.
- [ ] No command performs hidden unrelated setup.
- [ ] No resource stores binaries, data, logs, state, or cache outside canonical
  layout helpers.
- [ ] No supervisor API or test references a concrete resource name.
- [ ] Resource-specific behavior stays inside the resource package.
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

If a task deliberately runs real local resource processes, document:

- resource name and version;
- temp data directory;
- ports used;
- cleanup performed;
- why fake-process coverage was insufficient.
