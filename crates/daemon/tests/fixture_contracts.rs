use std::io::{Error, ErrorKind, Read, Seek};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use camino::Utf8Path;
use camino_tempfile::{tempdir, tempfile};
use insta::{Settings, assert_debug_snapshot};
use rustix::io::Errno;
use rustix::process::{Pid, Signal, kill_process, kill_process_group, test_kill_process};
use state::StateError;

const FIXTURE_COMMAND_TIMEOUT: Duration = Duration::from_secs(3);
const FIXTURE_COMMAND_POLL_INTERVAL: Duration = Duration::from_millis(10);
const FIXTURE_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
const FIXTURE_COMMAND_TIMEOUT_SCHEDULING_MARGIN: Duration = Duration::from_millis(100);
const FIXTURE_HANDLER_MARKER_CONTENTS: &str = "started\n";

const MYSQL_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/mysql.py"
));
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

#[derive(Clone, Copy)]
enum SingleServerFixture {
    Mysql,
}

impl SingleServerFixture {
    fn name(self) -> &'static str {
        match self {
            Self::Mysql => "MySQL",
        }
    }

    fn executable_name(self) -> &'static str {
        match self {
            Self::Mysql => "mysqld",
        }
    }

    fn source(self) -> &'static str {
        match self {
            Self::Mysql => MYSQL_FIXTURE,
        }
    }
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
fn mysql_fixture_exits_after_sigterm_with_idle_client() -> Result<()> {
    let tempdir = tempdir()?;
    let fixture = tempdir.path().join("mysqld");
    let handler_marker = tempdir.path().join("mysql-handler-started");
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let port = listener.local_addr()?.port();
    let port_argument = port.to_string();

    materialize_fixture(&fixture, MYSQL_FIXTURE)?;
    drop(listener);

    let mut child = FixtureCommand::new(fixture.as_std_path())
        .args(["--port", port_argument.as_str()])
        .current_dir(tempdir.path())
        .env("PV_FIXTURE_HANDLER_STARTED", handler_marker.as_std_path())
        .spawn()?;
    let lifecycle = (|| {
        let _idle_client = connect_to_loopback(port, FIXTURE_COMMAND_TIMEOUT)?;
        wait_for_handler_marker(&handler_marker, FIXTURE_COMMAND_TIMEOUT)?;
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
fn single_server_fixture_exits_after_signal_status() -> Result<()> {
    for fixture in [SingleServerFixture::Mysql] {
        for signal in [Signal::TERM, Signal::INT] {
            assert_single_server_fixture_exits_after_signal(fixture, signal)?;
        }
    }

    Ok(())
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

#[test]
fn mysql_fixture_cli_preserves_shell_contract() -> Result<()> {
    let tempdir = tempdir()?;
    let fixture = tempdir.path().join("mysqld");
    let probe_dir = tempdir.path().join("probe");
    let probe_path = tempdir.path().join("mkdir-target");
    let rejected_data_dir = tempdir.path().join("rejected-data");
    let first_data_dir = tempdir.path().join("first-data");
    let selected_data_dir = tempdir.path().join("selected-data");

    materialize_fixture(&fixture, MYSQL_FIXTURE)?;
    state::fs::write_sensitive_file(
        &probe_dir.join("sitecustomize.py"),
        r#"import os


def record_makedirs(path, mode=0o777, exist_ok=False):
    with open(os.environ["PV_MYSQL_MKDIR_PROBE"], "w", encoding="utf-8") as probe:
        probe.write(os.fspath(path))


os.makedirs = record_makedirs
"#,
    )?;

    let first_argument_failure = run_fixture(
        &fixture,
        &[
            "--initialize-insecure",
            "--no-defaults",
            "--datadir",
            rejected_data_dir.as_str(),
        ],
        tempdir.path(),
    )?;
    let successful_initialization = run_fixture(
        &fixture,
        &[
            "--no-defaults",
            "--bind-address=127.0.0.1",
            "--future-option",
            "--initialize-insecure",
            "--datadir",
            first_data_dir.as_str(),
            "--datadir",
            selected_data_dir.as_str(),
            "--basedir",
            tempdir.path().as_str(),
        ],
        tempdir.path(),
    )?;
    let mut empty_data_dir_command = FixtureCommand::new(fixture.as_std_path());
    empty_data_dir_command
        .args(["--no-defaults", "--initialize-insecure"])
        .current_dir(tempdir.path())
        .env("PYTHONPATH", &probe_dir)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("PV_MYSQL_MKDIR_PROBE", &probe_path);
    let empty_data_dir_initialization =
        run_fixture_command(&mut empty_data_dir_command, FIXTURE_COMMAND_TIMEOUT, None)?;

    assert_fixture_snapshot(
        tempdir.path(),
        "mysql_fixture_cli_preserves_shell_contract",
        (
            first_argument_failure,
            successful_initialization,
            path_exists(&rejected_data_dir.join("mysql"))?,
            path_exists(&first_data_dir.join("mysql"))?,
            path_exists(&selected_data_dir.join("mysql"))?,
            empty_data_dir_initialization,
            state::fs::read_to_string(&probe_path)?,
        ),
    )
}

fn assert_single_server_fixture_exits_after_signal(
    fixture: SingleServerFixture,
    signal: Signal,
) -> Result<()> {
    let tempdir = tempdir()?;
    let executable = tempdir.path().join(fixture.executable_name());
    let port_reservation = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let port = port_reservation.local_addr()?.port();
    let port_argument = port.to_string();

    materialize_fixture(&executable, fixture.source())?;
    let mut command = FixtureCommand::new(executable.as_std_path());
    command.current_dir(tempdir.path());
    match fixture {
        SingleServerFixture::Mysql => {
            let data_dir = tempdir.path().join("mysql-data");
            let socket_path = tempdir.path().join("mysql.sock");
            let init_file = tempdir.path().join("mysql-init.sql");
            state::fs::write_sensitive_file(&init_file, "")?;
            command.args([
                "--no-defaults",
                "--datadir",
                data_dir.as_str(),
                "--bind-address=127.0.0.1",
                "--port",
                port_argument.as_str(),
                "--mysqlx=0",
                "--socket",
                socket_path.as_str(),
                "--init-file",
                init_file.as_str(),
            ]);
        }
    }
    drop(port_reservation);

    command.process_group(0);
    let mut child = command.spawn()?;
    let process_group = process_pid(child.id())?;
    let lifecycle = (|| {
        let readiness = wait_for_loopback_ports([port], FIXTURE_COMMAND_TIMEOUT)?;
        if readiness != [true] {
            bail!(
                "{} fixture did not become ready on port {port}",
                fixture.name()
            );
        }

        kill_process_group(process_group, signal)?;
        let status =
            wait_for_child_status(&mut child, FIXTURE_SHUTDOWN_TIMEOUT)?.ok_or_else(|| {
                anyhow!(
                    "{} fixture did not exit after signal {}",
                    fixture.name(),
                    signal.as_raw()
                )
            })?;
        if status.signal() != Some(signal.as_raw()) {
            bail!(
                "{} fixture exited with {status} after signal {}; expected signal status {}",
                fixture.name(),
                signal.as_raw(),
                signal.as_raw()
            );
        }

        Ok::<(), anyhow::Error>(())
    })();
    let cleanup = kill_process_group_and_reap_child(&mut child, process_group);

    if let Err(error) = lifecycle {
        cleanup?;
        return Err(error);
    }
    cleanup
}

fn materialize_fixture(path: &Utf8Path, source: &str) -> Result<()> {
    state::fs::write_sensitive_file(path, source)?;
    set_executable(path)
}

fn run_fixture(
    path: &Utf8Path,
    arguments: &[&str],
    current_dir: &Utf8Path,
) -> Result<FixtureOutput> {
    let mut command = FixtureCommand::new(path.as_std_path());
    command.args(arguments).current_dir(current_dir);

    run_fixture_command(&mut command, FIXTURE_COMMAND_TIMEOUT, None)
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

fn wait_for_loopback_ports<const PORT_COUNT: usize>(
    ports: [u16; PORT_COUNT],
    timeout: Duration,
) -> Result<[bool; PORT_COUNT]> {
    let deadline = Instant::now() + timeout;
    let mut readiness = [false; PORT_COUNT];

    loop {
        for (index, port) in ports.into_iter().enumerate() {
            if !readiness[index] && TcpStream::connect((Ipv4Addr::LOCALHOST, port)).is_ok() {
                readiness[index] = true;
            }
        }

        if readiness.iter().all(|ready| *ready) {
            return Ok(readiness);
        }
        if Instant::now() >= deadline {
            bail!("timed out waiting for fixture ports {ports:?}; readiness: {readiness:?}");
        }

        thread::sleep(Duration::from_millis(10));
    }
}

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
    reason = "daemon fixture contract tests inspect fixture filesystem effects directly"
)]
fn path_exists(path: &Utf8Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
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
