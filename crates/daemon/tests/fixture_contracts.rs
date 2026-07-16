use std::io::{Error, ErrorKind, Read, Seek};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::process::{Child, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use camino::Utf8Path;
use camino_tempfile::{tempdir, tempfile};
use insta::{Settings, assert_debug_snapshot};
use rustix::process::{Pid, Signal, kill_process, test_kill_process};
use state::StateError;

const FIXTURE_COMMAND_TIMEOUT: Duration = Duration::from_secs(3);
const FIXTURE_COMMAND_POLL_INTERVAL: Duration = Duration::from_millis(10);
const FIXTURE_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
const FIXTURE_COMMAND_TIMEOUT_SCHEDULING_MARGIN: Duration = Duration::from_millis(100);
const FIXTURE_HANDLER_MARKER_CONTENTS: &str = "started\n";
const POSTGRES_SHUTDOWN_INJECTION_MARKER_CONTENTS: &str = "injected\n";

const MYSQL_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/mysql.py"
));
const FAKE_MAILPIT_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/fake-mailpit.py"
));
const POSTGRES_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/postgres.py"
));
const MAILPIT_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/mailpit.py"
));
const RUSTFS_FIXTURE_TEMPLATE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/rustfs.py.in"
));
const RUSTFS_REJECT_S3_SENTINEL: &str = "__PV_REJECT_S3__";
const HANGING_FIXTURE: &str = r#"#!/usr/bin/env python3
import os
import signal
import sys


with open(sys.argv[1], "w", encoding="utf-8") as pid_file:
    pid_file.write(f"{os.getpid()}\n")

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
const POSTGRES_SHUTDOWN_SITECUSTOMIZE: &str = r#"import os
import signal
import socketserver
import threading


injected = False


def inject():
    global injected
    if injected:
        return
    injected = True
    with open(os.environ["PV_POSTGRES_SHUTDOWN_MARKER"], "w", encoding="utf-8") as marker:
        marker.write("injected\n")
    os.kill(os.getpid(), signal.SIGTERM)


original_pause = signal.pause


def pause():
    inject()
    return original_pause()


signal.pause = pause

original_serve_forever = socketserver.BaseServer.serve_forever


def serve_forever(self, *args, **kwargs):
    if threading.current_thread() is threading.main_thread():
        inject()
    return original_serve_forever(self, *args, **kwargs)


socketserver.BaseServer.serve_forever = serve_forever
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
    let pid_path = tempdir.path().join("hanging-fixture.pid");
    let timeout = Duration::from_secs(1);

    materialize_fixture(&fixture, HANGING_FIXTURE)?;
    let mut command = FixtureCommand::new(fixture.as_std_path());
    command
        .arg(pid_path.as_std_path())
        .current_dir(tempdir.path());

    let started_at = Instant::now();
    let error = match run_fixture_command(&mut command, timeout) {
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

    let raw_pid = state::fs::read_to_string(&pid_path)?
        .trim()
        .parse::<u32>()?;
    let pid = process_pid(raw_pid)?;
    assert!(test_kill_process(pid).is_err());

    Ok(())
}

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
fn postgres_fixture_exits_after_sigterm_with_idle_client() -> Result<()> {
    let tempdir = tempdir()?;
    let fixture = tempdir.path().join("postgres");
    let data_dir = tempdir.path().join("postgres-data");
    let handler_marker = tempdir.path().join("postgres-handler-started");
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
        .env("PV_FIXTURE_HANDLER_STARTED", handler_marker.as_std_path())
        .spawn()?;
    let lifecycle = (|| {
        let _idle_client = connect_to_loopback(port, FIXTURE_COMMAND_TIMEOUT)?;
        wait_for_handler_marker(&handler_marker, FIXTURE_COMMAND_TIMEOUT)?;
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

#[test]
fn postgres_fixture_shutdown_is_deterministic_after_sigterm() -> Result<()> {
    let tempdir = tempdir()?;
    let fixture = tempdir.path().join("postgres");
    let data_dir = tempdir.path().join("postgres-data");
    let probe_dir = tempdir.path().join("probe");
    let shutdown_marker = tempdir.path().join("postgres-shutdown-injected");
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let port = listener.local_addr()?.port();
    let port_argument = port.to_string();

    materialize_fixture(&fixture, POSTGRES_FIXTURE)?;
    state::fs::write_sensitive_file(
        &probe_dir.join("sitecustomize.py"),
        POSTGRES_SHUTDOWN_SITECUSTOMIZE,
    )?;
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
        .env("PYTHONPATH", probe_dir.as_std_path())
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("PV_POSTGRES_SHUTDOWN_MARKER", shutdown_marker.as_std_path())
        .spawn()?;
    let lifecycle = (|| {
        let deadline = Instant::now() + FIXTURE_COMMAND_TIMEOUT;
        loop {
            match state::fs::read_to_string(&shutdown_marker) {
                Ok(contents) if contents == POSTGRES_SHUTDOWN_INJECTION_MARKER_CONTENTS => break,
                Ok(_) => {}
                Err(StateError::Filesystem { source, .. })
                    if source.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            if Instant::now() >= deadline {
                bail!("timed out waiting for PostgreSQL shutdown injection marker");
            }

            thread::sleep(FIXTURE_COMMAND_POLL_INTERVAL);
        }

        let deadline = Instant::now() + FIXTURE_SHUTDOWN_TIMEOUT;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() >= deadline {
                bail!("PostgreSQL fixture did not exit after injected SIGTERM");
            }

            thread::sleep(FIXTURE_COMMAND_POLL_INTERVAL);
        };
        if !status.success() && status.signal() != Some(Signal::TERM.as_raw()) {
            bail!("PostgreSQL fixture exited unexpectedly after injected SIGTERM: {status}");
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
        run_fixture_command(&mut empty_data_dir_command, FIXTURE_COMMAND_TIMEOUT)?;

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

#[test]
fn fake_mailpit_fixture_cli_ignores_extra_arguments() -> Result<()> {
    let tempdir = tempdir()?;
    let fixture = tempdir.path().join("fake-mailpit");
    let smtp_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let smtp_port = smtp_listener.local_addr()?.port();
    let dashboard_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let dashboard_port = dashboard_listener.local_addr()?.port();

    materialize_fixture(&fixture, FAKE_MAILPIT_FIXTURE)?;
    drop(smtp_listener);
    drop(dashboard_listener);

    let mut child = FixtureCommand::new(fixture.as_std_path())
        .args([
            smtp_port.to_string(),
            dashboard_port.to_string(),
            "ignored-extra".to_owned(),
        ])
        .current_dir(tempdir.path())
        .spawn()?;
    let lifecycle = (|| {
        let readiness =
            wait_for_loopback_ports([smtp_port, dashboard_port], Duration::from_secs(3))?;
        let running_after_readiness = child.try_wait()?.is_none();

        Ok::<_, anyhow::Error>((readiness, running_after_readiness))
    })();
    let cleanup = kill_and_reap_child(&mut child);

    let lifecycle = match lifecycle {
        Ok(lifecycle) => lifecycle,
        Err(error) => {
            cleanup?;
            return Err(error);
        }
    };
    cleanup?;

    assert_fixture_snapshot(
        tempdir.path(),
        "fake_mailpit_fixture_cli_ignores_extra_arguments",
        lifecycle,
    )
}

#[test]
fn postgres_fixture_cli_preserves_shell_contract() -> Result<()> {
    let tempdir = tempdir()?;
    let fixture = tempdir.path().join("postgres");
    let initialized_data_dir = tempdir.path().join("initialized-postgres");
    let selected_missing_data_dir = tempdir.path().join("selected-missing-postgres");

    materialize_fixture(&fixture, POSTGRES_FIXTURE)?;
    state::fs::write_sensitive_file(&initialized_data_dir.join("PG_VERSION"), "16\n")?;

    let unknown_argument = run_fixture(&fixture, &["--unexpected"], tempdir.path())?;
    let last_data_dir_wins = run_fixture(
        &fixture,
        &[
            "-D",
            initialized_data_dir.as_str(),
            "-D",
            selected_missing_data_dir.as_str(),
            "-h",
            "127.0.0.1",
            "-p",
            "5432",
        ],
        tempdir.path(),
    )?;

    assert_fixture_snapshot(
        tempdir.path(),
        "postgres_fixture_cli_preserves_shell_contract",
        (unknown_argument, last_data_dir_wins),
    )
}

#[test]
fn mailpit_fixture_cli_preserves_shell_contract() -> Result<()> {
    let tempdir = tempdir()?;
    let fixture = tempdir.path().join("mailpit");
    let missing_database = tempdir.path().join("missing/mailpit.db");

    materialize_fixture(&fixture, MAILPIT_FIXTURE)?;

    let unknown_argument = run_fixture(&fixture, &["--unexpected"], tempdir.path())?;
    let missing_required_arguments =
        run_fixture(&fixture, &["--disable-version-check"], tempdir.path())?;
    let missing_version_check = run_fixture(
        &fixture,
        &[
            "--smtp",
            "127.0.0.1:1025",
            "--listen",
            "127.0.0.1:8025",
            "--database",
            missing_database.as_str(),
        ],
        tempdir.path(),
    )?;
    let invalid_database_path = run_fixture(
        &fixture,
        &[
            "--smtp",
            "127.0.0.1:1025",
            "--listen",
            "127.0.0.1:8025",
            "--database",
            "mailpit.db",
            "--disable-version-check",
        ],
        tempdir.path(),
    )?;
    let missing_database_directory = run_fixture(
        &fixture,
        &[
            "--smtp",
            "127.0.0.1:1025",
            "--listen",
            "127.0.0.1:8025",
            "--database",
            missing_database.as_str(),
            "--disable-version-check",
        ],
        tempdir.path(),
    )?;
    let duplicate_database_last_wins = run_fixture(
        &fixture,
        &[
            "--smtp",
            "127.0.0.1:1025",
            "--listen",
            "127.0.0.1:8025",
            "--database",
            "mailpit.db",
            "--database",
            missing_database.as_str(),
            "--disable-version-check",
        ],
        tempdir.path(),
    )?;

    assert_fixture_snapshot(
        tempdir.path(),
        "mailpit_fixture_cli_preserves_shell_contract",
        (
            unknown_argument,
            missing_required_arguments,
            missing_version_check,
            invalid_database_path,
            missing_database_directory,
            duplicate_database_last_wins,
        ),
    )
}

#[test]
fn rustfs_fixture_cli_preserves_shell_contract() -> Result<()> {
    let tempdir = tempdir()?;
    let fixture = tempdir.path().join("rustfs");
    let first_data_dir = tempdir.path().join("first-rustfs-data");
    let selected_data_dir = tempdir.path().join("selected-rustfs-data");
    let rendered = render_rustfs_fixture(false)?;

    materialize_fixture(&fixture, &rendered)?;
    let output = run_fixture(
        &fixture,
        &[
            first_data_dir.as_str(),
            "--future-option",
            selected_data_dir.as_str(),
            "--address",
            "invalid-api-address",
            "--console-address",
            "invalid-console-address",
        ],
        tempdir.path(),
    )?;

    assert_fixture_snapshot(
        tempdir.path(),
        "rustfs_fixture_cli_preserves_shell_contract",
        (
            output.code,
            output.stdout,
            output.stderr.contains("ValueError"),
            path_exists(&first_data_dir)?,
            path_exists(&selected_data_dir.join("buckets"))?,
            path_exists(&selected_data_dir.join("process-env"))?,
            path_exists(&tempdir.path().join("invalid-api-address"))?,
            path_exists(&tempdir.path().join("invalid-console-address"))?,
            rendered.contains(RUSTFS_REJECT_S3_SENTINEL),
        ),
    )
}

fn render_rustfs_fixture(reject_s3: bool) -> Result<String> {
    let occurrence_count = RUSTFS_FIXTURE_TEMPLATE
        .matches(RUSTFS_REJECT_S3_SENTINEL)
        .count();
    if occurrence_count != 1 {
        bail!(
            "RustFS fixture must contain exactly one {RUSTFS_REJECT_S3_SENTINEL} sentinel; found {occurrence_count}"
        );
    }

    let replacement = if reject_s3 { "True" } else { "False" };
    let rendered = RUSTFS_FIXTURE_TEMPLATE.replacen(RUSTFS_REJECT_S3_SENTINEL, replacement, 1);
    if rendered.contains(RUSTFS_REJECT_S3_SENTINEL) {
        bail!("RustFS fixture still contains {RUSTFS_REJECT_S3_SENTINEL} after rendering");
    }

    Ok(rendered)
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

    run_fixture_command(&mut command, FIXTURE_COMMAND_TIMEOUT)
}

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

fn process_pid(pid: u32) -> Result<Pid> {
    let raw_pid = i32::try_from(pid)?;
    Pid::from_raw(raw_pid).ok_or_else(|| anyhow!("invalid process id {raw_pid}"))
}

fn wait_for_loopback_ports(ports: [u16; 2], timeout: Duration) -> Result<[bool; 2]> {
    let deadline = Instant::now() + timeout;
    let mut readiness = [false; 2];

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
            bail!("timed out waiting for fake Mailpit ports {ports:?}; readiness: {readiness:?}");
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
