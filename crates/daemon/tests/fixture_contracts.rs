use std::io::{Error, ErrorKind, Read, Seek};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::process::{Child, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use camino::Utf8Path;
use camino_tempfile::{tempdir, tempfile};
use insta::{Settings, assert_debug_snapshot};
use rustix::io::Errno;
use rustix::process::{Pid, Signal, kill_process_group, test_kill_process};
use state::StateError;

const FIXTURE_COMMAND_TIMEOUT: Duration = Duration::from_secs(3);
const FIXTURE_COMMAND_POLL_INTERVAL: Duration = Duration::from_millis(10);
const FIXTURE_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
const FIXTURE_COMMAND_TIMEOUT_SCHEDULING_MARGIN: Duration = Duration::from_millis(100);
const FIXTURE_HANDLER_MARKER_CONTENTS: &str = "started\n";

const HANGING_FIXTURE: &str = r#"#!/usr/bin/env python3
import signal

signal.pause()
"#;
const VERBOSE_FIXTURE: &str = r#"#!/usr/bin/env python3
import sys


contents = "v" * (2 * 1024 * 1024)
sys.stdout.write(contents)
sys.stdout.flush()
sys.stderr.write(contents)
sys.stderr.flush()
"#;
const EXITED_FIXTURE: &str = r#"#!/usr/bin/env python3
import sys


sys.exit(0)
"#;
const LIVE_PROCESS_GROUP_FIXTURE: &str = r#"#!/usr/bin/env python3
import signal
import sys


with open(sys.argv[1], "w", encoding="utf-8") as marker:
    marker.write("started\n")

signal.pause()
"#;
#[expect(
    clippy::disallowed_types,
    reason = "daemon fixture contract tests execute materialized test programs"
)]
type FixtureCommand = std::process::Command;

#[derive(Debug)]
struct FixtureOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

#[test]
fn fixture_command_timeout_kills_and_reaps_child() -> Result<()> {
    let tempdir = tempdir()?;
    let fixture = tempdir.path().join("hanging-fixture");
    let timeout = Duration::from_secs(1);

    materialize_fixture(&fixture, HANGING_FIXTURE)?;
    let mut command = FixtureCommand::new(fixture.as_std_path());
    command.current_dir(tempdir.path());

    let started_at = Instant::now();
    let mut raw_child_pid = None;
    let error = match run_fixture_command(&mut command, timeout, Some(&mut raw_child_pid)) {
        Ok(output) => bail!("hanging fixture unexpectedly exited: {output:?}"),
        Err(error) => error,
    };
    let io_error = error
        .downcast_ref::<std::io::Error>()
        .ok_or_else(|| anyhow!("fixture timeout did not return an I/O error: {error}"))?;
    assert_eq!(io_error.kind(), ErrorKind::TimedOut);
    assert!(
        started_at.elapsed()
            < timeout
                + FIXTURE_SHUTDOWN_TIMEOUT
                + FIXTURE_COMMAND_POLL_INTERVAL
                + FIXTURE_COMMAND_POLL_INTERVAL
                + FIXTURE_COMMAND_TIMEOUT_SCHEDULING_MARGIN,
        "fixture command timeout exceeded its cleanup deadline"
    );

    let raw_child_pid =
        raw_child_pid.ok_or_else(|| anyhow!("fixture command did not report its child PID"))?;
    let child_pid = process_pid(raw_child_pid)?;
    match test_kill_process(child_pid) {
        Err(rustix::io::Errno::SRCH) => {}
        Ok(()) => bail!("fixture command child {child_pid} remained alive"),
        Err(error) => bail!("failed to inspect fixture command child {child_pid}: {error}"),
    }

    Ok(())
}

#[test]
fn fixture_command_captures_verbose_output_without_pipe_backpressure() -> Result<()> {
    let tempdir = tempdir()?;
    let fixture = tempdir.path().join("verbose-fixture");

    materialize_fixture(&fixture, VERBOSE_FIXTURE)?;
    let mut command = FixtureCommand::new(fixture.as_std_path());
    command.current_dir(tempdir.path());

    let output = run_fixture_command(&mut command, FIXTURE_COMMAND_TIMEOUT, None)?;
    assert_eq!(output.stdout.len(), 2 * 1024 * 1024);
    assert_eq!(output.stderr.len(), 2 * 1024 * 1024);
    assert_fixture_snapshot(
        tempdir.path(),
        "fixture_command_captures_verbose_output_without_pipe_backpressure",
        (output.code, output.stdout.len(), output.stderr.len()),
    )
}

#[test]
fn process_group_cleanup_does_not_kill_recycled_group_after_child_reaped() -> Result<()> {
    let tempdir = tempdir()?;
    let exited_fixture = tempdir.path().join("exited-fixture");
    let live_fixture = tempdir.path().join("live-fixture");
    let live_marker = tempdir.path().join("live-fixture-started");

    materialize_fixture(&exited_fixture, EXITED_FIXTURE)?;
    materialize_fixture(&live_fixture, LIVE_PROCESS_GROUP_FIXTURE)?;

    let mut exited_command = FixtureCommand::new(exited_fixture.as_std_path());
    exited_command.current_dir(tempdir.path()).process_group(0);
    let mut exited_child = exited_command.spawn()?;
    let exited_status = wait_for_child_status(&mut exited_child, FIXTURE_COMMAND_TIMEOUT)?
        .ok_or_else(|| anyhow!("exited fixture did not exit before the test setup completed"))?;
    if !exited_status.success() {
        bail!("exited fixture returned unexpected status {exited_status}");
    }

    let mut live_command = FixtureCommand::new(live_fixture.as_std_path());
    live_command
        .arg(live_marker.as_std_path())
        .current_dir(tempdir.path())
        .process_group(0);
    let mut live_child = live_command.spawn()?;
    let simulated_recycled_process_group = process_pid(live_child.id())?;
    let lifecycle = (|| {
        wait_for_handler_marker(&live_marker, FIXTURE_COMMAND_TIMEOUT)?;
        let cleanup_result =
            kill_process_group_and_reap_child(&mut exited_child, simulated_recycled_process_group);
        let live_group_was_alive =
            wait_for_child_status(&mut live_child, FIXTURE_SHUTDOWN_TIMEOUT)?.is_none();

        Ok::<_, anyhow::Error>((cleanup_result, live_group_was_alive))
    })();
    let cleanup =
        kill_process_group_and_reap_child(&mut live_child, simulated_recycled_process_group);

    let (cleanup_result, live_group_was_alive) = match lifecycle {
        Ok(result) => result,
        Err(error) => {
            cleanup?;
            return Err(error);
        }
    };
    cleanup?;
    assert!(
        live_group_was_alive,
        "cleanup killed the live process group through a recycled PGID"
    );
    cleanup_result
}

#[test]
fn fixture_handler_marker_requires_complete_contents() -> Result<()> {
    let tempdir = tempdir()?;
    let handler_marker = tempdir.path().join("handler-started");

    state::fs::write_sensitive_file(&handler_marker, "started")?;

    let error = match wait_for_handler_marker(&handler_marker, Duration::ZERO) {
        Ok(()) => bail!("incomplete fixture handler marker unexpectedly satisfied waiter"),
        Err(error) => error,
    };

    assert_fixture_snapshot(
        tempdir.path(),
        "fixture_handler_marker_requires_complete_contents",
        error.to_string(),
    )
}

fn materialize_fixture(path: &Utf8Path, source: &str) -> Result<()> {
    state::fs::write_sensitive_file(path, source)?;
    set_executable(path)
}

fn run_fixture_command(
    command: &mut FixtureCommand,
    timeout: Duration,
    child_pid: Option<&mut Option<u32>>,
) -> Result<FixtureOutput> {
    let mut stdout = tempfile()?;
    let mut stderr = tempfile()?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout.try_clone()?))
        .stderr(Stdio::from(stderr.try_clone()?));
    let mut child = command.spawn()?;
    if let Some(child_pid) = child_pid {
        *child_pid = Some(child.id());
    }
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

fn process_pid(pid: u32) -> Result<Pid> {
    let raw_pid = i32::try_from(pid)?;
    Pid::from_raw(raw_pid).ok_or_else(|| anyhow!("invalid process id {raw_pid}"))
}

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

fn wait_for_child_exit(child: &mut Child, timeout: Duration) -> Result<bool> {
    Ok(wait_for_child_status(child, timeout)?.is_some())
}

fn wait_for_child_status(child: &mut Child, timeout: Duration) -> Result<Option<ExitStatus>> {
    let deadline = Instant::now() + timeout;

    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }

        thread::sleep(FIXTURE_COMMAND_POLL_INTERVAL);
    }
}

fn kill_process_group_and_reap_child(child: &mut Child, process_group: Pid) -> Result<()> {
    if child.try_wait()?.is_some() {
        return Ok(());
    }

    let kill_error = match kill_process_group(process_group, Signal::KILL) {
        Ok(()) | Err(Errno::SRCH) => None,
        Err(error) => Some(error),
    };
    let reap_result = wait_for_child_status(child, FIXTURE_SHUTDOWN_TIMEOUT);

    if let Some(error) = kill_error {
        return Err(error.into());
    }
    if reap_result?.is_none() {
        return Err(Error::new(
            ErrorKind::TimedOut,
            format!(
                "timed out reaping fixture process-group leader after {} ms",
                FIXTURE_SHUTDOWN_TIMEOUT.as_millis()
            ),
        )
        .into());
    }

    Ok(())
}

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

fn assert_fixture_snapshot(
    tempdir: &Utf8Path,
    name: &'static str,
    snapshot: impl std::fmt::Debug,
) -> Result<()> {
    let mut settings = Settings::clone_current();
    settings.add_filter(&regex_literal(tempdir.as_str()), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!(name, snapshot);
        Ok::<(), anyhow::Error>(())
    })
}

#[expect(
    clippy::disallowed_methods,
    reason = "daemon fixture contract tests set materialized fixture executable bits directly"
)]
fn set_executable(path: &Utf8Path) -> Result<()> {
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions)?;

    Ok(())
}

fn regex_literal(value: &str) -> String {
    let mut literal = String::new();

    for character in value.chars() {
        if matches!(
            character,
            '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$'
        ) {
            literal.push('\\');
        }
        literal.push(character);
    }

    literal
}
