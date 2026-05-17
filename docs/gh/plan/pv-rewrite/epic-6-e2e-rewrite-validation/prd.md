# Epic PRD: E2E Rewrite Validation

## Problem

The rewrite plan covers architecture, implementation, unit tests, integration
tests, status, QA, and scope control, but it does not dedicate an epic to
black-box end-to-end tests of the new rewrite. Without E2E coverage, the MVP can
pass package-level tests while failing the actual Laravel developer workflow:
initialize a project, link it, reconcile resources, serve HTTPS `.test`, run
helpers, observe failures, and recover.

## Solution

Add a sixth epic that builds a safe E2E harness and validates the full rewrite
through staged workflows. The E2E suite uses the compiled `pv` binary, isolated
`HOME`, temporary project fixtures, deterministic ports, fake or local artifact
sources, and CI-only gates for real process and privileged host checks. It proves
the rewrite works as a user experiences it without touching the user's real
`~/.pv`, project files, DNS, trust store, browser, or long-running services by
default.

## Product Promise

A maintainer can run the E2E gate and get evidence that the rewritten pv works
for the Laravel-first MVP as an actual command-line product, not just as isolated
packages.

## MVP Scope

- E2E harness that builds and invokes the active rewrite `pv` binary.
- Sandboxed test home, state root, project root, ports, logs, and cleanup.
- Minimal generated Laravel fixture sufficient for `pv init`, `pv link`, setup,
  env rendering, gateway routing, helper commands, and status checks.
- Hermetic E2E mode using fake host adapters, fake artifact catalogs, and fake
  runnable processes where real OS mutation is not required.
- CI-only real-process checks for daemon, supervisor, gateway, and selected
  resources in GitHub-hosted runners.
- CI-only privileged host checks for DNS, TLS trust, and browser open behavior in
  GitHub-hosted runners.
- `.github/workflows/tests.yml` jobs that run the required hermetic E2E suite and
  CI-only host tiers.
- Release evidence template for Tier 0, CI Tier 1, and CI Tier 2 checks.

## Out Of Scope For MVP

- Running E2E tests against the legacy prototype.
- Docker or VM orchestration for E2E isolation.
- Network artifact downloads in default E2E runs.
- Mutating `/etc/hosts`, trust stores, keychains, browsers, or real `~/.pv` by
  default.
- Full browser automation or frontend visual regression.
- Load testing, soak testing, and performance benchmarking beyond basic timeout
  guardrails.
- Cross-platform E2E parity on every OS in the first implementation.
- Testing post-MVP capabilities from `../post-mvp-backlog.md`.

## Success Criteria

- The hermetic E2E suite builds the active rewrite binary and runs in isolated
  temp directories.
- E2E tests prove `pv init` creates deterministic `pv.yml` without `.env`
  mutation.
- E2E tests prove `pv link` records project desired state, writes only declared
  env keys, runs declared setup commands, and signals reconciliation after
  durable writes.
- E2E tests prove status reports desired state, observed status, failures, log
  paths, and next actions across representative workflows.
- E2E tests prove helper commands route through current project state and
  declared resources.
- E2E tests prove failure and recovery workflows for missing installs, blocked
  resources, setup failure, crashed runnable process, and gateway failure.
- Default E2E runs do not touch the user's real home, DNS, TLS trust, browser,
  network artifact downloads, or live resource processes.
- CI or release gate documentation identifies which E2E tier is required before
  MVP release and which tiers run only in GitHub-hosted CI VMs.

## Primary Users

- Maintainer: needs release confidence for the rewrite stack.
- Implementation agent: needs exact E2E expectations before claiming an epic is
  complete.
- Automation user: benefits from black-box validation of stdout/stderr, exit
  codes, and scriptable behavior.

## Key Architecture Rule

```text
E2E tests exercise pv like a user.
Default E2E tests are hermetic.
Real host mutation runs only in GitHub-hosted CI VMs and is documented.
E2E evidence gates the rewrite stack after Epic 5.
```
