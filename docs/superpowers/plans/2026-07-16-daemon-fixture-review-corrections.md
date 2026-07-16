# Daemon Fixture Review Corrections Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** Preserve the accepted historical fixture-contract corrections and specify the final timeout-budget, complete-marker, and backpressure-safe output-capture follow-ups.

**Architecture:** Tasks 1–4 are an immutable historical sequence: a private synchronous fixture-command runner owns deadline polling and bounded cleanup, and MySQL/PostgreSQL lifecycle tests coordinate through a handler marker. Tasks 5–7 supersede the accepted final details by adding scheduling slack to the test-only timeout budget, requiring the complete marker contents, and redirecting child output to anonymous temporary files before reading it after confirmed exit.

**Tech Stack:** Rust 2024, anyhow, camino, rustix, Python 3 standard library, cargo-nextest, Clippy.

## Global Constraints

- Work only in the existing refactor/daemon-test-fixtures worktree and preserve unrelated user changes.
- Do not change production daemon or supervisor behavior, process ownership matching, nextest configuration, CI scheduling, fixture process-group topology, Cargo.toml, Cargo.lock, dependencies, manifests, or Python fixture producers.
- Python fixtures remain standard-library-only. Preserve normal exit status, stdout, stderr, custom MySQL environment variables, CLI behavior, and protocol behavior.
- Tasks 1–4 are historical implementations, including their commits and code blocks. Do not amend them beyond correcting the PostgreSQL marker prose: the marker is written at handler entry before the first blocking `read_startup`, and `statements`/`portals` initialize after the marker.
- Tasks 5–7 supersede the accepted final details. Final behavior uses a 3-second command deadline, 10-millisecond poll interval, 1-second bounded cleanup deadline, and a 100-millisecond test-only scheduling margin. Every post-spawn timeout or `try_wait` error routes through bounded cleanup.
- Final lifecycle tests use a per-test `PV_FIXTURE_HANDLER_STARTED` marker rather than a fixed delay. MySQL and PostgreSQL Python producers remain unchanged; Rust accepts only the exact UTF-8 contents `"started\n"` before signalling the fixture.
- The final runner captures stdout and stderr in two anonymous `camino_tempfile::tempfile` files, reads each only after confirmed exit, and preserves `String::from_utf8` conversion and `ExitStatus::code`. Do not use threads, pipes, new dependencies, manifest/lockfile changes, unsafe code, `panic!`, `unwrap()`, or Clippy ignores.
- Set daemon_threads = True on custom ThreadingMixIn servers for MySQL, PostgreSQL, fake Mailpit, and Mailpit. Redis retains its existing setting. Do not add Mailpit implementation-detail lifecycle tests because its handlers already return promptly.
- Preserve the positive HANGING_FIXTURE extraction-plan correction in docs/superpowers/plans/2026-07-15-daemon-test-fixtures.md; this plan does not alter it.
- The four historical commit boundaries map one-to-one to Tasks 1–4. Their blocking reap and fixed 100-millisecond settle sleep are historical interim behavior. Tasks 5–7 have three additional, separate commit boundaries and define the final accepted details.

## File Map

- Modify: crates/daemon/tests/fixture_contracts.rs
  - Final state: bounded command cleanup with a scheduling-margin test budget, complete-marker synchronization, backpressure-safe temporary-file output capture, and MySQL/PostgreSQL idle-client shutdown coverage.
- Create: crates/daemon/tests/snapshots/fixture_contracts__fixture_handler_marker_requires_complete_contents.snap
  - Captures the complete-marker regression's timeout result.
- Create: crates/daemon/tests/snapshots/fixture_contracts__fixture_command_captures_verbose_output_without_pipe_backpressure.snap
  - Captures the successful verbose fixture exit code and exact stdout/stderr lengths.
- Modify: crates/daemon/test-fixtures/managed-resources/mysql.py
  - Final state: daemon request threads and optional handler marker before recv.
- Modify: crates/daemon/test-fixtures/managed-resources/postgres.py
  - Historical state: daemon request threads and an optional marker at handler entry, before `statements`/`portals` initialization and the first blocking `read_startup`.
- Modify: crates/daemon/test-fixtures/managed-resources/fake-mailpit.py
  - Final state: daemon SMTP request threads.
- Modify: crates/daemon/test-fixtures/managed-resources/mailpit.py
  - Final state: daemon SMTP request threads.

### Task 1: Bound Fixture-Contract Subprocesses

**Historical commit:** b07cb793882ba59224b6a59ce9492013b1930e11 — first implementation commit.

**Files:**
- Modify: crates/daemon/tests/fixture_contracts.rs

**Interfaces:**
- Produces FIXTURE_COMMAND_TIMEOUT, FIXTURE_COMMAND_POLL_INTERVAL, run_fixture_command, fixture_output, and process_pid.
- This task deliberately has no lifecycle cleanup helper. Its timeout cleanup is an interim blocking implementation, superseded by Task 3 before final verification.

- [ ] **Step 1: Add the hanging-child regression and runner**

Add the Error, Output, Stdio, anyhow, Pid, and test_kill_process imports; add the 3-second command timeout and 10-millisecond poll interval; then add the scenario-local HANGING_FIXTURE and the PID-reaping regression. Keep the accepted extraction-plan fixture text unchanged in the separate extraction plan.

~~~rust
const HANGING_FIXTURE: &str = r#"#!/usr/bin/env python3
import os
import signal
import sys


with open(sys.argv[1], "w", encoding="utf-8") as pid_file:
    pid_file.write(f"{os.getpid()}\n")

signal.pause()
"#;

#[test]
fn fixture_command_timeout_kills_and_reaps_child() -> Result<()> {
    let tempdir = tempdir()?;
    let fixture = tempdir.path().join("hanging-fixture");
    let pid_path = tempdir.path().join("hanging-fixture.pid");

    materialize_fixture(&fixture, HANGING_FIXTURE)?;
    let mut command = FixtureCommand::new(fixture.as_std_path());
    command
        .arg(pid_path.as_std_path())
        .current_dir(tempdir.path());

    let error = match run_fixture_command(&mut command, Duration::from_secs(1)) {
        Ok(output) => bail!("hanging fixture unexpectedly exited: {output:?}"),
        Err(error) => error,
    };
    let io_error = error
        .downcast_ref::<std::io::Error>()
        .ok_or_else(|| anyhow!("fixture timeout did not return an I/O error: {error}"))?;
    assert_eq!(io_error.kind(), ErrorKind::TimedOut);

    let raw_pid = state::fs::read_to_string(&pid_path)?
        .trim()
        .parse::<u32>()?;
    let pid = process_pid(raw_pid)?;
    assert!(test_kill_process(pid).is_err());

    Ok(())
}
~~~

- [ ] **Step 2: Implement the b07-era runner**

Use this historical runner exactly. It intentionally performs a blocking wait_with_output after timeout; Task 3 replaces that behavior.

~~~rust
fn run_fixture_command(command: &mut FixtureCommand, timeout: Duration) -> Result<FixtureOutput> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let deadline = Instant::now() + timeout;

    loop {
        if child.try_wait()?.is_some() {
            return fixture_output(child.wait_with_output()?);
        }
        if Instant::now() >= deadline {
            let kill_result = child.kill();
            let wait_result = child.wait_with_output();
            if let Err(error) = wait_result {
                return Err(error.into());
            }
            if let Err(error) = kill_result
                && error.kind() != ErrorKind::InvalidInput
            {
                return Err(error.into());
            }

            return Err(Error::new(
                ErrorKind::TimedOut,
                format!("fixture command timed out after {} ms", timeout.as_millis()),
            )
            .into());
        }

        thread::sleep(FIXTURE_COMMAND_POLL_INTERVAL);
    }
}
~~~

Add these b07-era private helpers immediately after the runner. The runner uses `fixture_output` for confirmed-exit capture, and the regression uses `process_pid` to convert the recorded child PID for `test_kill_process`.

~~~rust
fn fixture_output(output: Output) -> Result<FixtureOutput> {
    Ok(FixtureOutput {
        code: output.status.code(),
        stdout: String::from_utf8(output.stdout)?,
        stderr: String::from_utf8(output.stderr)?,
    })
}

fn process_pid(pid: u32) -> Result<Pid> {
    let raw_pid = i32::try_from(pid)?;
    Pid::from_raw(raw_pid).ok_or_else(|| anyhow!("invalid process id {raw_pid}"))
}
~~~

Route run_fixture and the direct MySQL empty-data-directory command through that runner.

- [ ] **Step 3: Verify and self-review the historical first commit**

Run:

~~~shell
cargo nextest run -p daemon --test fixture_contracts -E 'test(fixture_command_timeout_kills_and_reaps_child)'
cargo nextest run -p daemon --test fixture_contracts
cargo fmt --all --check
cargo clippy -p daemon --test fixture_contracts --locked -- -D warnings
git diff --check
~~~

Confirm that both former output() callers use run_fixture_command, the child PID is gone after the timeout regression, and no dependency, lockfile, snapshot, or unrelated change exists. This interim self-review must not claim blocking cleanup is final; Task 3 hardens it before final verification.

Commit:

~~~shell
git add crates/daemon/tests/fixture_contracts.rs
git commit -m "fix(daemon): bound fixture contract subprocesses"
~~~

### Task 2: Daemonize Custom Fixture Request Handlers

**Historical commit:** 89affe2eb9ac6fb01dc932a2ec0a69d53d78e125 — second implementation commit.

**Files:**
- Modify: crates/daemon/tests/fixture_contracts.rs
- Modify: crates/daemon/test-fixtures/managed-resources/mysql.py
- Modify: crates/daemon/test-fixtures/managed-resources/postgres.py
- Modify: crates/daemon/test-fixtures/managed-resources/fake-mailpit.py
- Modify: crates/daemon/test-fixtures/managed-resources/mailpit.py

**Interfaces:**
- Consumes Task 1's runner and process_pid.
- Produces connect_to_loopback, wait_for_child_exit, the initial kill_and_reap_child, idle-client lifecycle tests, and daemon request threads.
- The 100-millisecond settle delay and this helper's blocking wait are interim. Tasks 3 and 4 supersede them before final verification.

- [ ] **Step 1: Add the historical lifecycle constants, RED tests, and helpers**

Add:

~~~rust
const FIXTURE_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
const FIXTURE_HANDLER_SETTLE_TIME: Duration = Duration::from_millis(100);
~~~

Add MySQL and PostgreSQL tests that start the fixture, connect an idle client, retain it in scope, sleep for FIXTURE_HANDLER_SETTLE_TIME, send SIGTERM, and require exit within FIXTURE_SHUTDOWN_TIMEOUT. Use this historical cleanup shape:

~~~rust
let lifecycle = (|| {
    let _idle_client = connect_to_loopback(port, FIXTURE_COMMAND_TIMEOUT)?;
    thread::sleep(FIXTURE_HANDLER_SETTLE_TIME);
    kill_process(process_pid(child.id())?, Signal::TERM)?;
    if !wait_for_child_exit(&mut child, FIXTURE_SHUTDOWN_TIMEOUT)? {
        bail!("fixture did not exit after SIGTERM with an idle client");
    }

    Ok::<(), anyhow::Error>(())
})();
let cleanup = kill_and_reap_child(&mut child);

if let Err(error) = lifecycle {
    cleanup?;
    return Err(error);
}
cleanup
~~~

Add the initial helpers. Its blocking wait is retained only for historical accuracy and is replaced in Task 3:

~~~rust
fn connect_to_loopback(port: u16, timeout: Duration) -> Result<TcpStream> {
    let deadline = Instant::now() + timeout;

    loop {
        if let Ok(stream) = TcpStream::connect((Ipv4Addr::LOCALHOST, port)) {
            return Ok(stream);
        }
        if Instant::now() >= deadline {
            bail!("timed out connecting to fixture port {port}");
        }

        thread::sleep(FIXTURE_COMMAND_POLL_INTERVAL);
    }
}

fn wait_for_child_exit(child: &mut Child, timeout: Duration) -> Result<bool> {
    let deadline = Instant::now() + timeout;

    loop {
        if child.try_wait()?.is_some() {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }

        thread::sleep(FIXTURE_COMMAND_POLL_INTERVAL);
    }
}

fn kill_and_reap_child(child: &mut Child) -> Result<()> {
    let kill_result = child.kill();
    let wait_result = child.wait();
    if let Err(error) = wait_result {
        return Err(error.into());
    }
    if let Err(error) = kill_result
        && error.kind() != ErrorKind::InvalidInput
    {
        return Err(error.into());
    }

    Ok(())
}
~~~

Extend the Task 1 imports with:

~~~rust
use std::process::{Child, Output, Stdio};
use rustix::process::{Pid, Signal, kill_process, test_kill_process};
~~~

Run the two lifecycle regressions. RED is the explicit did-not-exit failure after the bounded lifecycle wait; cleanup must still kill and reap both fixtures.

- [ ] **Step 2: Apply the minimal threaded-server GREEN change**

Add daemon_threads = True to each custom ThreadingMixIn class:

~~~python
class TcpServer(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True
    daemon_threads = True
~~~

Use the same extra line in MySQL, fake Mailpit, and Mailpit; use it in PostgreSQL's Server class. Do not change Redis or any ThreadingHTTPServer.

- [ ] **Step 3: Verify and self-review the historical second commit**

Run:

~~~shell
cargo nextest run -p daemon --test fixture_contracts -E 'test(mysql_fixture_exits_after_sigterm_with_idle_client) | test(postgres_fixture_exits_after_sigterm_with_idle_client)'
python3 -c 'import ast, pathlib; paths = [pathlib.Path(path) for path in ["crates/daemon/test-fixtures/managed-resources/mysql.py", "crates/daemon/test-fixtures/managed-resources/postgres.py", "crates/daemon/test-fixtures/managed-resources/fake-mailpit.py", "crates/daemon/test-fixtures/managed-resources/mailpit.py"]]; [ast.parse(path.read_text(), filename=str(path)) for path in paths]; print(f"parsed {len(paths)} Python fixtures")'
cargo nextest run -p daemon --test fixture_contracts
cargo fmt --all --check
cargo clippy -p daemon --test fixture_contracts --locked -- -D warnings
git diff --check
~~~

Confirm the RED-to-GREEN transition is caused only by daemon request threads and the idle client remains open through the exit assertion. Record that the settle constant and blocking helper are interim and must be replaced by Tasks 3–4 before final verification.

Commit:

~~~shell
git add crates/daemon/tests/fixture_contracts.rs
git add crates/daemon/test-fixtures/managed-resources/mysql.py
git add crates/daemon/test-fixtures/managed-resources/postgres.py
git add crates/daemon/test-fixtures/managed-resources/fake-mailpit.py
git add crates/daemon/test-fixtures/managed-resources/mailpit.py
git commit -m "fix(daemon): daemonize fixture request handlers"
~~~

### Task 3: Harden Fixture Child Cleanup

**Historical commit:** 7efda23ec96991d18505d5855fb602d9e27c8275 — supersedes Tasks 1–2's blocking cleanup before final verification.

**Files:**
- Modify: crates/daemon/tests/fixture_contracts.rs

**Interfaces:**
- Keeps wait_for_child_exit and replaces the initial cleanup helper with bounded cleanup.
- Strengthens the timeout regression's elapsed-time bound and makes all post-spawn timeout and try_wait errors use cleanup.

- [ ] **Step 1: Strengthen the timeout regression**

Use a named one-second timeout and require the timeout-plus-cleanup path to finish within the command, cleanup, and poll budgets:

~~~rust
let timeout = Duration::from_secs(1);
let started_at = Instant::now();
let error = match run_fixture_command(&mut command, timeout) {
    Ok(output) => bail!("hanging fixture unexpectedly exited: {output:?}"),
    Err(error) => error,
};
// ... assert ErrorKind::TimedOut ...
assert!(
    started_at.elapsed() < timeout + FIXTURE_SHUTDOWN_TIMEOUT + FIXTURE_COMMAND_POLL_INTERVAL,
    "fixture command timeout exceeded its cleanup deadline"
);
~~~

- [ ] **Step 2: Funnel every post-spawn failure through bounded cleanup**

Replace the runner with the current behavior. The confirmed-exit wait_with_output path remains unchanged; only timeout and try_wait failures fall through to cleanup.

~~~rust
fn run_fixture_command(command: &mut FixtureCommand, timeout: Duration) -> Result<FixtureOutput> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let deadline = Instant::now() + timeout;

    let error = loop {
        match child.try_wait() {
            Ok(Some(_)) => return fixture_output(child.wait_with_output()?),
            Ok(None) => {}
            Err(error) => break error.into(),
        }
        if Instant::now() >= deadline {
            break Error::new(
                ErrorKind::TimedOut,
                format!("fixture command timed out after {} ms", timeout.as_millis()),
            )
            .into();
        }

        thread::sleep(FIXTURE_COMMAND_POLL_INTERVAL);
    };

    kill_and_reap_child(&mut child)?;
    Err(error)
}
~~~

- [ ] **Step 3: Make fake Mailpit cleanup use the same helper**

After its readiness lifecycle closure, clean up on success and failure with the Task 3 helper:

~~~rust
let cleanup = kill_and_reap_child(&mut child);

let lifecycle = match lifecycle {
    Ok(lifecycle) => lifecycle,
    Err(error) => {
        cleanup?;
        return Err(error);
    }
};
cleanup?;
~~~

Keep the snapshot assertion after lifecycle and cleanup both succeed.

- [ ] **Step 4: Replace the blocking reaper with the bounded final helper**

Capture a genuine kill error, always attempt bounded wait_for_child_exit, tolerate InvalidInput only as the already-exited race, never block after an unconfirmed exit, and return the genuine kill error before the typed cleanup timeout:

~~~rust
fn kill_and_reap_child(child: &mut Child) -> Result<()> {
    let kill_error = match child.kill() {
        Ok(()) => None,
        Err(error) if error.kind() == ErrorKind::InvalidInput => None,
        Err(error) => Some(error),
    };
    let reap_result = wait_for_child_exit(child, FIXTURE_SHUTDOWN_TIMEOUT);

    if let Some(error) = kill_error {
        return Err(error.into());
    }
    if !reap_result? {
        return Err(Error::new(
            ErrorKind::TimedOut,
            format!(
                "timed out reaping fixture child after {} ms",
                FIXTURE_SHUTDOWN_TIMEOUT.as_millis()
            ),
        )
        .into());
    }

    Ok(())
}
~~~

- [ ] **Step 5: Verify focused cleanup behavior and commit**

Run:

~~~shell
cargo nextest run -p daemon --test fixture_contracts -E 'test(fixture_command_timeout_kills_and_reaps_child) | test(fake_mailpit_fixture_cli_ignores_extra_arguments)'
cargo nextest run -p daemon --test fixture_contracts
cargo fmt --all --check
cargo clippy -p daemon --test fixture_contracts --locked -- -D warnings
git diff --check
~~~

Confirm the elapsed bound, post-spawn cleanup funnel, unmodified confirmed-exit output path, fake-Mailpit success/failure cleanup, and bounded error precedence. Then commit:

~~~shell
git add crates/daemon/tests/fixture_contracts.rs
git commit -m "fix(daemon): bound fixture child cleanup"
~~~

### Task 4: Synchronize Fixture Handler Entry

**Historical commit:** fafd3c4f470ed2ed328801ce43f7b0cf778e8931 — supersedes Task 2's settle delay before final verification.

**Files:**
- Modify: crates/daemon/tests/fixture_contracts.rs
- Modify: crates/daemon/test-fixtures/managed-resources/mysql.py
- Modify: crates/daemon/test-fixtures/managed-resources/postgres.py

**Interfaces:**
- Produces wait_for_path(path: &Utf8Path, timeout: Duration) -> Result<()>.
- Rust tests opt in through PV_FIXTURE_HANDLER_STARTED; Python handlers write and close the UTF-8 marker before blocking reads.

- [ ] **Step 1: RED — add only Rust marker synchronization**

Remove FIXTURE_HANDLER_SETTLE_TIME and both settle sleeps. Add isolated marker paths, the environment variable, and bounded marker waits to both lifecycle tests:

~~~rust
let handler_marker = tempdir.path().join("mysql-handler-started");
// ... spawn ...
.env("PV_FIXTURE_HANDLER_STARTED", handler_marker.as_std_path())
// ... after connect ...
wait_for_path(&handler_marker, FIXTURE_COMMAND_TIMEOUT)?;
~~~

Use the analogous postgres-handler-started path for PostgreSQL. Add the helper near the other bounded waits:

~~~rust
fn wait_for_path(path: &Utf8Path, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;

    loop {
        if path_exists(path)? {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("timed out waiting for fixture handler marker at {path}");
        }

        thread::sleep(FIXTURE_COMMAND_POLL_INTERVAL);
    }
}
~~~

Do not change either Python fixture in this RED step. Both lifecycle tests must fail by the bounded marker timeout and still use Task 3's bounded cleanup.

- [ ] **Step 2: GREEN — write and close the optional marker before blocking reads**

At the beginning of each `Handler.handle`, add this exact env-gated UTF-8 write-and-close block. In MySQL it is immediately before `self.request.recv(1024)`. In PostgreSQL it is at handler entry; `statements` and `portals` initialize after the marker, and the first blocking `read_startup(self.request)` follows those initializations.

~~~python
handler_marker = os.environ.get("PV_FIXTURE_HANDLER_STARTED")
if handler_marker:
    with open(handler_marker, "w", encoding="utf-8") as marker:
        marker.write("started\n")
~~~

The Task 4 MySQL shape is:

~~~python
class Handler(socketserver.BaseRequestHandler):
    def handle(self):
        handler_marker = os.environ.get("PV_FIXTURE_HANDLER_STARTED")
        if handler_marker:
            with open(handler_marker, "w", encoding="utf-8") as marker:
                marker.write("started\n")
        self.request.recv(1024)
~~~

The Task 4 PostgreSQL shape places the completed marker write at handler entry, before `statements`/`portals` initialization and its first blocking read:

~~~python
class Handler(socketserver.BaseRequestHandler):
    def handle(self):
        handler_marker = os.environ.get("PV_FIXTURE_HANDLER_STARTED")
        if handler_marker:
            with open(handler_marker, "w", encoding="utf-8") as marker:
                marker.write("started\n")
        statements = {}
        portals = {}
        try:
            read_startup(self.request)
~~~

- [ ] **Step 3: Verify GREEN and the historical marker synchronization**

Run the lifecycle tests, full fixture-contract suite, Python parsing, formatting, Clippy, diff checks, and 20 repetitions of each lifecycle regression:

~~~shell
cargo nextest run -p daemon --test fixture_contracts -E 'test(mysql_fixture_exits_after_sigterm_with_idle_client) | test(postgres_fixture_exits_after_sigterm_with_idle_client)'
cargo nextest run -p daemon --test fixture_contracts
python3 -c 'import ast, pathlib; paths = [pathlib.Path(path) for path in ["crates/daemon/test-fixtures/managed-resources/mysql.py", "crates/daemon/test-fixtures/managed-resources/postgres.py"]]; [ast.parse(path.read_text(), filename=str(path)) for path in paths]; print(f"parsed {len(paths)} Python fixtures")'
cargo fmt --all --check
cargo clippy -p daemon --test fixture_contracts --locked -- -D warnings
git diff --check
for iteration in {1..20}; do
  cargo nextest run -p daemon --test fixture_contracts -E 'test(mysql_fixture_exits_after_sigterm_with_idle_client)'
done
for iteration in {1..20}; do
  cargo nextest run -p daemon --test fixture_contracts -E 'test(postgres_fixture_exits_after_sigterm_with_idle_client)'
done
~~~

Confirm the RED failures were marker-timeout failures with cleanup, GREEN waits for an actually-entered handler, no fixed settle sleep remains, and marker writes occur before both blocking reads.

Commit:

~~~shell
git add crates/daemon/tests/fixture_contracts.rs
git add crates/daemon/test-fixtures/managed-resources/mysql.py
git add crates/daemon/test-fixtures/managed-resources/postgres.py
git commit -m "test(daemon): synchronize fixture handler shutdown"
~~~

### Task 5: Correct the Timeout Timing Budget

**Files:**
- Modify: crates/daemon/tests/fixture_contracts.rs

**Interfaces:**
- Consumes the historical `FIXTURE_COMMAND_TIMEOUT`, `FIXTURE_SHUTDOWN_TIMEOUT`, and `FIXTURE_COMMAND_POLL_INTERVAL` constants and the timeout regression.
- Produces `FIXTURE_COMMAND_TIMEOUT_SCHEDULING_MARGIN: Duration = Duration::from_millis(100)` and the final test-only elapsed-time budget.

- [ ] **Step 1: Add the scheduling margin and make the regression reliably GREEN**

Add the named test-budget-only constant alongside the timeout and polling constants:

~~~rust
const FIXTURE_COMMAND_TIMEOUT_SCHEDULING_MARGIN: Duration = Duration::from_millis(100);
~~~

In `fixture_command_timeout_kills_and_reaps_child`, replace the historical elapsed-time assertion with the final complete budget. The old one-poll budget is intentionally too tight when scheduling delays occur; do not change runtime deadlines, poll behavior, or cleanup behavior.

~~~rust
assert!(
    started_at.elapsed()
        < timeout
            + FIXTURE_SHUTDOWN_TIMEOUT
            + FIXTURE_COMMAND_POLL_INTERVAL
            + FIXTURE_COMMAND_POLL_INTERVAL
            + FIXTURE_COMMAND_TIMEOUT_SCHEDULING_MARGIN,
    "fixture command timeout exceeded its cleanup deadline"
);
~~~

Run:

~~~shell
cargo nextest run -p daemon --test fixture_contracts -E 'test(fixture_command_timeout_kills_and_reaps_child)'
~~~

Expected: PASS. This test-only budget makes no production or fixture-runner timing change.

- [ ] **Step 2: Verify scope and commit**

Run:

~~~shell
cargo fmt --all --check
cargo clippy -p daemon --test fixture_contracts --locked -- -D warnings
git diff --check
~~~

Confirm only `crates/daemon/tests/fixture_contracts.rs` changed and that the runtime timeout, shutdown timeout, and poll interval values are unchanged. Commit:

~~~shell
git add crates/daemon/tests/fixture_contracts.rs
git commit -m "test(daemon): allow fixture timeout scheduling margin"
~~~

### Task 6: Require Complete Handler Markers

**Files:**
- Modify: crates/daemon/tests/fixture_contracts.rs
- Create: crates/daemon/tests/snapshots/fixture_contracts__fixture_handler_marker_requires_complete_contents.snap

**Interfaces:**
- Consumes the historical marker paths and `PV_FIXTURE_HANDLER_STARTED` producer contract.
- Replaces `wait_for_path(path: &Utf8Path, timeout: Duration) -> Result<()>` with `wait_for_handler_marker(path: &Utf8Path, timeout: Duration) -> Result<()>`.
- Produces `FIXTURE_HANDLER_MARKER_CONTENTS: &str = "started\n"`; the existing Python producers remain unchanged.

- [ ] **Step 1: Add the incomplete-marker RED regression and snapshot target**

Add the exact expected contents beside the timing constants:

~~~rust
const FIXTURE_HANDLER_MARKER_CONTENTS: &str = "started\n";
~~~

Add this regression beside the lifecycle tests. It creates an already-existing but incomplete marker and uses the existing `wait_for_path` helper with a zero timeout, so it has no sleep-based coordination. Under the old existence-only helper, the `Ok(())` branch makes the test fail before the snapshot is reached.

~~~rust
#[test]
fn fixture_handler_marker_requires_complete_contents() -> Result<()> {
    let tempdir = tempdir()?;
    let handler_marker = tempdir.path().join("handler-started");

    state::fs::write_sensitive_file(&handler_marker, "started")?;

    let error = match wait_for_path(&handler_marker, Duration::ZERO) {
        Ok(()) => bail!("incomplete fixture handler marker unexpectedly satisfied waiter"),
        Err(error) => error,
    };

    assert_fixture_snapshot(
        tempdir.path(),
        "fixture_handler_marker_requires_complete_contents",
        error.to_string(),
    )
}
~~~

Run:

~~~shell
cargo nextest run -p daemon --test fixture_contracts -E 'test(fixture_handler_marker_requires_complete_contents)'
~~~

Expected: FAIL with `incomplete fixture handler marker unexpectedly satisfied waiter`; no timing delay is involved.

- [ ] **Step 2: Replace the existence-only waiter with exact-content polling**

Replace `wait_for_path` with `wait_for_handler_marker`, update both lifecycle tests to call the new name after connecting the idle client, and update the new regression call from `wait_for_path(&handler_marker, Duration::ZERO)` to `wait_for_handler_marker(&handler_marker, Duration::ZERO)`. Add `use state::StateError;` with the top-level imports. Poll `state::fs::read_to_string`: return only for the exact marker contents, retry `NotFound` and incomplete contents, and immediately propagate every other state/filesystem error. Keep the historical timeout message shape and polling interval.

~~~rust
fn wait_for_handler_marker(path: &Utf8Path, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;

    loop {
        match state::fs::read_to_string(path) {
            Ok(contents) if contents == FIXTURE_HANDLER_MARKER_CONTENTS => return Ok(()),
            Ok(_) => {}
            Err(StateError::Filesystem { source, .. }) if source.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if Instant::now() >= deadline {
            bail!("timed out waiting for fixture handler marker at {path}");
        }

        thread::sleep(FIXTURE_COMMAND_POLL_INTERVAL);
    }
}
~~~

Use the exact final calls in both lifecycle closures:

~~~rust
wait_for_handler_marker(&handler_marker, FIXTURE_COMMAND_TIMEOUT)?;
~~~

Do not edit `mysql.py` or `postgres.py`: their existing write-and-close `"started\n"` producers already satisfy this contract.

- [ ] **Step 3: Verify GREEN, accept the snapshot, and commit**

Run:

~~~shell
cargo insta test --accept --test-runner nextest -- fixture_handler_marker_requires_complete_contents
cargo nextest run -p daemon --test fixture_contracts -E 'test(fixture_handler_marker_requires_complete_contents) | test(mysql_fixture_exits_after_sigterm_with_idle_client) | test(postgres_fixture_exits_after_sigterm_with_idle_client)'
cargo fmt --all --check
cargo clippy -p daemon --test fixture_contracts --locked -- -D warnings
git diff --check
~~~

Verify the created snapshot is exactly:

~~~text
---
source: crates/daemon/tests/fixture_contracts.rs
expression: snapshot
---
"timed out waiting for fixture handler marker at <tempdir>/handler-started"
~~~

Confirm incomplete existing contents time out, a missing marker still retries until its deadline, other read errors propagate, and both lifecycle tests require complete contents. Commit:

~~~shell
git add crates/daemon/tests/fixture_contracts.rs
git add crates/daemon/tests/snapshots/fixture_contracts__fixture_handler_marker_requires_complete_contents.snap
git commit -m "test(daemon): require complete fixture handler markers"
~~~

### Task 7: Capture Fixture Output Without Pipe Backpressure

**Files:**
- Modify: crates/daemon/tests/fixture_contracts.rs
- Create: crates/daemon/tests/snapshots/fixture_contracts__fixture_command_captures_verbose_output_without_pipe_backpressure.snap

**Interfaces:**
- Consumes `run_fixture_command`, bounded `kill_and_reap_child`, `FixtureOutput`, and `camino_tempfile::tempfile` already available to the daemon test crate.
- Replaces `std::process::Output` with `std::process::ExitStatus` in `fixture_output` and introduces `read_fixture_output(file: &mut (impl Read + Seek)) -> Result<Vec<u8>>`.
- Produces a backpressure-safe runner that returns the child `ExitStatus::code` and UTF-8 stdout/stderr after confirmed exit.

- [ ] **Step 1: Add the 2 MiB-per-stream RED regression**

Add this standard-library-only scenario-local fixture beside `HANGING_FIXTURE`. It writes and flushes deterministic UTF-8 output totaling 2 MiB to each stream, then exits successfully.

~~~rust
const VERBOSE_FIXTURE: &str = r#"#!/usr/bin/env python3
import sys


contents = "v" * (2 * 1024 * 1024)
sys.stdout.write(contents)
sys.stdout.flush()
sys.stderr.write(contents)
sys.stderr.flush()
"#;
~~~

Add this test beside the timeout regression. It snapshots the successful exit code and both byte lengths, and explicitly checks the exact `2 * 1024 * 1024` expected length for each UTF-8 string.

~~~rust
#[test]
fn fixture_command_captures_verbose_output_without_pipe_backpressure() -> Result<()> {
    let tempdir = tempdir()?;
    let fixture = tempdir.path().join("verbose-fixture");

    materialize_fixture(&fixture, VERBOSE_FIXTURE)?;
    let mut command = FixtureCommand::new(fixture.as_std_path());
    command.current_dir(tempdir.path());

    let output = run_fixture_command(&mut command, FIXTURE_COMMAND_TIMEOUT)?;
    assert_eq!(output.stdout.len(), 2 * 1024 * 1024);
    assert_eq!(output.stderr.len(), 2 * 1024 * 1024);
    assert_fixture_snapshot(
        tempdir.path(),
        "fixture_command_captures_verbose_output_without_pipe_backpressure",
        (output.code, output.stdout.len(), output.stderr.len()),
    )
}
~~~

Run:

~~~shell
cargo nextest run -p daemon --test fixture_contracts -E 'test(fixture_command_captures_verbose_output_without_pipe_backpressure)'
~~~

Expected: FAIL with the existing piped implementation timing out. The 2 MiB written and flushed to each stream exceeds dynamically grown pipe capacity across supported Unix environments while remaining small for anonymous temporary files.

- [ ] **Step 2: Redirect child output to anonymous files and read it after confirmed exit**

Replace the `Output` import with `ExitStatus`; add `Read` and `Seek` imports; and import `camino_tempfile::tempfile` alongside `tempdir`:

~~~rust
use std::io::{Error, ErrorKind, Read, Seek};
use std::process::{Child, ExitStatus, Stdio};

use camino_tempfile::{tempdir, tempfile};
~~~

Use two anonymous files for child stdout and stderr. Clone each file only for its child `Stdio`; retain the parent handles. Preserve the existing `try_wait`, deadline, and `kill_and_reap_child` error flow. On confirmed exit, pass the `ExitStatus` and retained files to the output helper. Do not add threads, pipes, dependencies, manifests, lockfile changes, unsafe code, `panic!`, `unwrap()`, or Clippy ignores.

~~~rust
fn run_fixture_command(command: &mut FixtureCommand, timeout: Duration) -> Result<FixtureOutput> {
    let mut stdout = tempfile()?;
    let mut stderr = tempfile()?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout.try_clone()?))
        .stderr(Stdio::from(stderr.try_clone()?));
    let mut child = command.spawn()?;
    let deadline = Instant::now() + timeout;

    let error = loop {
        match child.try_wait() {
            Ok(Some(status)) => return fixture_output(status, &mut stdout, &mut stderr),
            Ok(None) => {}
            Err(error) => break error.into(),
        }
        if Instant::now() >= deadline {
            break Error::new(
                ErrorKind::TimedOut,
                format!("fixture command timed out after {} ms", timeout.as_millis()),
            )
            .into();
        }

        thread::sleep(FIXTURE_COMMAND_POLL_INTERVAL);
    };

    kill_and_reap_child(&mut child)?;
    Err(error)
}

fn fixture_output(
    status: ExitStatus,
    stdout: &mut (impl Read + Seek),
    stderr: &mut (impl Read + Seek),
) -> Result<FixtureOutput> {
    Ok(FixtureOutput {
        code: status.code(),
        stdout: String::from_utf8(read_fixture_output(stdout)?)?,
        stderr: String::from_utf8(read_fixture_output(stderr)?)?,
    })
}

fn read_fixture_output(file: &mut (impl Read + Seek)) -> Result<Vec<u8>> {
    file.rewind()?;
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)?;

    Ok(contents)
}
~~~

- [ ] **Step 3: Verify GREEN, accept the snapshot, and commit**

Run:

~~~shell
cargo insta test --accept --test-runner nextest -- fixture_command_captures_verbose_output_without_pipe_backpressure
cargo nextest run -p daemon --test fixture_contracts -E 'test(fixture_command_captures_verbose_output_without_pipe_backpressure) | test(fixture_command_timeout_kills_and_reaps_child)'
cargo fmt --all --check
cargo clippy -p daemon --test fixture_contracts --locked -- -D warnings
git diff --check
~~~

Verify the created snapshot is exactly:

~~~text
---
source: crates/daemon/tests/fixture_contracts.rs
expression: snapshot
---
(
    Some(
        0,
    ),
    2097152,
    2097152,
)
~~~

Confirm both `2 * 1024 * 1024` outputs are captured without a timeout; output is read only after a confirmed exit; the timeout and `try_wait` failure path still uses bounded cleanup; and only the intended source file plus new snapshot changed. Commit:

~~~shell
git add crates/daemon/tests/fixture_contracts.rs
git add crates/daemon/tests/snapshots/fixture_contracts__fixture_command_captures_verbose_output_without_pipe_backpressure.snap
git commit -m "fix(daemon): capture fixture output without pipe backpressure"
~~~

## Final Verification

Run final verification only after Task 7 and its commit. Tasks 1–4 are historical; Tasks 5–7 define the accepted final details.

~~~shell
cargo nextest run -p daemon --test fixture_contracts -E 'test(fixture_command_timeout_kills_and_reaps_child) | test(fixture_handler_marker_requires_complete_contents) | test(fixture_command_captures_verbose_output_without_pipe_backpressure)'
cargo nextest run -p daemon --test fixture_contracts
for iteration in {1..20}; do
  cargo nextest run -p daemon --test fixture_contracts -E 'test(mysql_fixture_exits_after_sigterm_with_idle_client)'
done
for iteration in {1..20}; do
  cargo nextest run -p daemon --test fixture_contracts -E 'test(postgres_fixture_exits_after_sigterm_with_idle_client)'
done
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo nextest run --workspace --all-features --locked
python3 -c 'import ast, pathlib; paths = [pathlib.Path(path) for path in ["crates/daemon/test-fixtures/managed-resources/mysql.py", "crates/daemon/test-fixtures/managed-resources/postgres.py", "crates/daemon/test-fixtures/managed-resources/fake-mailpit.py", "crates/daemon/test-fixtures/managed-resources/mailpit.py"]]; [ast.parse(path.read_text(), filename=str(path)) for path in paths]; print(f"parsed {len(paths)} Python fixtures")'
git diff --check HEAD~3..HEAD
git diff --name-only HEAD~3..HEAD
! rg --files crates/daemon/tests/snapshots -g '*.snap.new' -g '*.snap.tmp'
git ls-files --others --exclude-standard
git status --short
~~~

Expected: the focused new regressions and full fixture-contract suite pass; each lifecycle test passes 20 times; formatting, locked workspace Clippy, and the locked workspace suite pass; all four Python fixtures parse; the three-task artifact diff contains only `fixture_contracts.rs` and the two new snapshots with no whitespace errors or temporary snapshot artifacts; and both untracked-file and status checks are empty. The final behavior retains bounded cleanup, includes the test-only scheduling margin, accepts only complete handler markers, and captures 2 MiB of UTF-8 output from each stream without pipe backpressure.
