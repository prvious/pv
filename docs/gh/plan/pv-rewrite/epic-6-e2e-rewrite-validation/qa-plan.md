# QA Plan: Epic 6 - E2E Rewrite Validation

## Quality Gates

| Gate | Required Evidence |
| --- | --- |
| Harness | Active rewrite binary is built, sandbox is isolated, command output is captured. |
| Hermeticity | Tier 0 does not touch real HOME, real `~/.pv`, DNS, TLS trust, browser, network downloads, or live resources. |
| Lifecycle | Init, link, env, setup, status, gateway, and helpers pass black-box scenarios. |
| Failure UX | Missing install, blocked dependency, setup failure, process crash, and gateway failure include next actions. |
| Recovery | Corrective action clears blocked or failed status in follow-up scenario. |
| Release gate | Tier 0 command and `tests.yml` job are documented, scriptable, and fail closed. |
| CI-only safety | Tier 1 and Tier 2 refuse local execution and run only in GitHub-hosted CI VMs. |

## Manual QA Checklist

- [ ] Run the Tier 0 E2E gate from a clean checkout of the Epic 6 branch.
- [ ] Confirm E2E evidence records binary path, sandbox root, scenarios, commands, and log paths.
- [ ] Confirm no files were written outside the sandbox.
- [ ] Confirm failure scenarios include next actions.
- [ ] Confirm recovery scenarios clear the prior failure.
- [ ] Confirm Tier 1 refuses local execution when `CI` is not `true`.
- [ ] Confirm Tier 2 refuses local execution when `CI` is not `true`.

## Review Checklist

- [ ] E2E scenarios invoke the compiled `pv` binary.
- [ ] E2E tests assert public CLI behavior and filesystem/log outputs.
- [ ] Default E2E uses fake artifact catalogs and fake process behavior.
- [ ] Tests that mutate pv state isolate `HOME`.
- [ ] Tests that call `t.Setenv` do not call `t.Parallel`.
- [ ] Tier 1 and Tier 2 controls refuse local execution and require GitHub-hosted CI.
- [ ] No default E2E test mutates `/etc/hosts`, trust stores, keychains, browsers, or real `~/.pv`.
- [ ] PR description lists exact verification commands run.

## Required Verification

```bash
gofmt -w .
go vet ./...
go build ./...
go test ./...
```

After the Tier 0 E2E command exists, run it before release-readiness handoff.
Tier 1 and Tier 2 must run only in GitHub-hosted CI VMs.

If any CI-only E2E run executes, document:

- tier;
- host actions;
- resource names and versions;
- temp directories;
- ports used;
- cleanup performed;
- follow-up issue for failures.
