# CI Performance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reduce PV pull-request CI from the cited seven-minute run while retaining all meaningful application coverage and making the affected daemon fixtures deterministic.

**Architecture:** Keep one `macos-14` job and its shared Cargo target directory. Repair test-only process fixtures at their source, consolidate recipe CLI smoke coverage into the existing integration test, restore normal nextest scheduling, add the repository's vetted Rust cache, and remove only the two approved redundant workflow gates.

**Tech Stack:** Rust 2024, Tokio, cargo-nextest, GitHub Actions, Python 3 fixture servers, shellcheck, cargo-shear, `insta`.

## Global Constraints

- Execute in `/Users/clovismuneza/Apps/pv/.worktrees/ci-performance` on branch `perf/ci-performance`, created from the approved plan commit.
- Read `CONTRIBUTING.md`, `DESIGN.md`, and `docs/superpowers/specs/2026-07-12-ci-performance-design.md` before changing implementation.
- Keep PV v1 CI on `macos-14`; do not add Linux jobs or split the workflow.
- Do not change production process supervision, the ten-second Managed Resource stop grace period, workflow triggers, Rust toolchain policy, or release workflows.
- Do not remove individual application tests or weaken formatting, Clippy, cargo-shear, recipe shellcheck, or the non-ignored workspace nextest suite.
- Preserve recipe CLI parsing and dispatch coverage before deleting its standalone workflow step.
- Use exactly `Swatinem/rust-cache@e18b497796c12c097a38f9edb9d0641fb99eee32`.
- Do not add dependencies or modify `Cargo.lock`.
- Prefer integration tests and existing helpers; avoid `panic!`, `unreachable!`, `.unwrap()`, unsafe code, and broad clippy ignores.
- Do not add a timing assertion to the committed suite. Measure fixture and suite time operationally so runner variance cannot create a new flaky test.
- Use Conventional Commit messages exactly as listed by each task.

---

## File Structure

- Modify `crates/daemon/tests/supervisor_foundation.rs` to cover ownership verification for a directly executable Python shebang script through the public supervisor API.
- Modify `crates/daemon/src/managed_resources/tests.rs` to make the fast-exit Mailpit fixture track Python directly and to correct Redis/RustFS signal shutdown.
- Modify `crates/pv-release/tests/recipe_fixtures.rs` so the committed-recipe integration test invokes both `pv-release` commands through the compiled binary.
- Modify `.github/workflows/ci.yml` to add the pinned Rust cache and remove the approved standalone recipe and rustdoc steps.
- Delete `.config/nextest.toml` to restore nextest's normal scheduler.
- Do not modify production Rust modules, snapshots, release workflows, or dependency manifests.

---

### Task 1: Deterministic Fast-Exit Fixture And Script Ownership Coverage

**Files:**
- Modify: `crates/daemon/tests/supervisor_foundation.rs`
- Modify: `crates/daemon/src/managed_resources/tests.rs`

**Interfaces:**
- Consumes: `ProcessSupervisor::start(ProcessSpec)`, `ProcessSupervisor::verify_ownership(&ProcessSpec)`, `ManagedProcess::stop(Duration)`, `FakeMailpitRuntimeAdapter::exits_after_readiness()`, and the fake adapter argument order `[smtp_port, dashboard_port]`.
- Produces: integration test `supervisor_verifies_owned_python_shebang_script` and a `fast_exit_fake_mailpit_script()` whose tracked process is the Python process that calls `os._exit(0)`.

- [ ] **Step 1: Record the existing timing-dependent failure evidence**

Run the existing regression repeatedly without editing it:

```shell
for run in {1..20}; do
  cargo nextest run -p daemon --lib --locked \
    -E 'test(demanded_resource_cleans_runtime_files_when_process_exits_after_readiness)' || break
done
```

Expected on current code: the test may fail with `expected fast-exit runtime failure, got Ok(...)`; it may also pass because the shell/Python PID race is timing-dependent. GitHub Actions run `29180535363` is the retained red evidence. Do not introduce sleeps or a timing assertion just to force a local failure.

- [ ] **Step 2: Add an ownership characterization integration test**

Add this test immediately after `supervisor_verifies_and_adopts_owned_runtime_metadata` in `crates/daemon/tests/supervisor_foundation.rs`. Existing imports already provide `Duration`, `Result`, `anyhow`, `tempdir`, `PvPaths`, `sleep`, and `timeout`; existing helpers provide `process_spec` and `set_executable`.

```rust
#[cfg(target_os = "macos")]
#[tokio::test]
async fn supervisor_verifies_owned_python_shebang_script() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    let runtime = paths.run().join("owned-python-runtime");
    state::fs::write_sensitive_file(
        &runtime,
        r#"#!/usr/bin/env python3
import signal
import sys


def stop(_signum, _frame):
    sys.exit(0)


if sys.argv[1:] != ["1025", "8025"]:
    sys.exit(2)

signal.signal(signal.SIGTERM, stop)
signal.pause()
"#,
    )?;
    set_executable(&runtime)?;

    let supervisor = ProcessSupervisor::new(paths.clone());
    let spec = process_spec(
        &paths,
        "owned-python-runtime",
        runtime,
        vec!["1025".to_string(), "8025".to_string()],
    );
    let process = supervisor.start(spec.clone()).await?;
    let pid = process.pid();
    let ownership = timeout(Duration::from_secs(1), async {
        loop {
            if let Some(owned) = supervisor.verify_ownership(&spec)? {
                return Ok::<_, daemon::DaemonError>(owned);
            }

            sleep(Duration::from_millis(10)).await;
        }
    })
    .await;

    process.stop(Duration::from_secs(1)).await?;
    let owned = ownership??;

    assert_eq!(owned.pid(), pid);

    Ok(())
}
```

The ownership result is retained until after `process.stop(...)`, so an assertion or ownership error cannot leak the child process.

- [ ] **Step 3: Run the ownership characterization test**

Run:

```shell
cargo nextest run -p daemon --test supervisor_foundation --locked \
  -E 'test(supervisor_verifies_owned_python_shebang_script)'
```

Expected: PASS. This is characterization coverage for the supervisor's existing interpreter-script command-line support, not a test that should fail before the fixture refactor.

- [ ] **Step 4: Replace the shell/Python fast-exit fixture with direct Python**

Replace `fast_exit_fake_mailpit_script()` in `crates/daemon/src/managed_resources/tests.rs` with:

```rust
fn fast_exit_fake_mailpit_script() -> &'static str {
    r#"#!/usr/bin/env python3
import http.server
import os
import sys


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"ready")
        self.wfile.flush()
        os._exit(0)

    def log_message(self, _format, *_args):
        pass


server = http.server.ThreadingHTTPServer(("127.0.0.1", int(sys.argv[2])), Handler)
server.serve_forever()
"#
}
```

`sys.argv[1]` remains the SMTP port and `sys.argv[2]` remains the dashboard port. Do not use `exec python3 -`; the expected fixture path must remain in the live command line for ownership verification.

- [ ] **Step 5: Run the focused regressions repeatedly**

Run:

```shell
cargo nextest run -p daemon --test supervisor_foundation --locked \
  -E 'test(supervisor_verifies_owned_python_shebang_script)'

for run in {1..20}; do
  cargo nextest run -p daemon --lib --locked \
    -E 'test(demanded_resource_cleans_runtime_files_when_process_exits_after_readiness)'
done
```

Expected: the ownership test passes and all 20 fast-exit runs report the expected runtime failure/cleanup snapshot without returning `Ok(...)`.

- [ ] **Step 6: Run task hygiene checks**

Run:

```shell
cargo fmt --all --check
git diff --check
```

Expected: both commands exit zero.

- [ ] **Step 7: Commit Task 1**

```shell
git add crates/daemon/tests/supervisor_foundation.rs crates/daemon/src/managed_resources/tests.rs
git commit -m "test(daemon): stabilize fast-exit runtime fixture"
```

---

### Task 2: Graceful Redis And RustFS Fixture Shutdown

**Files:**
- Modify: `crates/daemon/src/managed_resources/tests.rs`

**Interfaces:**
- Consumes: the current fake Redis and RustFS protocols, ports, credentials, data directories, and Python `BaseServer.serve_forever()`/`shutdown()` contract.
- Produces: signal handlers that request shutdown from helper threads while Redis and the RustFS API continue running `serve_forever()` on the main Python thread.

- [ ] **Step 1: Capture the focused pre-change duration**

Run:

```shell
/usr/bin/time -p cargo nextest run -p daemon --lib --locked \
  -E 'test(redis_) | test(rustfs_)'
```

Expected on current code: all selected behavior tests pass, but Redis/RustFS lifecycle cases include repeated waits near the production ten-second grace period. Record the nextest summary and `real` duration for comparison; do not add a committed timing assertion.

- [ ] **Step 2: Make Redis request shutdown from another thread**

Within the embedded Python returned by `redis_server_script()`:

1. Add `import threading` with the other top-level Python imports.
2. Replace the current `stop` function with:

```python
shutdown_requested = threading.Event()


def stop(_signum, _frame):
    if shutdown_requested.is_set():
        return
    shutdown_requested.set()
    threading.Thread(target=server.shutdown, daemon=True).start()
```

Keep these lines unchanged after server construction:

```python
server = RedisServer(("127.0.0.1", port), RedisPingHandler)
signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)
server.serve_forever()
```

Python signal handlers run serially on the main thread, so the event prevents redundant helper threads after the first shutdown request. The helper is a daemon thread so a repeated late signal cannot keep the fixture alive after the main server loop exits. Do not move `serve_forever()` off the main thread and do not change protocol behavior.

- [ ] **Step 3: Make RustFS unwind through the main API loop**

Within the embedded Python returned by `rustfs_script_source()`, replace the current `stop` function and the final server-loop block with:

```python
shutdown_requested = threading.Event()


def stop(_signum, _frame):
    if shutdown_requested.is_set():
        return
    shutdown_requested.set()
    threading.Thread(target=api.shutdown, daemon=True).start()


signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)

threading.Thread(target=console.serve_forever, daemon=True).start()
api.serve_forever()
console.shutdown()
```

Do not call `api.shutdown()` or `console.shutdown()` directly from the signal handler. The API loop remains on the main thread so unexpected loop errors terminate the fixture; after it returns, the main thread safely shuts down the console loop running on its existing worker thread.

- [ ] **Step 4: Run and repeat the focused fixture suite**

Run three times:

```shell
for run in {1..3}; do
  /usr/bin/time -p cargo nextest run -p daemon --lib --locked \
    -E 'test(redis_) | test(rustfs_)'
done
```

Expected: every selected test passes on every run, individual fixture tests no longer spend approximately ten seconds in forced shutdown, and the focused summary is materially lower than Step 1. Treat duration as operational evidence, not a pass/fail threshold.

- [ ] **Step 5: Confirm fixture scope and formatting**

Run:

```shell
cargo fmt --all --check
git diff --check
git diff -- crates/daemon/src/managed_resources/tests.rs
```

Expected: only the embedded Redis/RustFS Python shutdown code changed in this task; Rust assertions, snapshots, production Rust modules, and timeout constants are unchanged.

- [ ] **Step 6: Commit Task 2**

```shell
git add crates/daemon/src/managed_resources/tests.rs
git commit -m "fix(ci): avoid resource fixture shutdown deadlocks"
```

---

### Task 3: Consolidate Recipe CLI Coverage Into The Integration Test

**Files:**
- Modify: `crates/pv-release/tests/recipe_fixtures.rs`

**Interfaces:**
- Consumes: binary `env!("CARGO_BIN_EXE_pv-release")`, commands `generate-recipe-fixtures` and `generate-manifest`, the committed recipe/revocation/default files, and existing archive/manifest assertions.
- Produces: automatic parser + dispatch + committed-path + generated-output coverage inside `recipe_fixture_generation_validates_archives_records_and_manifest`.

- [ ] **Step 1: Add binary command imports and helpers**

At the top of `crates/pv-release/tests/recipe_fixtures.rs`, add:

```rust
use std::process::Output;

use anyhow::{Context, Result, bail};
```

Replace the existing `anyhow::{Result, bail}` import rather than duplicating it. Retain the direct `generate_recipe_fixtures_with_backing` and `generate_manifest_file_with_defaults` imports because the two custom-recipe tests still call the library APIs.

Add this alias after imports:

```rust
#[expect(
    clippy::disallowed_types,
    reason = "release tooling CLI tests execute the pv-release binary"
)]
type StdCommand = std::process::Command;
```

Add these helpers immediately before the existing filesystem helpers:

```rust
fn run_pv_release(args: &[&str]) -> Result<Output> {
    StdCommand::new(env!("CARGO_BIN_EXE_pv-release"))
        .args(args)
        .output()
        .context("failed to execute pv-release")
}

fn assert_command_success(output: &Output, label: &str) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }

    bail!(
        "{label} failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
```

- [ ] **Step 2: Invoke `generate-recipe-fixtures` through the binary**

In `recipe_fixture_generation_validates_archives_records_and_manifest`, change the revocation path to the committed directory:

```rust
let revocations = workspace_root.join("release/artifacts/revocations");
```

Remove `create_dir_all(&revocations)?;` from this test only. Replace its direct `generate_recipe_fixtures_with_backing(...)` call with:

```rust
let fixture_output = run_pv_release(&[
    "generate-recipe-fixtures",
    "--php",
    php.as_str(),
    "--composer",
    composer.as_str(),
    "--redis",
    redis.as_str(),
    "--mysql",
    mysql.as_str(),
    "--postgres",
    postgres.as_str(),
    "--mailpit",
    mailpit.as_str(),
    "--rustfs",
    rustfs.as_str(),
    "--archives",
    archives.as_str(),
    "--records",
    records.as_str(),
    "--pv-commit",
    "0123456789abcdef0123456789abcdef01234567",
    "--build-run-id",
    "local-test",
])?;
assert_command_success(&fixture_output, "generate-recipe-fixtures")?;
```

Keep the complete existing `generated_archive_roots` assertion unchanged.

- [ ] **Step 3: Invoke `generate-manifest` through the binary**

In the same test, replace its direct `generate_manifest_file_with_defaults(...)` call with:

```rust
let manifest_output = run_pv_release(&[
    "generate-manifest",
    "--records",
    records.as_str(),
    "--revocations",
    revocations.as_str(),
    "--defaults",
    defaults.as_str(),
    "--output",
    manifest.as_str(),
    "--base-url",
    "https://artifacts.example.test",
])?;
assert_command_success(&manifest_output, "generate-manifest")?;
```

Keep the existing manifest parse and snapshot unchanged:

```rust
let manifest_json = read_to_string(&manifest)?;
ArtifactManifest::parse(&manifest_json)?;
assert_snapshot!(manifest_json);
```

- [ ] **Step 4: Run the recipe integration test**

Run:

```shell
cargo nextest run -p pv-release --test recipe_fixtures --locked
```

Expected: all recipe fixture tests pass; the committed-recipe test invokes both compiled binary commands and its archive roots and manifest snapshot remain unchanged. No new snapshot is expected because the committed revocation directory currently contains only `.gitkeep`.

- [ ] **Step 5: Run task quality checks**

Run:

```shell
cargo fmt --all --check
cargo clippy -p pv-release --all-targets --locked -- -D warnings
git diff --check
```

Expected: all commands exit zero.

- [ ] **Step 6: Commit Task 3**

```shell
git add crates/pv-release/tests/recipe_fixtures.rs
git commit -m "test(release): cover recipe commands through binary"
```

---

### Task 4: Simplify And Cache Pull-Request CI

**Files:**
- Modify: `.github/workflows/ci.yml`
- Delete: `.config/nextest.toml`

**Interfaces:**
- Consumes: the recipe binary coverage committed in Task 3 and the vetted cache action revision from `.github/workflows/real-artifact-e2e.yml`.
- Produces: one cached `macos-14` job with formatting, Clippy, cargo-shear, recipe shellcheck, and the complete normally scheduled workspace test suite.

- [ ] **Step 1: Replace `ci.yml` with the approved gate sequence**

The complete `.github/workflows/ci.yml` after editing must be:

```yaml
name: CI

on:
  push:
    branches: ["main"]
  pull_request:

jobs:
  rust:
    name: Rust
    runs-on: macos-14

    steps:
      - name: Checkout
        uses: actions/checkout@v6

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable

      - name: Cache Rust build
        uses: Swatinem/rust-cache@e18b497796c12c097a38f9edb9d0641fb99eee32

      - name: Install cargo-nextest
        uses: taiki-e/install-action@v2
        with:
          tool: cargo-nextest

      - name: Install cargo-shear
        uses: taiki-e/install-action@v2
        with:
          tool: cargo-shear

      - name: Install shellcheck
        run: brew install shellcheck

      - name: Check formatting
        run: cargo fmt --all --check

      - name: Run Clippy
        run: cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

      - name: Check unused dependencies
        run: cargo shear

      - name: Check artifact recipe scripts
        run: shellcheck release/artifacts/recipes/common.sh release/artifacts/recipes/php/*.sh release/artifacts/recipes/composer/*.sh release/artifacts/recipes/redis/*.sh release/artifacts/recipes/mysql/*.sh release/artifacts/recipes/postgres/*.sh release/artifacts/recipes/mailpit/*.sh release/artifacts/recipes/rustfs/*.sh

      - name: Run tests
        run: cargo nextest run --workspace --all-features --locked
```

The `Build docs` and standalone `Validate artifact recipe metadata and fixtures` steps must be absent. Do not modify any other workflow.

- [ ] **Step 2: Delete blanket nextest serialization**

Delete `.config/nextest.toml` entirely. Do not add another test group or override.

- [ ] **Step 3: Verify the configuration-only diff**

Run:

```shell
test ! -e .config/nextest.toml
git diff --check
git diff -- .github/workflows/ci.yml .config/nextest.toml
```

Expected: the nextest config is absent; the workflow diff adds only the exact pinned cache action and removes only the two approved steps. No dedicated test is added for these configuration-only edits; Task 5 validates their retained commands and normal scheduling.

- [ ] **Step 4: Run one complete suite with normal scheduling**

Run:

```shell
/usr/bin/time -p cargo nextest run --workspace --all-features --locked
```

Expected: all non-ignored tests pass (at least 1,004 after Task 1), no `daemon-lifecycle` group appears, and nextest execution is materially below the cited 141.910 seconds.

- [ ] **Step 5: Commit Task 4**

```shell
git add .github/workflows/ci.yml
git add -u .config/nextest.toml
git commit -m "ci: remove redundant gates and test serialization"
```

---

### Task 5: Full Verification And Performance Evidence

**Files:**
- Verify only; modify files only if a check exposes a defect within the approved scope.

**Interfaces:**
- Consumes: Tasks 1–4.
- Produces: final correctness evidence, three clean normally scheduled suite runs, quality-gate results, and before/after timing suitable for the handoff.

- [ ] **Step 1: Re-run all focused regression coverage**

Run:

```shell
cargo nextest run -p daemon --test supervisor_foundation --locked \
  -E 'test(supervisor_verifies_owned_python_shebang_script)'
cargo nextest run -p daemon --lib --locked \
  -E 'test(demanded_resource_cleans_runtime_files_when_process_exits_after_readiness)'
cargo nextest run -p daemon --lib --locked \
  -E 'test(redis_) | test(rustfs_)'
cargo nextest run -p pv-release --test recipe_fixtures --locked
```

Expected: every focused test passes.

- [ ] **Step 2: Run every retained non-test CI gate**

Run:

```shell
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo shear
shellcheck release/artifacts/recipes/common.sh release/artifacts/recipes/php/*.sh release/artifacts/recipes/composer/*.sh release/artifacts/recipes/redis/*.sh release/artifacts/recipes/mysql/*.sh release/artifacts/recipes/postgres/*.sh release/artifacts/recipes/mailpit/*.sh release/artifacts/recipes/rustfs/*.sh
```

Expected: every command exits zero. Do not run `cargo doc`; removing internal rustdoc from the PR contract is an approved design decision.

- [ ] **Step 3: Run the complete suite three times and retain timings**

Run:

```shell
for run in {1..3}; do
  /usr/bin/time -p cargo nextest run --workspace --all-features --locked
done
```

Expected: all non-ignored tests pass in all three runs. Record each nextest execution summary and wall time. The operational target is approximately 40 seconds of nextest execution on comparable warm local hardware; timing is not a correctness threshold.

- [ ] **Step 4: Inspect the final branch**

Run:

```shell
git diff --check HEAD~4..HEAD
git status --short --branch
git log --oneline --decorate -5
```

Expected: no uncommitted changes, four implementation commits after the plan commit, no lockfile or snapshot churn, and only the files declared by this plan changed.

- [ ] **Step 5: Request final review**

Provide reviewers with:

- the cited CI baseline: 7m04s job, 4m32s test step, 141.910s nextest execution,
- the three post-change nextest execution summaries and wall times,
- confirmation that all focused tests, all three full suites, Clippy, formatting, cargo-shear, and shellcheck passed,
- the intentional coverage decision for internal rustdoc, and
- confirmation that recipe binary dispatch remains automatically covered.

Do not claim a GitHub Actions wall time until the branch has an actual remote run; report the local evidence and the expected cache-hit range separately.
