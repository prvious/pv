# PR 21 Diagnostics Status Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement PR 21 from `IMPLEMENTATION.md`: `pv status`, `pv logs`, `pv doctor`, `pv jobs`, and PR21 JSON outputs for PV-090 through PV-095.

**Architecture:** Keep diagnostics as CLI read models composed from existing state, PV-owned files, daemon health, and platform inspectors. Add one structured daemon log file path and file append initialization, but do not add new aggregate observed-state subjects or a richer daemon protocol.

**Tech Stack:** Rust 2024, clap, serde/serde_json, SQLite state via `state::Database`, platform inspectors, `linemux` for follow-mode active log multiplexing, `anstyle` for TTY-safe log coloring, insta snapshots, cargo nextest.

---

## File Structure

- Modify `DESIGN.md` with the resolved public-behavior clarifications before code relies on them.
- Modify root `Cargo.toml`, `Cargo.lock`, and `crates/cli/Cargo.toml` to add `anstyle`, `linemux`, and `serde` only for PR21 needs.
- Modify `crates/state/src/paths.rs` to add `daemon_log()`, `launchd_stdout_log()`, `launchd_stderr_log()`, `gateway_access_log()`, and `gateway_error_log()` helpers while preserving the existing combined `gateway_log()`.
- Modify `crates/daemon/src/lib.rs` and a small daemon-local logging module to append structured daemon/reconciliation lines to `~/.pv/logs/daemon.log`.
- Modify `crates/platform/src/launch_agent.rs` to expose a read-only LaunchAgent runtime inspection helper backed by `launchctl print`.
- Modify `crates/cli/src/environment.rs` to add testable hooks for stdout TTY detection and LaunchAgent runtime inspection.
- Modify `crates/cli/src/args.rs` to add `StatusArgs`, `LogsArgs`, `DoctorArgs`, `JobsArgs`, `ListArgs`, and resource list JSON args.
- Modify `crates/cli/src/commands/mod.rs` to route new commands.
- Create `crates/cli/src/commands/diagnostics.rs` for shared status/doctor/jobs read models and JSON structs.
- Create `crates/cli/src/commands/jobs.rs`, `logs.rs`, `status.rs`, and `doctor.rs` for command-specific rendering and exit-code decisions.
- Modify `crates/cli/src/commands/project.rs` for `pv list --json`.
- Modify `crates/cli/src/commands/artifact_resource.rs` and resource command wrappers for managed resource `*:list --json`.
- Modify `it/cli.rs` for help/completion snapshot coverage.
- Add `crates/cli/tests/jobs.rs`, `logs.rs`, `status.rs`, and `doctor.rs` with integration-style command tests and insta snapshots.
- Extend focused resource-list tests in existing CLI command tests where the command wrapper accepts `--json`.

## Task 1: Baseline And DESIGN Text

**Files:**
- Modify: `DESIGN.md`
- Create: `docs/superpowers/plans/2026-06-10-pr-21-diagnostics-status.md`

- [ ] **Step 1: Verify the isolated branch**

Run:

```bash
git status --short --branch
git merge-base HEAD origin/main
git worktree list --porcelain
```

Expected: branch `feat/pr21-diagnostics-status`, clean worktree before edits, merge-base equal to `origin/main`.

- [ ] **Step 2: Run the baseline build check**

Run:

```bash
cargo check --workspace --all-targets --all-features --locked
```

Expected: PASS before production code changes.

- [ ] **Step 3: Update public design clarifications**

Apply the resolved behavior from the goal file: aggregate status from existing state, minimal JSON without schema envelopes, structured daemon log path, combined Gateway log fallback, severity/source coloring rules, daemon-state classification, and doctor repair hints.

- [ ] **Step 4: Save this implementation plan**

Run:

```bash
git diff -- DESIGN.md docs/superpowers/plans/2026-06-10-pr-21-diagnostics-status.md
```

Expected: docs-only diff.

## Task 2: Add CLI Surface Tests First

**Files:**
- Modify: `it/cli.rs`
- Snapshot updates under `it/snapshots/`
- Modify: `crates/cli/src/args.rs`
- Modify: `crates/cli/src/commands/mod.rs`

- [ ] **Step 1: Write failing help snapshot tests**

Add test cases named `diagnostic_commands_are_documented` and `diagnostic_command_options_are_documented` that run:

```rust
run_pv(&["status", "--help"])?;
run_pv(&["logs", "--help"])?;
run_pv(&["doctor", "--help"])?;
run_pv(&["jobs", "--help"])?;
run_pv(&["list", "--help"])?;
```

Expected new options include `--json`, `logs -n/--lines`, `logs --follow`, `logs --all`, `logs --gateway`, `logs --worker`, `logs --resource`, and `logs --track`.

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo insta test --test-runner nextest -- diagnostic_commands_are_documented diagnostic_command_options_are_documented
```

Expected: FAIL because commands or options do not exist yet.

- [ ] **Step 3: Add clap args and routing stubs**

Add new command variants and args:

```rust
Status(StatusArgs)
Logs(LogsArgs)
Doctor(DoctorArgs)
Jobs(JobsArgs)
List(ListArgs)
```

Resource list variants accept list args, for example `RedisList(ResourceListArgs)`.

- [ ] **Step 4: Verify GREEN for help**

Run:

```bash
cargo insta test --accept --test-runner nextest -- diagnostic_commands_are_documented diagnostic_command_options_are_documented
```

Expected: PASS and accepted snapshots for the new public CLI surface.

## Task 3: Implement Shared Diagnostics Read Models

**Files:**
- Create: `crates/cli/src/commands/diagnostics.rs`
- Modify: `crates/cli/src/commands/mod.rs`
- Modify: `crates/cli/src/environment.rs`
- Modify: `crates/platform/src/launch_agent.rs`
- Test through command tests in later tasks.

- [ ] **Step 1: Add minimal test-only consumers through status/doctor tests**

Write the first status and doctor tests before implementing this module. Those tests must exercise daemon missing, LaunchAgent current, DNS repair required, ports repair required, CA trust missing, failed jobs, and resource runtime states through command output snapshots.

- [ ] **Step 2: Implement read structs**

Add serializable structs for:

```rust
DiagnosticsSnapshot
DaemonDiagnostic
GatewayDiagnostic
IntegrationDiagnostic
ManagedResourceDiagnostic
ProjectDiagnostic
RecentJobDiagnostic
DiagnosticCheck
```

Fields map to visible output and avoid secret-bearing env maps.

- [ ] **Step 3: Implement readers**

Readers open `PvPaths` from `Environment`, then collect:

```rust
Database::recent_jobs()
Database::runtime_observed_states()
Database::managed_resource_tracks()
Database::assigned_ports()
platform::inspect_resolver_file(...)
platform::inspect_pf_anchor_file(...)
platform::inspect_pf_conf_reference(...)
environment.active_pf_redirect_config()
platform::inspect_local_ca_files(...)
environment.trusted_ca_certificates()
platform::inspect_launch_agent_file(...)
environment.launch_agent_runtime_state()
```

Expected: no new daemon protocol request beyond socket health.

## Task 4: Implement `pv jobs`

**Files:**
- Create: `crates/cli/src/commands/jobs.rs`
- Add: `crates/cli/tests/jobs.rs`
- Modify: `crates/cli/src/commands/diagnostics.rs`

- [ ] **Step 1: Write failing plain-output tests**

Add tests named `jobs_lists_empty_history`, `jobs_lists_recent_history`, and `jobs_shows_failure_summary`. Seed jobs with `Database::start_job`, `complete_job`, and `fail_job`.

- [ ] **Step 2: Write failing JSON test**

Add `jobs_json_lists_recent_history` and assert with `insta::assert_json_snapshot!`.

- [ ] **Step 3: Verify RED**

Run:

```bash
cargo nextest run -E 'test(jobs_)' -p cli --all-features --locked
```

Expected: FAIL because `pv jobs` is not implemented.

- [ ] **Step 4: Implement jobs rendering**

Plain output includes id, kind, scope, status, started, finished, and summary/error. JSON output serializes an object with a `jobs` array and no schema envelope.

- [ ] **Step 5: Verify GREEN**

Run:

```bash
cargo insta test --accept --test-runner nextest -- jobs_
cargo nextest run -E 'test(jobs_)' -p cli --all-features --locked
```

Expected: PASS.

## Task 5: Implement `pv logs`

**Files:**
- Create: `crates/cli/src/commands/logs.rs`
- Add: `crates/cli/tests/logs.rs`
- Modify: `crates/state/src/paths.rs`
- Modify: `crates/cli/src/environment.rs`

- [ ] **Step 1: Write failing log selection and validation tests**

Add tests named `logs_defaults_to_daemon_sources`, `logs_rejects_negative_lines`, `logs_caps_lines_at_5000`, `logs_prefixes_multiple_sources`, `logs_reports_missing_selected_source`, `logs_gateway_uses_combined_fallback`, `logs_resource_alias_requires_track_when_ambiguous`, and `logs_resource_alias_infers_single_installed_track`.

- [ ] **Step 2: Write failing follow test with active files**

Add `logs_follow_uses_active_files_after_initial_tail` with temp log files and a short writer thread. The test starts `pv logs --follow -n 0 --all`, appends to active files, and verifies prefixed lines. Use a deterministic stop condition by terminating the child process after the expected lines.

- [ ] **Step 3: Verify RED**

Run:

```bash
cargo nextest run -E 'test(logs_)' -p cli --all-features --locked
```

Expected: FAIL because `pv logs` is not implemented.

- [ ] **Step 4: Implement log source selection**

Default sources are `daemon`, `launchd:stdout`, and `launchd:stderr`. `--gateway` chooses split access/error logs when present and falls back to combined `gateway`. `--worker <track>` resolves `latest` to the manifest default PHP track. `--resource <name>` resolves aliases and infers track only when one installed track exists.

- [ ] **Step 5: Implement tail and follow**

Initial tail reads rotated files plus active file up to the requested cap. Follow mode prints initial tail first, then uses `linemux` for active files only. `-n` accepts `0..=5000`; negative values are rejected by clap validation or command validation.

- [ ] **Step 6: Implement coloring**

Use `anstyle` only when stdout is an interactive TTY and neither `--no-color` nor `NO_COLOR` is active. Color source prefixes and obvious severity words; snapshots run plain.

- [ ] **Step 7: Verify GREEN**

Run:

```bash
cargo insta test --accept --test-runner nextest -- logs_
cargo nextest run -E 'test(logs_)' -p cli --all-features --locked
```

Expected: PASS.

## Task 6: Add Structured Daemon Log File

**Files:**
- Modify: `crates/state/src/paths.rs`
- Modify: `crates/daemon/src/lib.rs`
- Create or modify daemon-local logging helper.
- Add focused daemon or CLI log tests only if the file-writing behavior is directly exercised.

- [ ] **Step 1: Add failing path/log test**

Add a focused test that starts a daemon reconciliation path or daemon run helper in a temp PV home and verifies `PvPaths::daemon_log()` is the structured daemon log source used by default `pv logs`.

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo nextest run -E 'test(daemon_log|logs_defaults_to_daemon_sources)' --workspace --all-features --locked
```

Expected: FAIL before daemon log path/source implementation.

- [ ] **Step 3: Implement minimal append logging**

Create `~/.pv/logs/daemon.log` through PV filesystem helpers and append JSONL-like daemon/reconciliation events with timestamp, level, target, and message. Do not replace LaunchAgent stdout/stderr.

- [ ] **Step 4: Verify GREEN**

Run:

```bash
cargo nextest run -E 'test(daemon_log|logs_defaults_to_daemon_sources)' --workspace --all-features --locked
```

Expected: PASS.

## Task 7: Implement `pv status`

**Files:**
- Create: `crates/cli/src/commands/status.rs`
- Add: `crates/cli/tests/status.rs`
- Modify: `crates/cli/src/commands/diagnostics.rs`

- [ ] **Step 1: Write failing status snapshots**

Add tests named `status_reports_disabled_daemon_without_breaking_integrations`, `status_reports_daemon_down_after_setup_as_failure`, `status_reports_runtime_and_resource_states`, `status_reports_recent_failed_jobs`, `status_json_redacts_secret_context`, and `status_pending_project_is_success`.

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo nextest run -E 'test(status_)' -p cli --all-features --locked
```

Expected: FAIL before `pv status` exists.

- [ ] **Step 3: Implement status rendering and exit codes**

Plain output is compact sections: daemon, integrations, gateway/workers, managed resources, projects needing attention, recent errors, and log directory. JSON output serializes the same categories. Exit success for healthy or pending-only states; failure for daemon down after setup, Gateway failed, DNS/ports repair required, CA required failure, or failed reconciliation.

- [ ] **Step 4: Verify GREEN**

Run:

```bash
cargo insta test --accept --test-runner nextest -- status_
cargo nextest run -E 'test(status_)' -p cli --all-features --locked
```

Expected: PASS.

## Task 8: Implement `pv doctor`

**Files:**
- Create: `crates/cli/src/commands/doctor.rs`
- Add: `crates/cli/tests/doctor.rs`
- Modify: `crates/cli/src/commands/diagnostics.rs`

- [ ] **Step 1: Write failing doctor snapshots**

Add tests named `doctor_passes_when_required_checks_pass`, `doctor_fails_with_repair_commands`, `doctor_warnings_do_not_fail`, and `doctor_is_read_only`.

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo nextest run -E 'test(doctor_)' -p cli --all-features --locked
```

Expected: FAIL before `pv doctor` exists.

- [ ] **Step 3: Implement doctor checks**

Checks cover layout permissions, LaunchAgent registration, daemon health, DNS resolver files, `pf` files and active redirects, CA files/trust, manifest cache readability, runtime metadata/pids when inspectable, and recent failed jobs. Suggested commands are limited to implemented commands: `pv daemon:enable`, `pv daemon:restart`, `pv dns:install`, `pv ports:install`, `pv ca:trust`, and `pv setup`.

- [ ] **Step 4: Verify GREEN**

Run:

```bash
cargo insta test --accept --test-runner nextest -- doctor_
cargo nextest run -E 'test(doctor_)' -p cli --all-features --locked
```

Expected: PASS.

## Task 9: Implement Remaining JSON Outputs

**Files:**
- Modify: `crates/cli/src/commands/project.rs`
- Modify: `crates/cli/src/commands/artifact_resource.rs`
- Modify: resource wrappers in `composer.rs`, `mailpit.rs`, `mysql.rs`, `php.rs`, `postgres.rs`, `redis.rs`, `rustfs.rs`
- Extend tests in existing CLI resource test files.

- [ ] **Step 1: Write failing JSON tests**

Add tests for `pv list --json`, `redis:list --json`, `mysql:list --json`, `postgres:list --json`, `pg:list --json`, `mailpit:list --json`, `mail:list --json`, `rustfs:list --json`, and `s3:list --json`. Keep `project:env --json` unchanged.

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo nextest run -E 'test(list_json|resource_list_json|project_env_json)' -p cli --all-features --locked
```

Expected: FAIL for new JSON outputs and PASS for unchanged `project:env --json`.

- [ ] **Step 3: Implement JSON renderers**

Use typed `serde::Serialize` structs. Do not include resource env context, generated credentials, private process environment, allocation env values, schema version, or update-check fields.

- [ ] **Step 4: Verify GREEN**

Run:

```bash
cargo insta test --accept --test-runner nextest -- list_json resource_list_json project_env_json
cargo nextest run -E 'test(list_json|resource_list_json|project_env_json)' -p cli --all-features --locked
```

Expected: PASS.

## Task 10: Final Verification, Commit, Push, PR

**Files:**
- All changed files.

- [ ] **Step 1: Run focused command suites**

Run:

```bash
cargo nextest run -E 'test(jobs_)' --workspace --all-features --locked
cargo nextest run -E 'test(logs_)' --workspace --all-features --locked
cargo nextest run -E 'test(status_)' --workspace --all-features --locked
cargo nextest run -E 'test(doctor_)' --workspace --all-features --locked
```

Expected: PASS.

- [ ] **Step 2: Run snapshot hygiene**

Run:

```bash
cargo insta pending-snapshots --workspace
```

Expected: no pending snapshots.

- [ ] **Step 3: Run workspace checks**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
git diff --check
```

Expected: PASS. If a host tool outside Rust is missing, report it explicitly.

- [ ] **Step 4: Audit scope**

Run:

```bash
git diff --name-only origin/main...HEAD
rg -n "update --check|UpdateCheck|schema_version|unwrap\\(|panic!|unreachable!|allow\\(" crates DESIGN.md docs/superpowers/plans/2026-06-10-pr-21-diagnostics-status.md
```

Expected: no `pv update --check` implementation change, no schema envelope, and no new disallowed production shortcuts.

- [ ] **Step 5: Commit, push, and open PR**

Run:

```bash
git status --short
git add DESIGN.md docs/superpowers/plans/2026-06-10-pr-21-diagnostics-status.md Cargo.toml Cargo.lock crates it
git commit -m "feat(cli): add diagnostics status commands"
git push -u origin feat/pr21-diagnostics-status
gh pr create --base main --head feat/pr21-diagnostics-status --title "feat(cli): add diagnostics status commands" --body-file <generated-pr-body>
```

Expected: branch pushed and GitHub PR open for review.

## Scope Guard

- Do not modify `pv update --check` or `pv update --check --json`.
- Do not add new persisted aggregate health subjects.
- Do not expose secrets in `status`, `list`, `jobs`, or resource-list JSON.
- Do not mutate system state from `pv doctor`.
- Do not hand-edit generated snapshots when `cargo insta accept` can promote them.
