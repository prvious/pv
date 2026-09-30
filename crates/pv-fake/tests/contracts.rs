//! Contract tests that run the `pv-fake` binary directly, without the daemon supervisor.
#![cfg(unix)]

use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, ExitStatus, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use camino::Utf8Path;
use camino_tempfile::tempdir;
use pv_fake::{EventKind, InstalledFake, Persona, Scenario};
use rustix::io::{FdFlags, fcntl_setfd};
use rustix::process::{Pid, Signal, kill_process};

#[expect(
    clippy::disallowed_types,
    reason = "contract tests run the pv-fake binary directly, without the daemon supervisor"
)]
type FakeCommand = std::process::Command;

/// Kills and reaps the fake when a test returns before it exits.
struct FakeProcess(Child);

impl Drop for FakeProcess {
    fn drop(&mut self) {
        let _kill_result = self.0.kill();
        let _wait_result = self.0.wait();
    }
}

const WAIT_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

#[test]
fn long_running_fake_records_its_lifecycle_and_exits_cleanly_on_sigterm() -> Result<()> {
    let tempdir = tempdir()?;
    let fake = install_long_running(tempdir.path(), "mysqld", None)?;
    let mut child = spawn(&fake, true)?;
    let lifeline_armed = wait_for_start(&fake)?;
    signal(&child, Signal::TERM)?;
    let status = wait_for_exit(&mut child)?;

    assert!(!lifeline_armed);
    assert_eq!(status.code(), Some(0));
    assert_eq!(
        fake.events()?.first().map(|event| &event.kind),
        Some(&EventKind::Started {
            persona: Persona::LongRunning,
            argv: vec![fake.executable().to_string()],
            lifeline_armed: false,
        })
    );
    assert_eq!(event_names(&fake)?, ["started", "signal SIGTERM", "exit 0"]);

    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn installed_fake_reports_its_install_path_as_its_process_identity() -> Result<()> {
    let tempdir = tempdir()?;
    let fake = install_long_running(tempdir.path(), "caddy", None)?;
    let mut child = spawn(&fake, true)?;
    wait_for_start(&fake)?;
    let identity = platform::inspect_process_identity(child.0.id())?;
    signal(&child, Signal::TERM)?;
    wait_for_exit(&mut child)?;

    let identity = identity.ok_or_else(|| anyhow!("fake process identity was unavailable"))?;
    assert_eq!(identity.executable.as_path(), fake.executable());
    assert_eq!(identity.argument_zero, fake.executable().as_str());

    Ok(())
}

#[test]
fn closing_the_lifeline_kills_a_fake_that_leads_its_process_group() -> Result<()> {
    let tempdir = tempdir()?;
    let (read, write) = lifeline_pipe()?;
    let fake = install_long_running(tempdir.path(), "frankenphp", Some(read.as_raw_fd()))?;
    let mut child = spawn(&fake, true)?;
    let lifeline_armed = wait_for_start(&fake)?;
    drop(write);
    let status = wait_for_exit(&mut child)?;

    assert!(lifeline_armed);
    assert_eq!(status.signal(), Some(Signal::KILL.as_raw()));
    assert_eq!(event_names(&fake)?, ["started", "lifeline_fired"]);

    Ok(())
}

#[test]
fn closing_the_lifeline_kills_only_a_fake_that_shares_the_test_process_group() -> Result<()> {
    let tempdir = tempdir()?;
    let (read, write) = lifeline_pipe()?;
    let fake = install_long_running(tempdir.path(), "mailpit", Some(read.as_raw_fd()))?;
    // Without its own group, the fake shares this test's group. If it signaled that group, the
    // test process would die with it.
    let mut child = spawn(&fake, false)?;
    let lifeline_armed = wait_for_start(&fake)?;
    drop(write);
    let status = wait_for_exit(&mut child)?;

    assert!(lifeline_armed);
    assert_eq!(status.signal(), Some(Signal::KILL.as_raw()));
    assert_eq!(event_names(&fake)?, ["started", "lifeline_fired"]);

    Ok(())
}

#[test]
fn descriptor_that_is_not_a_pipe_leaves_the_lifeline_unarmed() -> Result<()> {
    let tempdir = tempdir()?;
    // The fake's stdin is /dev/null, not a pipe.
    let fake = install_long_running(tempdir.path(), "redis-server", Some(0))?;
    let mut child = spawn(&fake, true)?;
    let lifeline_armed = wait_for_start(&fake)?;
    signal(&child, Signal::TERM)?;
    let status = wait_for_exit(&mut child)?;

    assert!(!lifeline_armed);
    assert_eq!(status.code(), Some(0));

    Ok(())
}

#[test]
fn fake_without_a_scenario_fails_with_an_actionable_error() -> Result<()> {
    let tempdir = tempdir()?;
    let fake = install_long_running(tempdir.path(), "postgres", None)?;
    let scenario_path = format!("{}.pv-fake.json", fake.executable());
    state::fs::remove_file(Utf8Path::new(&scenario_path))?;
    let output = FakeCommand::new(fake.executable())
        .stdin(Stdio::null())
        .output()?;

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr)?,
        format!(
            "pv-fake: no scenario file at {scenario_path}; install fakes with pv_fake::install\n"
        )
    );

    Ok(())
}

fn install_long_running(
    root: &Utf8Path,
    name: &str,
    lifeline_fd: Option<i32>,
) -> Result<InstalledFake> {
    pv_fake::install_with(
        Utf8Path::new(env!("CARGO_BIN_EXE_pv-fake")),
        &root.join("bin").join(name),
        &Scenario {
            persona: Persona::LongRunning,
            lifeline_fd,
        },
    )
}

fn spawn(fake: &InstalledFake, own_process_group: bool) -> Result<FakeProcess> {
    let mut command = FakeCommand::new(fake.executable());
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if own_process_group {
        command.process_group(0);
    }

    Ok(FakeProcess(command.spawn()?))
}

/// A pipe whose read end children inherit and whose write end they never do.
fn lifeline_pipe() -> Result<(OwnedFd, OwnedFd)> {
    let (read, write) = rustix::pipe::pipe()?;
    fcntl_setfd(&write, FdFlags::CLOEXEC)?;

    Ok((read, write))
}

/// Waits for the fake's `started` event and returns whether its lifeline is armed.
fn wait_for_start(fake: &InstalledFake) -> Result<bool> {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    loop {
        let started = fake
            .events()?
            .into_iter()
            .find_map(|event| match event.kind {
                EventKind::Started { lifeline_armed, .. } => Some(lifeline_armed),
                _ => None,
            });
        if let Some(lifeline_armed) = started {
            return Ok(lifeline_armed);
        }
        if Instant::now() >= deadline {
            bail!("fake {} did not record a started event", fake.executable());
        }
        sleep(POLL_INTERVAL);
    }
}

fn wait_for_exit(child: &mut FakeProcess) -> Result<ExitStatus> {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    loop {
        if let Some(status) = child.0.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            bail!("fake process {} did not exit", child.0.id());
        }
        sleep(POLL_INTERVAL);
    }
}

fn signal(child: &FakeProcess, signal: Signal) -> Result<()> {
    let raw_pid = i32::try_from(child.0.id())?;
    let pid = Pid::from_raw(raw_pid).ok_or_else(|| anyhow!("invalid process id {raw_pid}"))?;
    kill_process(pid, signal)?;

    Ok(())
}

fn event_names(fake: &InstalledFake) -> Result<Vec<String>> {
    Ok(fake
        .events()?
        .into_iter()
        .map(|event| match event.kind {
            EventKind::Started { .. } => "started".to_owned(),
            EventKind::Signal { signal } => format!("signal {signal}"),
            EventKind::LifelineFired => "lifeline_fired".to_owned(),
            EventKind::Exit { code } => format!("exit {code}"),
        })
        .collect())
}
