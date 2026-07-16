# Daemon Fixture Review Corrections Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** Document the accepted fixture-contract subprocess cleanup hardening and deterministic threaded-handler synchronization in an executable historical sequence.

**Architecture:** A private synchronous fixture-command runner owns deadline polling, confirmed-exit output capture, and bounded cleanup. MySQL and PostgreSQL lifecycle tests wait for an opt-in UTF-8 handler-acceptance marker before signalling the fixture. Task 1's blocking cleanup and Task 2's fixed settle delay are historical interim states only; Tasks 3 and 4 supersede them before final verification.

**Tech Stack:** Rust 2024, anyhow, camino, rustix, Python 3 standard library, cargo-nextest, Clippy.

## Global Constraints

- Work only in the existing refactor/daemon-test-fixtures worktree and preserve unrelated user changes.
- Do not change production daemon or supervisor behavior, process ownership matching, nextest configuration, CI scheduling, fixture process-group topology, snapshots, or Cargo.lock.
- Python fixtures remain standard-library-only. Preserve normal exit status, stdout, stderr, custom MySQL environment variables, CLI behavior, and protocol behavior.
- Final behavior uses a 3-second command deadline, 10-millisecond poll interval, and 1-second bounded cleanup deadline. Every post-spawn timeout or try_wait error routes through bounded cleanup; the confirmed-exit wait_with_output path is unchanged.
- Final lifecycle tests use a per-test PV_FIXTURE_HANDLER_STARTED marker rather than a fixed delay. MySQL and PostgreSQL write and close that UTF-8 marker at the start of Handler.handle, before their blocking reads.
- Set daemon_threads = True on custom ThreadingMixIn servers for MySQL, PostgreSQL, fake Mailpit, and Mailpit. Redis retains its existing setting. Do not add Mailpit implementation-detail lifecycle tests because its handlers already return promptly.
- Preserve the positive HANGING_FIXTURE extraction-plan correction in docs/superpowers/plans/2026-07-15-daemon-test-fixtures.md; this plan does not alter it.
- The four historical commit boundaries map one-to-one to Tasks 1–4. Tasks 1 and 2 are historical interim states and must be followed by Tasks 3 and 4; neither their blocking reap nor their 100-millisecond settle sleep is acceptable final behavior.

## File Map

- Modify: crates/daemon/tests/fixture_contracts.rs
  - Final state: bounded command cleanup, handler-marker synchronization, and MySQL/PostgreSQL idle-client shutdown coverage.
- Modify: crates/daemon/test-fixtures/managed-resources/mysql.py
  - Final state: daemon request threads and optional handler marker before recv.
- Modify: crates/daemon/test-fixtures/managed-resources/postgres.py
  - Final state: daemon request threads and optional handler marker before read_startup.
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

At the beginning of each Handler.handle, add this exact env-gated UTF-8 write-and-close block. In MySQL it is immediately before self.request.recv(1024); in PostgreSQL it is immediately before the initial read_startup(self.request).

~~~python
handler_marker = os.environ.get("PV_FIXTURE_HANDLER_STARTED")
if handler_marker:
    with open(handler_marker, "w", encoding="utf-8") as marker:
        marker.write("started\n")
~~~

The final MySQL shape is:

~~~python
class Handler(socketserver.BaseRequestHandler):
    def handle(self):
        handler_marker = os.environ.get("PV_FIXTURE_HANDLER_STARTED")
        if handler_marker:
            with open(handler_marker, "w", encoding="utf-8") as marker:
                marker.write("started\n")
        self.request.recv(1024)
~~~

The final PostgreSQL shape places the same completed marker write before its first blocking read:

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

- [ ] **Step 3: Verify GREEN and final synchronization**

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

## Final Verification

Run final verification only after all four tasks. The accepted state has bounded cleanup and marker-based handler synchronization; it does not accept Task 1's blocking reap or Task 2's fixed settle sleep.

~~~shell
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo nextest run --workspace --all-features --locked
git diff --check 04ddff4802be29d14ae1bf78a6f03d5256543a9d..HEAD
git status --short
~~~

Expected: formatting, Clippy, and the locked workspace suite pass; normal fixture exits capture output only after confirmed exit; timeout and try_wait failures use bounded cleanup; MySQL and PostgreSQL use acceptance markers instead of a sleep; the branch diff has no whitespace errors; and the worktree is clean.
