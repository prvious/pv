use std::ffi::OsString;
use std::io::{Error, ErrorKind, Read, Seek};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::PathBuf;
use std::process::{Child, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use camino::Utf8Path;
use camino_tempfile::{tempdir, tempfile};
use insta::{Settings, assert_debug_snapshot};
use rustix::io::Errno;
use rustix::process::{
    Pid, Signal, getpgid, kill_process, kill_process_group, test_kill_process,
    test_kill_process_group,
};
use state::StateError;
use state::fs::ensure_user_dir;

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
const REDIS_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/redis-server.py"
));
const MAILPIT_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/mailpit.py"
));
const POSTGRES_UNREADY_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/postgres-unready.sh"
));
const FAKE_CADDY_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/gateway/fake-caddy.sh"
));
const FAKE_CADDY_SERVER_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/gateway/fake-caddy-server.py"
));
const RUSTFS_FIXTURE_TEMPLATE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/rustfs.py.in"
));
const RUSTFS_REJECT_S3_SENTINEL: &str = "__PV_REJECT_S3__";
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
const FAILING_FQDN_SITECUSTOMIZE: &str = r#"import socket


def getfqdn(_name=""):
    raise RuntimeError("fixture attempted an FQDN lookup")


socket.getfqdn = getfqdn
"#;
const PARENT_LOSS_PYTHON_PROBE: &str = r#"import os


marker_path = os.environ["PV_PARENT_LOSS_MEMBER_PID"]
staging_path = f"{marker_path}.{os.getpid()}.tmp"
with open(staging_path, "w", encoding="utf-8") as marker:
    marker.write(str(os.getpid()))
os.replace(staging_path, marker_path)
"#;
const PARENT_LOSS_PS_PROBE: &str = r#"#!/bin/sh
/bin/ps "$@"
status=$?
printf 'started\n' > ./watcher-ready
exit "$status"
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
    Postgres,
    Redis,
}

impl SingleServerFixture {
    fn name(self) -> &'static str {
        match self {
            Self::Mysql => "MySQL",
            Self::Postgres => "PostgreSQL",
            Self::Redis => "Redis",
        }
    }

    fn executable_name(self) -> &'static str {
        match self {
            Self::Mysql => "mysqld",
            Self::Postgres => "postgres",
            Self::Redis => "redis-server",
        }
    }

    fn source(self) -> &'static str {
        match self {
            Self::Mysql => MYSQL_FIXTURE,
            Self::Postgres => POSTGRES_FIXTURE,
            Self::Redis => REDIS_FIXTURE,
        }
    }
}

#[derive(Clone, Copy)]
enum MultiServerFixture {
    FakeMailpit,
    Mailpit,
    Rustfs,
}

impl MultiServerFixture {
    fn name(self) -> &'static str {
        match self {
            Self::FakeMailpit => "fake Mailpit",
            Self::Mailpit => "Mailpit",
            Self::Rustfs => "RustFS",
        }
    }

    fn executable_name(self) -> &'static str {
        match self {
            Self::FakeMailpit => "fake-mailpit",
            Self::Mailpit => "mailpit",
            Self::Rustfs => "rustfs",
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum ParentLossFixture {
    ShellSql,
    DirectPythonMailpit,
    ShellToPythonGateway,
}

impl ParentLossFixture {
    fn name(self) -> &'static str {
        match self {
            Self::ShellSql => "shell-only SQL",
            Self::DirectPythonMailpit => "direct-Python Mailpit",
            Self::ShellToPythonGateway => "shell-to-Python gateway",
        }
    }

    fn has_python_member(self) -> bool {
        matches!(self, Self::DirectPythonMailpit | Self::ShellToPythonGateway)
    }
}

#[derive(Clone, Copy, Debug)]
enum ParentLossTiming {
    AfterReadiness,
    BeforeWatcherInitialization,
}

impl ParentLossTiming {
    fn name(self) -> &'static str {
        match self {
            Self::AfterReadiness => "after readiness",
            Self::BeforeWatcherInitialization => "before watcher initialization",
        }
    }
}

#[derive(Debug)]
struct ParentLossOutcome {
    fixture: &'static str,
    timing: &'static str,
    parent_exit_signal: Option<i32>,
    leader_stopped: bool,
    process_group_stopped: bool,
    ports_rebound: Vec<bool>,
}

#[derive(Clone)]
struct CapturedFixtureIdentity {
    process_group: Pid,
    members: Vec<CapturedFixtureMember>,
}

#[derive(Clone)]
struct CapturedFixtureMember {
    pid: Pid,
    start_identity: platform::ProcessStartIdentity,
}

impl ParentLossOutcome {
    fn succeeded(&self) -> bool {
        self.parent_exit_signal == Some(Signal::KILL.as_raw())
            && self.leader_stopped
            && self.process_group_stopped
            && self.ports_rebound.iter().all(|rebound| *rebound)
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
        if status.signal() != Some(Signal::TERM.as_raw()) {
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
fn single_server_fixture_exits_after_signal_status() -> Result<()> {
    for fixture in [
        SingleServerFixture::Mysql,
        SingleServerFixture::Postgres,
        SingleServerFixture::Redis,
    ] {
        for signal in [Signal::TERM, Signal::INT] {
            assert_single_server_fixture_exits_after_signal(fixture, signal)?;
        }
    }

    Ok(())
}

#[test]
fn multi_server_fixture_avoids_fqdn_lookup_and_exits_after_signal_status() -> Result<()> {
    for fixture in [
        MultiServerFixture::FakeMailpit,
        MultiServerFixture::Mailpit,
        MultiServerFixture::Rustfs,
    ] {
        for signal in [Signal::TERM, Signal::INT] {
            assert_multi_server_fixture_exits_after_signal(fixture, signal)?;
        }
    }

    Ok(())
}

#[test]
fn long_running_fixtures_exit_when_their_test_parent_is_lost() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }

    let mut outcomes = Vec::new();

    for fixture in [
        ParentLossFixture::ShellSql,
        ParentLossFixture::DirectPythonMailpit,
        ParentLossFixture::ShellToPythonGateway,
    ] {
        for timing in [
            ParentLossTiming::AfterReadiness,
            ParentLossTiming::BeforeWatcherInitialization,
        ] {
            outcomes.push(assert_fixture_exits_after_parent_loss(fixture, timing)?);
        }
    }

    assert_debug_snapshot!(outcomes, @r#"
    [
        ParentLossOutcome {
            fixture: "shell-only SQL",
            timing: "after readiness",
            parent_exit_signal: Some(
                9,
            ),
            leader_stopped: true,
            process_group_stopped: true,
            ports_rebound: [],
        },
        ParentLossOutcome {
            fixture: "shell-only SQL",
            timing: "before watcher initialization",
            parent_exit_signal: Some(
                9,
            ),
            leader_stopped: true,
            process_group_stopped: true,
            ports_rebound: [],
        },
        ParentLossOutcome {
            fixture: "direct-Python Mailpit",
            timing: "after readiness",
            parent_exit_signal: Some(
                9,
            ),
            leader_stopped: true,
            process_group_stopped: true,
            ports_rebound: [
                true,
                true,
            ],
        },
        ParentLossOutcome {
            fixture: "direct-Python Mailpit",
            timing: "before watcher initialization",
            parent_exit_signal: Some(
                9,
            ),
            leader_stopped: true,
            process_group_stopped: true,
            ports_rebound: [
                true,
                true,
            ],
        },
        ParentLossOutcome {
            fixture: "shell-to-Python gateway",
            timing: "after readiness",
            parent_exit_signal: Some(
                9,
            ),
            leader_stopped: true,
            process_group_stopped: true,
            ports_rebound: [
                true,
            ],
        },
        ParentLossOutcome {
            fixture: "shell-to-Python gateway",
            timing: "before watcher initialization",
            parent_exit_signal: Some(
                9,
            ),
            leader_stopped: true,
            process_group_stopped: true,
            ports_rebound: [
                true,
            ],
        },
    ]
    "#);

    Ok(())
}

#[test]
#[ignore = "nested parent process used by long_running_fixtures_exit_when_their_test_parent_is_lost"]
fn parent_loss_fixture_test_parent_inner() -> Result<()> {
    let mut command = FixtureCommand::new("./fixture-entrypoint");
    command.process_group(0);
    let mut fixture = command.spawn()?;
    let process_group = process_pid(fixture.id())?;
    if let Err(error) =
        state::fs::write_sensitive_file(Utf8Path::new("./fixture.pid"), &fixture.id().to_string())
    {
        return match kill_process_group_and_reap_child(&mut fixture, process_group) {
            Ok(()) => Err(error.into()),
            Err(cleanup_error) => Err(anyhow!(
                "{error}; fixture startup cleanup also failed: {cleanup_error}"
            )),
        };
    }

    loop {
        if let Some(status) = fixture.try_wait()? {
            bail!("parent-loss fixture exited before its test parent: {status}");
        }
        thread::sleep(FIXTURE_COMMAND_POLL_INTERVAL);
    }
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

#[test]
fn fake_mailpit_fixture_cli_ignores_extra_arguments() -> Result<()> {
    let tempdir = tempdir()?;
    let fixture = tempdir.path().join("fake-mailpit");
    let smtp_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let smtp_port = smtp_listener.local_addr()?.port();
    let dashboard_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let dashboard_port = dashboard_listener.local_addr()?.port();

    materialize_fixture(&fixture, FAKE_MAILPIT_FIXTURE)?;
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
        thread::sleep(Duration::from_millis(250));
        let running_while_smtp_is_reserved = child.try_wait()?.is_none();
        drop(smtp_listener);
        let readiness =
            wait_for_loopback_ports([smtp_port, dashboard_port], Duration::from_secs(3))?;
        let running_after_readiness = child.try_wait()?.is_none();

        Ok::<_, anyhow::Error>((
            running_while_smtp_is_reserved,
            readiness,
            running_after_readiness,
        ))
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

fn assert_fixture_exits_after_parent_loss(
    fixture: ParentLossFixture,
    timing: ParentLossTiming,
) -> Result<ParentLossOutcome> {
    let tempdir = tempdir()?;
    let port_reservations = prepare_parent_loss_fixture(tempdir.path(), fixture, timing)?;
    let ports = port_reservations
        .iter()
        .map(TcpListener::local_addr)
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .map(|address| address.port())
        .collect::<Vec<_>>();
    drop(port_reservations);

    let mut parent_stdout = tempfile()?;
    let mut parent_stderr = tempfile()?;
    let mut parent = FixtureCommand::new(current_test_binary()?);
    parent
        .args([
            "--exact",
            "parent_loss_fixture_test_parent_inner",
            "--ignored",
            "--nocapture",
        ])
        .current_dir(tempdir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::from(parent_stdout.try_clone()?))
        .stderr(Stdio::from(parent_stderr.try_clone()?));
    let mut parent = parent.spawn()?;
    let mut captured_identity = None;

    let lifecycle = (|| {
        let leader = wait_for_recorded_pid(
            &tempdir.path().join("fixture.pid"),
            &mut parent,
            FIXTURE_COMMAND_TIMEOUT,
        )?;
        let process_group = getpgid(Some(leader))
            .with_context(|| format!("failed to inspect fixture leader {leader}"))?;
        if process_group != leader {
            bail!(
                "parent-loss fixture leader {leader} did not own its process group {process_group}"
            );
        }
        captured_identity = Some(CapturedFixtureIdentity {
            process_group,
            members: vec![capture_fixture_member_identity(leader)?],
        });

        if matches!(timing, ParentLossTiming::BeforeWatcherInitialization) {
            wait_for_handler_marker(
                &tempdir.path().join("parent-captured"),
                FIXTURE_COMMAND_TIMEOUT,
            )?;
        }

        match timing {
            ParentLossTiming::AfterReadiness if ports.is_empty() => {
                wait_for_handler_marker(
                    &tempdir.path().join("watcher-ready"),
                    FIXTURE_COMMAND_TIMEOUT,
                )?;
            }
            ParentLossTiming::AfterReadiness => {
                let readiness = wait_for_loopback_ports_slice(&ports, FIXTURE_COMMAND_TIMEOUT)?;
                if !readiness.iter().all(|ready| *ready) {
                    bail!("{} did not become ready on ports {ports:?}", fixture.name());
                }
            }
            ParentLossTiming::BeforeWatcherInitialization => {}
        }
        if fixture.has_python_member() && matches!(timing, ParentLossTiming::AfterReadiness) {
            capture_fixture_member(
                captured_identity.as_mut(),
                &tempdir.path().join("member.pid"),
                FIXTURE_COMMAND_TIMEOUT,
            )
            .with_context(|| {
                format!(
                    "failed to capture {} member after readiness",
                    fixture.name()
                )
            })?;
        }

        parent
            .kill()
            .context("failed to kill fixture test parent")?;
        let parent_status = wait_for_child_status(&mut parent, FIXTURE_SHUTDOWN_TIMEOUT)?
            .ok_or_else(|| anyhow!("timed out reaping the fixture test parent"))?;

        let identity = captured_identity
            .as_ref()
            .ok_or_else(|| anyhow!("fixture process identity was not captured"))?;
        let (members_stopped, process_group_stopped) =
            wait_for_process_identity_exit(identity, FIXTURE_COMMAND_TIMEOUT)?;
        let ports_rebound = wait_for_loopback_ports_to_rebind(&ports, FIXTURE_COMMAND_TIMEOUT);

        Ok::<_, anyhow::Error>(ParentLossOutcome {
            fixture: fixture.name(),
            timing: timing.name(),
            parent_exit_signal: parent_status.signal(),
            leader_stopped: members_stopped.first().copied().unwrap_or(false),
            process_group_stopped,
            ports_rebound,
        })
    })();

    let lifecycle = lifecycle.and_then(|outcome| {
        if outcome.succeeded() {
            Ok(outcome)
        } else {
            bail!(
                "{} fixture failed parent-loss contract {}: {outcome:?}",
                outcome.fixture,
                outcome.timing
            )
        }
    });
    let emergency_identity = match &lifecycle {
        Ok(outcome) if outcome.succeeded() => None,
        Ok(_) | Err(_) => captured_identity.clone(),
    };
    let cleanup = cleanup_parent_loss_fixture(&mut parent, emergency_identity, &ports);

    match (lifecycle, cleanup) {
        (Ok(outcome), Ok(())) => Ok(outcome),
        (Err(error), Ok(())) => Err(parent_loss_error_with_output(
            error,
            &mut parent_stdout,
            &mut parent_stderr,
        )),
        (Ok(_outcome), Err(cleanup_error)) => Err(cleanup_error),
        (Err(error), Err(cleanup_error)) => {
            let error =
                parent_loss_error_with_output(error, &mut parent_stdout, &mut parent_stderr);
            Err(anyhow!(
                "{error}; emergency fixture cleanup also failed: {cleanup_error}"
            ))
        }
    }
}

fn parent_loss_error_with_output(
    error: anyhow::Error,
    stdout: &mut (impl Read + Seek),
    stderr: &mut (impl Read + Seek),
) -> anyhow::Error {
    let stdout = captured_fixture_output(stdout);
    let stderr = captured_fixture_output(stderr);
    anyhow!("{error}; nested stdout={stdout:?}; nested stderr={stderr:?}")
}

fn captured_fixture_output(file: &mut (impl Read + Seek)) -> String {
    match read_fixture_output(file) {
        Ok(contents) => String::from_utf8_lossy(&contents).into_owned(),
        Err(error) => format!("<failed to capture output: {error}>"),
    }
}

fn prepare_parent_loss_fixture(
    root: &Utf8Path,
    fixture: ParentLossFixture,
    timing: ParentLossTiming,
) -> Result<Vec<TcpListener>> {
    let mut port_reservations = Vec::new();
    let fixture_command = match fixture {
        ParentLossFixture::ShellSql => {
            let executable = root.join("postgres-unready");
            let data_dir = root.join("postgres-data");
            materialize_fixture(&executable, POSTGRES_UNREADY_FIXTURE)?;
            materialize_fixture(&root.join("probe-bin/ps"), PARENT_LOSS_PS_PROBE)?;
            state::fs::write_sensitive_file(&data_dir.join("PG_VERSION"), "16\n")?;
            "PATH=./probe-bin:$PATH\nexport PATH\nexec ./postgres-unready -D ./postgres-data -h 127.0.0.1 -p 5432\n".to_owned()
        }
        ParentLossFixture::DirectPythonMailpit => {
            port_reservations.push(TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?);
            port_reservations.push(TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?);
            let smtp_port = port_reservations[0].local_addr()?.port();
            let dashboard_port = port_reservations[1].local_addr()?.port();
            materialize_fixture(&root.join("mailpit"), MAILPIT_FIXTURE)?;
            ensure_user_dir(&root.join("mailpit-data"))?;
            format!(
                "exec ./mailpit --smtp 127.0.0.1:{smtp_port} --listen 127.0.0.1:{dashboard_port} --database ./mailpit-data/mailpit.db --disable-version-check\n"
            )
        }
        ParentLossFixture::ShellToPythonGateway => {
            port_reservations.push(TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?);
            let http_port = port_reservations[0].local_addr()?.port();
            let admin_socket = root.join("admin.sock");
            materialize_fixture(&root.join("fake-caddy"), FAKE_CADDY_FIXTURE)?;
            state::fs::write_sensitive_file(
                &root.join("fake-caddy.server.py"),
                FAKE_CADDY_SERVER_FIXTURE,
            )?;
            state::fs::write_sensitive_file(
                &root.join("Caddyfile"),
                &format!(
                    "{{\n    admin \"unix/{admin_socket}|0600\"\n    http_port {http_port}\n}}\n"
                ),
            )?;
            "exec ./fake-caddy run --config ./Caddyfile\n".to_owned()
        }
    };

    let parent_capture_gate = if matches!(timing, ParentLossTiming::BeforeWatcherInitialization) {
        "PV_TEST_PARENT_CAPTURE_MARKER=./parent-captured\n\
PV_TEST_PARENT_CAPTURE_RELEASE=./parent-capture-release\n\
export PV_TEST_PARENT_CAPTURE_MARKER PV_TEST_PARENT_CAPTURE_RELEASE\n"
    } else {
        ""
    };
    state::fs::write_sensitive_file(
        &root.join("parent-loss-probe/sitecustomize.py"),
        PARENT_LOSS_PYTHON_PROBE,
    )?;
    materialize_fixture(
        &root.join("fixture-entrypoint"),
        &format!(
            "#!/bin/sh\nset -eu\n\n{parent_capture_gate}PYTHONPATH=./parent-loss-probe\nPV_PARENT_LOSS_MEMBER_PID=./member.pid\nexport PYTHONPATH PV_PARENT_LOSS_MEMBER_PID\n{fixture_command}"
        ),
    )?;

    Ok(port_reservations)
}

fn wait_for_recorded_pid(path: &Utf8Path, parent: &mut Child, timeout: Duration) -> Result<Pid> {
    let deadline = Instant::now() + timeout;

    loop {
        match state::fs::read_to_string(path) {
            Ok(contents) => {
                let raw_pid = contents.trim().parse::<i32>()?;
                return Pid::from_raw(raw_pid)
                    .ok_or_else(|| anyhow!("fixture recorded invalid process id {raw_pid}"));
            }
            Err(StateError::Filesystem { source, .. }) if source.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if let Some(status) = parent.try_wait()? {
            bail!("fixture test parent exited before recording its child PID: {status}");
        }
        if Instant::now() >= deadline {
            bail!("timed out waiting for fixture PID at {path}");
        }

        thread::sleep(FIXTURE_COMMAND_POLL_INTERVAL);
    }
}

fn capture_fixture_member(
    identity: Option<&mut CapturedFixtureIdentity>,
    path: &Utf8Path,
    timeout: Duration,
) -> Result<()> {
    let identity = identity.ok_or_else(|| anyhow!("fixture process identity was not captured"))?;
    let member = wait_for_fixture_member_pid(path, timeout)?;
    record_captured_member(identity, member)
}

fn record_captured_member(identity: &mut CapturedFixtureIdentity, member: Pid) -> Result<()> {
    if identity
        .members
        .iter()
        .any(|captured| captured.pid == member)
    {
        return Ok(());
    }

    match getpgid(Some(member)) {
        Ok(process_group) if process_group == identity.process_group => {}
        Ok(process_group) => bail!(
            "fixture member {member} joined process group {process_group}; expected {}",
            identity.process_group
        ),
        Err(Errno::SRCH | Errno::PERM) => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect fixture member {member}"));
        }
    }
    identity
        .members
        .push(capture_fixture_member_identity(member)?);

    Ok(())
}

fn capture_fixture_member_identity(pid: Pid) -> Result<CapturedFixtureMember> {
    let raw_pid = u32::try_from(pid.as_raw_pid())?;
    let identity = platform::inspect_process_identity(raw_pid)?
        .ok_or_else(|| anyhow!("fixture member {pid} exited before identity capture"))?;

    Ok(CapturedFixtureMember {
        pid,
        start_identity: identity.start_identity,
    })
}

fn wait_for_fixture_member_pid(path: &Utf8Path, timeout: Duration) -> Result<Pid> {
    let deadline = Instant::now() + timeout;

    loop {
        match state::fs::read_to_string(path) {
            Ok(contents) => {
                let raw_pid = contents.trim().parse::<i32>()?;
                return Pid::from_raw(raw_pid)
                    .ok_or_else(|| anyhow!("fixture recorded invalid member PID {raw_pid}"));
            }
            Err(StateError::Filesystem { source, .. }) if source.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if Instant::now() >= deadline {
            bail!("timed out waiting for fixture member PID at {path}");
        }
        thread::sleep(FIXTURE_COMMAND_POLL_INTERVAL);
    }
}

fn wait_for_process_identity_exit(
    identity: &CapturedFixtureIdentity,
    timeout: Duration,
) -> Result<(Vec<bool>, bool)> {
    let deadline = Instant::now() + timeout;

    loop {
        let stopped = identity
            .members
            .iter()
            .map(|member| member_identity_is_stopped(member, identity.process_group))
            .collect::<Result<Vec<_>>>()?;
        let process_group_stopped = stopped.iter().all(|stopped| *stopped)
            && captured_process_group_is_stopped(identity.process_group)?;
        if process_group_stopped || Instant::now() >= deadline {
            return Ok((stopped, process_group_stopped));
        }
        thread::sleep(FIXTURE_COMMAND_POLL_INTERVAL);
    }
}

fn captured_process_group_is_stopped(process_group: Pid) -> Result<bool> {
    match test_kill_process_group(process_group) {
        Ok(()) => Ok(false),
        Err(Errno::SRCH) => Ok(true),
        // An inaccessible group cannot be verified as stopped. Cleanup will
        // fail without signaling it unless a captured member still matches.
        Err(Errno::PERM) => Ok(false),
        Err(error) => Err(error)
            .with_context(|| format!("failed to inspect fixture process group {process_group}")),
    }
}

fn member_identity_is_stopped(
    member: &CapturedFixtureMember,
    expected_process_group: Pid,
) -> Result<bool> {
    let process_group_changed = match getpgid(Some(member.pid)) {
        Ok(process_group) => process_group != expected_process_group,
        // An inaccessible PID cannot be proven to still be the captured fixture
        // member, so it must not authorize process-group cleanup.
        Err(Errno::SRCH | Errno::PERM) => return Ok(true),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect fixture member {}", member.pid));
        }
    };
    if process_group_changed {
        return Ok(true);
    }

    Ok(
        platform::inspect_process_identity(u32::try_from(member.pid.as_raw_pid())?)?
            .is_none_or(|identity| identity.start_identity != member.start_identity),
    )
}

fn wait_for_loopback_ports_to_rebind(ports: &[u16], timeout: Duration) -> Vec<bool> {
    let deadline = Instant::now() + timeout;

    loop {
        let rebound = ports
            .iter()
            .map(|port| TcpListener::bind((Ipv4Addr::LOCALHOST, *port)).is_ok())
            .collect::<Vec<_>>();
        if rebound.iter().all(|rebound| *rebound) || Instant::now() >= deadline {
            return rebound;
        }
        thread::sleep(FIXTURE_COMMAND_POLL_INTERVAL);
    }
}

fn cleanup_parent_loss_fixture(
    parent: &mut Child,
    identity: Option<CapturedFixtureIdentity>,
    ports: &[u16],
) -> Result<()> {
    let parent_cleanup = match parent.try_wait() {
        Ok(Some(_status)) => Ok(()),
        Ok(None) => kill_and_reap_child(parent).context("failed to stop fixture test parent"),
        Err(error) => Err(error).context("failed to inspect fixture test parent"),
    };
    let group_cleanup = if let Some(identity) = identity {
        cleanup_captured_process_group(&identity, ports)
    } else {
        Ok(())
    };

    match (parent_cleanup, group_cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(parent_error), Err(group_error)) => Err(anyhow!(
            "fixture parent cleanup failed: {parent_error}; fixture group cleanup failed: {group_error}"
        )),
    }
}

fn cleanup_captured_process_group(identity: &CapturedFixtureIdentity, ports: &[u16]) -> Result<()> {
    let mut verified_member = None;
    for member in &identity.members {
        if !member_identity_is_stopped(member, identity.process_group)? {
            verified_member = Some(member.pid);
            break;
        }
    }

    if verified_member.is_some() {
        match kill_process_group(identity.process_group, Signal::KILL) {
            Ok(()) | Err(Errno::SRCH) => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to kill verified fixture process group {}",
                        identity.process_group
                    )
                });
            }
        }
    }

    let (stopped, process_group_stopped) =
        wait_for_process_identity_exit(identity, FIXTURE_SHUTDOWN_TIMEOUT)?;
    let ports_rebound = wait_for_loopback_ports_to_rebind(ports, FIXTURE_SHUTDOWN_TIMEOUT);
    if !process_group_stopped || !ports_rebound.iter().all(|rebound| *rebound) {
        bail!(
            "fixture cleanup could not verify process group {} stopped; members={stopped:?}; ports={ports_rebound:?}",
            identity.process_group
        );
    }

    Ok(())
}

fn wait_for_loopback_ports_slice(ports: &[u16], timeout: Duration) -> Result<Vec<bool>> {
    let deadline = Instant::now() + timeout;
    let mut readiness = vec![false; ports.len()];

    loop {
        for (index, port) in ports.iter().enumerate() {
            if !readiness[index] && TcpStream::connect((Ipv4Addr::LOCALHOST, *port)).is_ok() {
                readiness[index] = true;
            }
        }
        if readiness.iter().all(|ready| *ready) {
            return Ok(readiness);
        }
        if Instant::now() >= deadline {
            return Ok(readiness);
        }
        thread::sleep(FIXTURE_COMMAND_POLL_INTERVAL);
    }
}

fn current_test_binary() -> Result<OsString> {
    let binary = std::env::args_os()
        .next()
        .ok_or_else(|| anyhow!("test binary path was missing"))?;
    let binary = PathBuf::from(binary);
    if binary.is_absolute() {
        return Ok(binary.into_os_string());
    }

    Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(binary)
        .into_os_string())
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
        SingleServerFixture::Postgres => {
            let data_dir = tempdir.path().join("postgres-data");
            state::fs::write_sensitive_file(&data_dir.join("PG_VERSION"), "16\n")?;
            state::fs::write_sensitive_file(
                &data_dir.join("postgresql.conf"),
                &format!("listen_addresses = '127.0.0.1'\nport = {port}\n"),
            )?;
            command.args([
                "-D",
                data_dir.as_str(),
                "-h",
                "127.0.0.1",
                "-p",
                port_argument.as_str(),
            ]);
        }
        SingleServerFixture::Redis => {
            let data_dir = tempdir.path().join("redis-data");
            let config_path = tempdir.path().join("redis.conf");
            state::fs::write_sensitive_file(
                &config_path,
                &format!(
                    "bind 127.0.0.1\nport {port}\ndir {}\nsave \"\"\nappendonly no\n",
                    data_dir.as_str()
                ),
            )?;
            command.arg(config_path.as_std_path());
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

fn assert_multi_server_fixture_exits_after_signal(
    fixture: MultiServerFixture,
    signal: Signal,
) -> Result<()> {
    let tempdir = tempdir()?;
    let executable = tempdir.path().join(fixture.executable_name());
    let first_port_reservation = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let first_port = first_port_reservation.local_addr()?.port();
    let second_port_reservation = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let second_port = second_port_reservation.local_addr()?.port();
    let first_address = format!("127.0.0.1:{first_port}");
    let second_address = format!("127.0.0.1:{second_port}");
    let sitecustomize = tempdir.path().join("sitecustomize.py");
    let source = match fixture {
        MultiServerFixture::FakeMailpit => FAKE_MAILPIT_FIXTURE.to_owned(),
        MultiServerFixture::Mailpit => MAILPIT_FIXTURE.to_owned(),
        MultiServerFixture::Rustfs => render_rustfs_fixture(false)?,
    };

    state::fs::write_sensitive_file(&sitecustomize, FAILING_FQDN_SITECUSTOMIZE)?;
    materialize_fixture(&executable, &source)?;
    let mut command = FixtureCommand::new(executable.as_std_path());
    command
        .current_dir(tempdir.path())
        .env("PYTHONPATH", tempdir.path());
    match fixture {
        MultiServerFixture::FakeMailpit => {
            command.args([first_port.to_string(), second_port.to_string()]);
        }
        MultiServerFixture::Mailpit => {
            let data_dir = tempdir.path().join("mailpit-data");
            ensure_user_dir(&data_dir)?;
            let database = data_dir.join("mailpit.db");
            command.args([
                "--smtp",
                first_address.as_str(),
                "--listen",
                second_address.as_str(),
                "--database",
                database.as_str(),
                "--disable-version-check",
            ]);
        }
        MultiServerFixture::Rustfs => {
            let data_dir = tempdir.path().join("rustfs-data");
            ensure_user_dir(&data_dir)?;
            command.args([
                "--address",
                first_address.as_str(),
                "--console-address",
                second_address.as_str(),
                data_dir.as_str(),
            ]);
        }
    }
    drop(first_port_reservation);
    drop(second_port_reservation);

    command.process_group(0);
    let mut child = command.spawn()?;
    let process_group = process_pid(child.id())?;
    let lifecycle = (|| {
        let readiness =
            wait_for_loopback_ports([first_port, second_port], FIXTURE_COMMAND_TIMEOUT)?;
        if readiness != [true, true] {
            bail!(
                "{} fixture did not become ready on ports {first_port} and {second_port}",
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
