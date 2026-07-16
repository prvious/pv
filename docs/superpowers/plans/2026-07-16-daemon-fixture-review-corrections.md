# Daemon Fixture Review Corrections Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** Bound fixture-contract subprocesses and make every custom threaded TCP fixture unable to delay interpreter shutdown through non-daemon request threads.

**Architecture:** Keep all changes inside daemon test fixtures and their executable-level integration test. One private synchronous command runner owns the deadline, output capture, kill, and reap behavior for commands expected to exit; the fixture classes themselves opt into daemon request threads without changing their CLI or protocol behavior.

**Tech Stack:** Rust 2024, anyhow, camino, rustix, Python 3 standard library, cargo-nextest, Clippy.

## Global Constraints

- Work only in the existing refactor/daemon-test-fixtures worktree and preserve unrelated user changes.
- Do not change production daemon or supervisor behavior, process ownership matching, nextest configuration, CI scheduling, or fixture process-group topology.
- Python fixtures remain standard-library-only; do not add dependencies or modify Cargo.lock.
- Normal fixture-contract commands use a 3-second deadline and a 10-millisecond poll interval.
- On timeout, kill and reap the direct child before returning std::io::ErrorKind::TimedOut; report kill or reap failures instead of the timeout.
- Preserve normal exit status, stdout, stderr, snapshots, custom MySQL environment variables, CLI behavior, and protocol behavior.
- Set daemon_threads = True on every custom socketserver.ThreadingMixIn fixture server: MySQL, PostgreSQL, fake Mailpit, and Mailpit. Redis retains its existing setting.
- Add behavior regressions for MySQL and PostgreSQL, whose handlers can remain blocked on accepted idle clients. Do not add implementation-detail tests for Mailpit handlers that already return promptly.
- Do not change adoption matching, add metric-driven docstrings, or include the unrelated gateway configuration-file cleanup.
- Follow test-driven development: observe the new test fail for the intended reason before implementing each behavior change.
- Keep implementation commits separate: timeout runner first, threaded-server shutdown second.

---

## File Map

- Modify: crates/daemon/tests/fixture_contracts.rs
  - Owns executable-level fixture CLI, timeout, cleanup, and shutdown regressions.
- Modify: crates/daemon/test-fixtures/managed-resources/mysql.py
  - Daemonizes MySQL request handler threads.
- Modify: crates/daemon/test-fixtures/managed-resources/postgres.py
  - Daemonizes PostgreSQL request handler threads.
- Modify: crates/daemon/test-fixtures/managed-resources/fake-mailpit.py
  - Normalizes its custom SMTP server thread policy.
- Modify: crates/daemon/test-fixtures/managed-resources/mailpit.py
  - Normalizes its custom SMTP server thread policy.

### Task 1: Bound Fixture-Contract Subprocesses

**Files:**
- Modify: crates/daemon/tests/fixture_contracts.rs

**Interfaces:**
- Consumes: the existing FixtureCommand alias, FixtureOutput type, materialize_fixture helper, and MySQL environment probe.
- Produces:
  - FIXTURE_COMMAND_TIMEOUT: Duration = 3 seconds
  - FIXTURE_COMMAND_POLL_INTERVAL: Duration = 10 milliseconds
  - run_fixture_command(command: &mut FixtureCommand, timeout: Duration) -> Result<FixtureOutput>
  - fixture_output(output: std::process::Output) -> Result<FixtureOutput>
  - process_pid(pid: u32) -> Result<rustix::process::Pid>

- [ ] **Step 1: Run the focused baseline**

Run:

~~~shell
cargo nextest run -p daemon --test fixture_contracts
~~~

Expected: all existing fixture-contract tests pass before the helper changes.

- [ ] **Step 2: Add the failing timeout regression and behavior-preserving scaffold**

Add these top-level imports and constants:

~~~rust
use std::io::{Error, ErrorKind};
use std::process::{Output, Stdio};

use anyhow::{Result, anyhow, bail};
use rustix::process::{Pid, test_kill_process};

const FIXTURE_COMMAND_TIMEOUT: Duration = Duration::from_secs(3);
const FIXTURE_COMMAND_POLL_INTERVAL: Duration = Duration::from_millis(10);
~~~

Keep the existing imports that are not replaced. Add this short scenario-local fixture beside the other fixture constants:

~~~rust
const HANGING_FIXTURE: &str = r#"#!/usr/bin/env python3
import os
import signal
import sys


with open(sys.argv[1], "w", encoding="utf-8") as pid_file:
    pid_file.write(f"{os.getpid()}\n")

signal.pause()
"#;
~~~

Add the regression after FixtureOutput:

~~~rust
#[test]
fn fixture_command_timeout_kills_and_reaps_child() -> Result<()> {
    let tempdir = tempdir()?;
    let fixture = tempdir.path().join("hanging-fixture");
    let pid_path = tempdir.path().join("hanging-fixture.pid");

    materialize_fixture(&fixture, HANGING_FIXTURE)?;
    let mut command = FixtureCommand::new(fixture.as_std_path());
    command.arg(pid_path.as_std_path()).current_dir(tempdir.path());

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

Add this temporary behavior-preserving scaffold so the test compiles but still demonstrates the old unbounded behavior:

~~~rust
fn run_fixture_command(
    command: &mut FixtureCommand,
    _timeout: Duration,
) -> Result<FixtureOutput> {
    fixture_output(command.output()?)
}

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

- [ ] **Step 3: Run the regression under a bounded outer process group and verify RED**

Run:

~~~shell
python3 - <<'PY'
import os
import signal
import subprocess
import sys

command = [
    "cargo",
    "nextest",
    "run",
    "-p",
    "daemon",
    "--test",
    "fixture_contracts",
    "-E",
    "test(fixture_command_timeout_kills_and_reaps_child)",
]
process = subprocess.Popen(command, start_new_session=True)
try:
    return_code = process.wait(timeout=3)
except subprocess.TimeoutExpired:
    os.killpg(process.pid, signal.SIGKILL)
    process.wait()
    print("RED: unbounded fixture command exceeded the outer deadline")
    sys.exit(1)

print(f"unexpected early exit from RED test: {return_code}")
sys.exit(2)
PY
~~~

Expected: the harness prints RED, kills the whole temporary test process group, reaps it, and exits 1. Confirm no hanging-fixture or fixture_contracts process remains before continuing.

- [ ] **Step 4: Implement the bounded runner**

Replace the scaffolded run_fixture_command with:

~~~rust
fn run_fixture_command(
    command: &mut FixtureCommand,
    timeout: Duration,
) -> Result<FixtureOutput> {
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

Keep fixture_output and process_pid exactly as introduced in Step 2.

Change run_fixture to build the command first and use the normal 3-second deadline:

~~~rust
fn run_fixture(
    path: &Utf8Path,
    arguments: &[&str],
    current_dir: &Utf8Path,
) -> Result<FixtureOutput> {
    let mut command = FixtureCommand::new(path.as_std_path());
    command.args(arguments).current_dir(current_dir);

    run_fixture_command(&mut command, FIXTURE_COMMAND_TIMEOUT)
}
~~~

Replace the direct MySQL empty-data-directory output call and manual conversion with:

~~~rust
let mut empty_data_dir_command = FixtureCommand::new(fixture.as_std_path());
empty_data_dir_command
    .args(["--no-defaults", "--initialize-insecure"])
    .current_dir(tempdir.path())
    .env("PYTHONPATH", &probe_dir)
    .env("PYTHONDONTWRITEBYTECODE", "1")
    .env("PV_MYSQL_MKDIR_PROBE", &probe_path);
let empty_data_dir_initialization =
    run_fixture_command(&mut empty_data_dir_command, FIXTURE_COMMAND_TIMEOUT)?;
~~~

- [ ] **Step 5: Run the timeout regression and verify GREEN**

Run:

~~~shell
cargo nextest run -p daemon --test fixture_contracts -E 'test(fixture_command_timeout_kills_and_reaps_child)'
~~~

Expected: one test passes in approximately one second, and the recorded child PID no longer exists.

- [ ] **Step 6: Run focused regression and lint checks**

Run:

~~~shell
cargo nextest run -p daemon --test fixture_contracts
cargo fmt --all --check
cargo clippy -p daemon --test fixture_contracts --locked -- -D warnings
git diff --check
~~~

Expected: all fixture-contract tests pass with the existing snapshots unchanged; formatting, Clippy, and diff checks pass.

- [ ] **Step 7: Self-review and commit**

Inspect the diff for these exact properties:

- both commands formerly using output() now call run_fixture_command,
- normal output conversion occurs only in fixture_output,
- timeout cleanup attempts wait_with_output even after a kill error,
- ErrorKind::InvalidInput is tolerated only as the exit race,
- no dependency or lockfile change exists.

Commit:

~~~shell
git add crates/daemon/tests/fixture_contracts.rs
git commit -m "fix(daemon): bound fixture contract subprocesses"
~~~

### Task 2: Daemonize Custom Fixture Request Handlers

**Files:**
- Modify: crates/daemon/tests/fixture_contracts.rs
- Modify: crates/daemon/test-fixtures/managed-resources/mysql.py
- Modify: crates/daemon/test-fixtures/managed-resources/postgres.py
- Modify: crates/daemon/test-fixtures/managed-resources/fake-mailpit.py
- Modify: crates/daemon/test-fixtures/managed-resources/mailpit.py

**Interfaces:**
- Consumes: Task 1's process_pid(pid: u32) -> Result<Pid>, FixtureCommand alias, fixture materialization helper, and timeout constants.
- Produces:
  - connect_to_loopback(port: u16, timeout: Duration) -> Result<TcpStream>
  - wait_for_child_exit(child: &mut std::process::Child, timeout: Duration) -> Result<bool>
  - kill_and_reap_child(child: &mut std::process::Child) -> Result<()>
  - MySQL and PostgreSQL active-idle-client shutdown regressions
  - daemon_threads = True on every custom ThreadingMixIn server

- [ ] **Step 1: Add top-level lifecycle imports and constants**

Extend the Task 1 imports:

~~~rust
use std::process::{Child, Output, Stdio};

use rustix::process::{Pid, Signal, kill_process, test_kill_process};

const FIXTURE_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
const FIXTURE_HANDLER_SETTLE_TIME: Duration = Duration::from_millis(100);
~~~

- [ ] **Step 2: Add the MySQL and PostgreSQL shutdown regressions before changing fixtures**

Add these tests after the timeout regression:

~~~rust
#[test]
fn mysql_fixture_exits_after_sigterm_with_idle_client() -> Result<()> {
    let tempdir = tempdir()?;
    let fixture = tempdir.path().join("mysqld");
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let port = listener.local_addr()?.port();
    let port_argument = port.to_string();

    materialize_fixture(&fixture, MYSQL_FIXTURE)?;
    drop(listener);

    let mut child = FixtureCommand::new(fixture.as_std_path())
        .args(["--port", port_argument.as_str()])
        .current_dir(tempdir.path())
        .spawn()?;
    let lifecycle = (|| {
        let _idle_client = connect_to_loopback(port, FIXTURE_COMMAND_TIMEOUT)?;
        thread::sleep(FIXTURE_HANDLER_SETTLE_TIME);
        kill_process(process_pid(child.id())?, Signal::TERM)?;
        if !wait_for_child_exit(&mut child, FIXTURE_SHUTDOWN_TIMEOUT)? {
            bail!("MySQL fixture did not exit after SIGTERM with an idle client");
        }

        Ok::<(), anyhow::Error>(())
    })();
    let cleanup = kill_and_reap_child(&mut child);

    if let Err(error) = lifecycle {
        cleanup?;
        return Err(error);
    }
    cleanup
}

#[test]
fn postgres_fixture_exits_after_sigterm_with_idle_client() -> Result<()> {
    let tempdir = tempdir()?;
    let fixture = tempdir.path().join("postgres");
    let data_dir = tempdir.path().join("postgres-data");
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let port = listener.local_addr()?.port();
    let port_argument = port.to_string();

    materialize_fixture(&fixture, POSTGRES_FIXTURE)?;
    state::fs::write_sensitive_file(&data_dir.join("PG_VERSION"), "16\n")?;
    state::fs::write_sensitive_file(
        &data_dir.join("postgresql.conf"),
        &format!("listen_addresses = '127.0.0.1'\nport = {port}\n"),
    )?;
    drop(listener);

    let mut child = FixtureCommand::new(fixture.as_std_path())
        .args([
            "-D",
            data_dir.as_str(),
            "-h",
            "127.0.0.1",
            "-p",
            port_argument.as_str(),
        ])
        .current_dir(tempdir.path())
        .spawn()?;
    let lifecycle = (|| {
        let _idle_client = connect_to_loopback(port, FIXTURE_COMMAND_TIMEOUT)?;
        thread::sleep(FIXTURE_HANDLER_SETTLE_TIME);
        kill_process(process_pid(child.id())?, Signal::TERM)?;
        if !wait_for_child_exit(&mut child, FIXTURE_SHUTDOWN_TIMEOUT)? {
            bail!("PostgreSQL fixture did not exit after SIGTERM with an idle client");
        }

        Ok::<(), anyhow::Error>(())
    })();
    let cleanup = kill_and_reap_child(&mut child);

    if let Err(error) = lifecycle {
        cleanup?;
        return Err(error);
    }
    cleanup
}
~~~

Add these helpers near wait_for_loopback_ports:

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

- [ ] **Step 3: Run both lifecycle regressions and verify RED**

Run:

~~~shell
cargo nextest run -p daemon --test fixture_contracts -E 'test(mysql_fixture_exits_after_sigterm_with_idle_client) | test(postgres_fixture_exits_after_sigterm_with_idle_client)'
~~~

Expected: both tests fail with their explicit did-not-exit messages after bounded waits. Their cleanup paths kill and reap both children, leaving no fixture listener behind.

- [ ] **Step 4: Apply the minimal threaded-server configuration**

In mysql.py:

~~~python
class TcpServer(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True
    daemon_threads = True
~~~

In postgres.py:

~~~python
class Server(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True
    daemon_threads = True
~~~

In fake-mailpit.py:

~~~python
class TcpServer(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True
    daemon_threads = True
~~~

In mailpit.py:

~~~python
class TcpServer(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True
    daemon_threads = True
~~~

Do not modify redis-server.py because it already has the required setting. Do not modify ThreadingHTTPServer subclasses or instances.

- [ ] **Step 5: Run both lifecycle regressions and verify GREEN**

Run:

~~~shell
cargo nextest run -p daemon --test fixture_contracts -E 'test(mysql_fixture_exits_after_sigterm_with_idle_client) | test(postgres_fixture_exits_after_sigterm_with_idle_client)'
~~~

Expected: both tests pass; each fixture exits during the one-second shutdown deadline while the idle client remains in scope.

- [ ] **Step 6: Verify Python syntax and focused daemon behavior**

Run:

~~~shell
python3 -c 'import ast, pathlib; paths = [pathlib.Path(path) for path in ["crates/daemon/test-fixtures/managed-resources/mysql.py", "crates/daemon/test-fixtures/managed-resources/postgres.py", "crates/daemon/test-fixtures/managed-resources/fake-mailpit.py", "crates/daemon/test-fixtures/managed-resources/mailpit.py"]]; [ast.parse(path.read_text(), filename=str(path)) for path in paths]; print(f"parsed {len(paths)} Python fixtures")'
cargo nextest run -p daemon --test fixture_contracts
cargo fmt --all --check
cargo clippy -p daemon --test fixture_contracts --locked -- -D warnings
git diff --check
~~~

Expected: four Python fixtures parse, all fixture-contract tests pass with no snapshot changes, and formatting, Clippy, and diff checks pass.

- [ ] **Step 7: Self-review and commit**

Inspect the diff for these exact properties:

- all four custom ThreadingMixIn classes set daemon_threads = True,
- Redis and all ThreadingHTTPServer code remain unchanged,
- shutdown tests keep the idle client alive until after the exit assertion,
- every failure path kills and reaps its child,
- no sleeps are used as the exit assertion; sleeps only allow the accepted handler to begin,
- adoption, docstrings, gateway configuration reading, dependencies, and snapshots remain unchanged.

Commit:

~~~shell
git add crates/daemon/tests/fixture_contracts.rs \
  crates/daemon/test-fixtures/managed-resources/mysql.py \
  crates/daemon/test-fixtures/managed-resources/postgres.py \
  crates/daemon/test-fixtures/managed-resources/fake-mailpit.py \
  crates/daemon/test-fixtures/managed-resources/mailpit.py
git commit -m "fix(daemon): daemonize fixture request handlers"
~~~

## Final Verification

After both implementation tasks and their task-scoped reviews are clean, run:

~~~shell
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo nextest run --workspace --all-features --locked
git diff --check 04ddff4802be29d14ae1bf78a6f03d5256543a9d..HEAD
git status --short
~~~

Expected: formatting and Clippy pass, the complete locked workspace suite passes, the branch diff has no whitespace errors, and the worktree is clean.
