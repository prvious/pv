//! Process contract tests for per-runtime monitors, run through the `pv-monitor` example.
//!
//! Each monitor watches this test process's lifeline, so a test that fails part-way leaves no
//! monitor or runtime behind: when the test process exits, the monitor stops the runtime.

#![cfg(target_os = "macos")]

use std::collections::BTreeMap;
use std::os::fd::{AsRawFd as _, OwnedFd};
use std::os::unix::process::CommandExt as _;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::{Utf8TempDir, tempdir};
use daemon::{
    DaemonError, MonitorCleanup, MonitorErrorKind, MonitorExit, MonitorHooks, MonitorPause,
    MonitorStart, MonitorState, MonitorStop, StopSignal,
};
use insta::{Settings, assert_snapshot};
use pv_fake::{EventKind, FakeSettings, InstalledFake, Persona, Scenario};
use rustix::io::{FdFlags, fcntl_setfd};
use rustix::process::{Pid, Signal, kill_process, kill_process_group, test_kill_process};
use state::{MonitorReservation, PvPaths, StateError};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixStream;
use tokio::time::sleep;

#[expect(
    clippy::disallowed_types,
    reason = "monitor tests spawn unrelated processes to prove what monitors never touch"
)]
type TestCommand = std::process::Command;

const SUBJECT: &str = "gateway";
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const WAIT_ATTEMPTS: usize = 1_000;
const FALLBACK: MonitorStop = MonitorStop {
    signal: StopSignal::Terminate,
    grace_ms: 1_000,
};
const STOP: MonitorStop = MonitorStop {
    signal: StopSignal::Terminate,
    grace_ms: 5_000,
};

#[tokio::test]
async fn monitor_runs_a_runtime_through_stop_and_release() -> Result<()> {
    let harness = Harness::new()?;
    let fake = harness.fake(Persona::LongRunning)?;

    let started = harness
        .start(harness.start_for(fake.executable(), &[])?)
        .await?;
    let fake_pid = wait_for_fake_start(&fake).await?;
    let record = harness.record()?;
    let runtime_birth = platform::inspect_process_start_identity(started.runtime_pid)?;
    let stopped = daemon::stop_monitor(&harness.paths, SUBJECT, STOP).await?;
    daemon::release_monitor(&harness.paths, SUBJECT).await?;

    assert_eq!(started.cleanup, MonitorCleanup::Pending);
    assert_eq!(started.exit, None);
    // The runtime is the gate after exec: same pid, same birth identity.
    assert_eq!(fake_pid, started.runtime_pid);
    assert_eq!(
        runtime_birth.map(|birth| serde_json::json!(birth)),
        Some(record["runtime"]["start_identity"].clone())
    );
    assert_eq!(stopped.stop, Some(STOP));
    assert_eq!(stopped.exit, Some(MonitorExit::Code(0)));
    assert_eq!(stopped.cleanup, MonitorCleanup::Complete);
    assert!(!state::fs::path_exists(&harness.monitor_dir()));
    assert!(harness.reservation_is_free()?);
    let record = serde_json::to_string_pretty(&record)?;
    harness.snapshot(|| assert_snapshot!(record));

    Ok(())
}

#[tokio::test]
async fn monitor_requests_follow_the_frozen_contract() -> Result<()> {
    let harness = Harness::new()?;
    let fake = harness.fake(Persona::LongRunning)?;
    harness
        .start(harness.start_for(fake.executable(), &[])?)
        .await?;
    let instance = harness.record()?["instance"]
        .as_str()
        .ok_or_else(|| anyhow!("record has no instance"))?
        .to_owned();

    let mut replies = Vec::new();
    for request in [
        format!(r#"{{"version":1,"instance":"{instance}","op":"state","added_later":true}}"#),
        format!(r#"{{"version":2,"instance":"{instance}","op":"state"}}"#),
        r#"{"version":1,"instance":"another","op":"state"}"#.to_owned(),
        format!(r#"{{"version":1,"instance":"{instance}","op":"restart"}}"#),
        format!(r#"{{"version":1,"instance":"{instance}","op":"release"}}"#),
        "not json".to_owned(),
    ] {
        let reply = harness.raw_request(&request).await?;
        replies.push(format!("{request}\n{reply}"));
    }
    daemon::stop_monitor(&harness.paths, SUBJECT, STOP).await?;
    daemon::release_monitor(&harness.paths, SUBJECT).await?;

    let mut settings = Settings::clone_current();
    settings.add_filter(&instance, "<instance>");
    settings.add_filter(r#""(monitor_pid|runtime_pid)":\d+"#, r#""$1":<pid>"#);
    settings.bind(|| assert_snapshot!(replies.join("\n\n")));

    Ok(())
}

#[tokio::test]
async fn monitor_start_reports_a_missing_command_and_leaves_no_instance() -> Result<()> {
    let harness = Harness::new()?;
    let missing = harness.paths.root().join("runtime/bin/missing");

    let started = harness.start(harness.start_for(&missing, &[])?).await;

    let Err(DaemonError::MonitorStartupFailed { message, .. }) = started else {
        bail!("expected a startup failure, got {started:?}");
    };
    harness.snapshot(|| assert_snapshot!(message));
    assert!(!state::fs::path_exists(&harness.monitor_dir()));
    assert!(harness.reservation_is_free()?);

    Ok(())
}

#[tokio::test]
async fn monitor_retains_an_exit_nobody_was_connected_for() -> Result<()> {
    let harness = Harness::new()?;
    let shell = Utf8Path::new("/bin/sh");

    harness
        .start(harness.start_for(shell, &["-c", "exit 7"])?)
        .await?;
    let exited = harness
        .wait_for_state(|state| state.cleanup != MonitorCleanup::Pending)
        .await?;
    // A bare release, as from a controller that disconnects right after: the monitor removes
    // its own records.
    let instance = harness.instance()?;
    harness
        .raw_request(&format!(
            r#"{{"version":1,"instance":"{instance}","op":"release"}}"#
        ))
        .await?;
    wait_until(|| harness.reservation_is_free()).await?;

    assert_eq!(exited.exit, Some(MonitorExit::Code(7)));
    assert_eq!(exited.stop, None);
    assert_eq!(exited.cleanup, MonitorCleanup::Complete);
    assert!(!state::fs::path_exists(&harness.monitor_dir()));

    Ok(())
}

#[tokio::test]
async fn monitor_stop_outlives_its_client_and_keeps_its_first_deadline() -> Result<()> {
    let harness = Harness::new()?;
    let shell = Utf8Path::new("/bin/sh");
    harness
        .start(harness.start_for(
            shell,
            &[
                "-c",
                "trap '' TERM; echo ready; while :; do sleep 0.1; done",
            ],
        )?)
        .await?;
    harness.wait_for_log("ready").await?;
    let instance = harness.instance()?;

    // The raw request disconnects as soon as it is accepted.
    harness
        .raw_request(&format!(
            r#"{{"version":1,"instance":"{instance}","op":"stop","signal":"terminate","grace_ms":200}}"#
        ))
        .await?;
    // A repeated stop waits for the stop the monitor runs, under the first request's deadline.
    let stopped = daemon::stop_monitor(
        &harness.paths,
        SUBJECT,
        MonitorStop {
            signal: StopSignal::Interrupt,
            grace_ms: 60_000,
        },
    )
    .await?;
    daemon::release_monitor(&harness.paths, SUBJECT).await?;

    assert_eq!(
        stopped.stop,
        Some(MonitorStop {
            signal: StopSignal::Terminate,
            grace_ms: 200,
        })
    );
    assert_eq!(stopped.exit, Some(MonitorExit::Signal(9)));
    assert_eq!(stopped.cleanup, MonitorCleanup::Complete);

    Ok(())
}

#[tokio::test]
async fn monitor_reports_a_leader_exit_while_its_group_lives_on() -> Result<()> {
    let harness = Harness::new()?;
    let shell = Utf8Path::new("/bin/sh");
    let mut start = harness.start_for(shell, &["-c", "trap '' TERM; sleep 30 & exit 3"])?;
    start.fallback_stop.grace_ms = 3_000;
    harness.start(start.clone()).await?;

    let exited = harness.wait_for_state(|state| state.exit.is_some()).await?;
    let release = daemon::release_monitor(&harness.paths, SUBJECT).await;
    let restart = harness.start(start).await;
    let cleaned = harness
        .wait_for_state(|state| state.cleanup != MonitorCleanup::Pending)
        .await?;
    daemon::release_monitor(&harness.paths, SUBJECT).await?;

    assert_eq!(exited.exit, Some(MonitorExit::Code(3)));
    assert_eq!(exited.cleanup, MonitorCleanup::Pending);
    assert!(matches!(
        release,
        Err(DaemonError::MonitorRejected {
            kind: MonitorErrorKind::NotReleasable,
            ..
        })
    ));
    assert!(matches!(
        restart,
        Err(DaemonError::MonitorUnavailable { .. })
    ));
    assert_eq!(cleaned.cleanup, MonitorCleanup::Complete);

    Ok(())
}

#[tokio::test]
async fn monitor_cleans_up_a_killed_group_whose_member_is_still_exiting() -> Result<()> {
    let harness = Harness::new()?;
    // Tearing down the descendant's resident memory outlasts the small leader's exit, so the
    // fallback stop starts while a member can no longer take a signal yet is no zombie.
    let fake = pv_fake::install_with_settings(
        &harness.paths.root().join("runtime/bin/runtime"),
        Persona::LongRunning,
        FakeSettings {
            descendant: true,
            descendant_resident_mib: 256,
            ..FakeSettings::default()
        },
    )?;
    let started = harness
        .start(harness.start_for(fake.executable(), &[])?)
        .await?;
    wait_until(|| {
        Ok(fake
            .events()?
            .iter()
            .any(|event| matches!(event.kind, EventKind::DescendantResident)))
    })
    .await?;

    kill_process_group(pid_of(started.runtime_pid)?, Signal::KILL)?;
    let cleaned = harness
        .wait_for_state(|state| state.cleanup != MonitorCleanup::Pending)
        .await?;
    let release = daemon::release_monitor(&harness.paths, SUBJECT).await;

    assert_eq!(cleaned.exit, Some(MonitorExit::Signal(9)));
    assert_eq!(cleaned.cleanup, MonitorCleanup::Complete);
    release?;

    Ok(())
}

#[tokio::test]
async fn monitor_keeps_its_records_when_group_cleanup_is_unproven() -> Result<()> {
    let harness = Harness::with_hooks(MonitorHooks {
        unknown_group_check: true,
        ..MonitorHooks::default()
    })?;
    let fake = harness.fake(Persona::LongRunning)?;
    let started = harness
        .start(harness.start_for(fake.executable(), &[])?)
        .await?;
    wait_for_fake_start(&fake).await?;

    let stopped = daemon::stop_monitor(&harness.paths, SUBJECT, STOP).await?;
    let release = daemon::release_monitor(&harness.paths, SUBJECT).await;
    let records_kept = state::fs::path_exists(&harness.monitor_dir().join("monitor.json"));
    kill(started.monitor_pid, Signal::KILL)?;
    wait_until(|| harness.reservation_is_free()).await?;
    daemon::recover_monitor(&harness.paths, SUBJECT).await?;

    assert_eq!(stopped.exit, Some(MonitorExit::Code(0)));
    assert_eq!(stopped.cleanup, MonitorCleanup::Unproven);
    assert!(matches!(
        release,
        Err(DaemonError::MonitorRejected {
            kind: MonitorErrorKind::NotReleasable,
            ..
        })
    ));
    assert!(records_kept);
    assert!(!state::fs::path_exists(&harness.monitor_dir()));

    Ok(())
}

#[tokio::test]
async fn monitor_death_before_publication_starts_nothing() -> Result<()> {
    let (harness, fake, started) = kill_monitor_at(MonitorPause::BeforePublication).await?;

    assert!(
        matches!(started, Err(DaemonError::MonitorStartupFailed { .. })),
        "{started:?}"
    );
    assert!(!state::fs::path_exists(&harness.monitor_dir()));
    assert!(fake.events()?.is_empty());

    Ok(())
}

#[tokio::test]
async fn monitor_death_after_publication_is_recovered() -> Result<()> {
    let (harness, fake, started) = kill_monitor_at(MonitorPause::BeforePermission).await?;

    assert!(
        matches!(started, Err(DaemonError::MonitorStartupFailed { .. })),
        "{started:?}"
    );
    assert!(!state::fs::path_exists(&harness.monitor_dir()));
    assert!(fake.events()?.is_empty());

    Ok(())
}

#[tokio::test]
async fn monitor_death_after_permission_is_recovered() -> Result<()> {
    let (harness, fake, started) = kill_monitor_at(MonitorPause::GateAfterPermission).await?;

    assert!(
        matches!(started, Err(DaemonError::MonitorStartupFailed { .. })),
        "{started:?}"
    );
    assert!(!state::fs::path_exists(&harness.monitor_dir()));
    assert!(fake.events()?.is_empty());

    Ok(())
}

#[tokio::test]
async fn recovery_refuses_a_live_monitor_and_stops_a_killed_monitors_runtime() -> Result<()> {
    let harness = Harness::new()?;
    let fake = harness.fake(Persona::LongRunning)?;
    let started = harness
        .start(harness.start_for(fake.executable(), &[])?)
        .await?;
    wait_for_fake_start(&fake).await?;

    kill(started.monitor_pid, Signal::STOP)?;
    let refused = daemon::recover_monitor(&harness.paths, SUBJECT).await;
    kill(started.monitor_pid, Signal::KILL)?;
    wait_until(|| harness.reservation_is_free()).await?;
    daemon::recover_monitor(&harness.paths, SUBJECT).await?;

    assert!(matches!(
        refused,
        Err(DaemonError::MonitorUnavailable { .. })
    ));
    // launchd reaps the orphaned runtime.
    wait_until(|| Ok(!process_exists(started.runtime_pid)?)).await?;

    assert!(
        fake.events()?.iter().any(
            |event| matches!(&event.kind, EventKind::Signal { signal } if signal == "SIGTERM")
        )
    );
    assert!(!state::fs::path_exists(&harness.monitor_dir()));

    Ok(())
}

#[tokio::test]
async fn recovery_never_signals_a_changed_foreign_or_previous_boot_runtime() -> Result<()> {
    let harness = Harness::new()?;
    let mut bystander = ChildGuard(
        TestCommand::new("/bin/sleep")
            .arg("30")
            .process_group(0)
            .spawn()?,
    );
    let pid = bystander.0.id();
    let birth = platform::inspect_process_start_identity(pid)?
        .ok_or_else(|| anyhow!("bystander {pid} has no identity"))?;
    let current_boot = String::from(platform::current_boot_session_id()?);
    let changed_birth = serde_json::json!({
        "seconds": 1,
        "microseconds": 0,
    });

    // The exact runtime identity, but recorded during another boot.
    harness.write_record(
        1,
        pid,
        &serde_json::json!(birth),
        "00000000-0000-0000-0000-000000000001",
    )?;
    let previous_boot_recovery = daemon::recover_monitor(&harness.paths, SUBJECT).await;
    let previous_boot_records = state::fs::path_exists(&harness.monitor_dir());
    // The exact runtime identity from this boot, but written by another protocol version.
    harness.write_record(2, pid, &serde_json::json!(birth), &current_boot)?;
    let foreign_recovery = daemon::recover_monitor(&harness.paths, SUBJECT).await;
    let foreign_records = state::fs::path_exists(&harness.monitor_dir().join("monitor.json"));
    harness.write_record(1, pid, &changed_birth, &current_boot)?;
    let changed_recovery = daemon::recover_monitor(&harness.paths, SUBJECT).await;

    assert!(previous_boot_recovery.is_ok());
    assert!(!previous_boot_records);
    assert!(matches!(
        foreign_recovery,
        Err(DaemonError::InvalidRuntimeRecord { .. })
    ));
    assert!(foreign_records);
    assert!(matches!(
        changed_recovery,
        Err(DaemonError::RuntimeProcessIdentityChanged { pid: refused }) if refused == pid
    ));
    assert!(bystander.0.try_wait()?.is_none());

    Ok(())
}

#[tokio::test]
async fn concurrent_spawns_never_inherit_the_reservation() -> Result<()> {
    let harness = Harness::with_hooks(MonitorHooks {
        pauses: vec![MonitorPause::BeforePublication],
        ..MonitorHooks::default()
    })?;
    let fake = harness.fake(Persona::LongRunning)?;
    let start = harness.start_for(fake.executable(), &[])?;

    let (started, bystander) = tokio::join!(harness.start(start), async {
        harness.reached(MonitorPause::BeforePublication).await?;
        // Spawned while the controller holds the reservation and the monitor's startup is open.
        let bystander = ChildGuard(TestCommand::new("/bin/sleep").arg("30").spawn()?);
        harness.resume(MonitorPause::BeforePublication)?;
        anyhow::Ok(bystander)
    });
    let _bystander = bystander?;
    started?;
    daemon::stop_monitor(&harness.paths, SUBJECT, STOP).await?;
    // Release waits for the reservation, which a bystander holding it would block.
    daemon::release_monitor(&harness.paths, SUBJECT).await?;

    assert!(harness.reservation_is_free()?);

    Ok(())
}

#[tokio::test]
async fn closed_lifeline_stops_the_runtime_and_releases_the_monitor() -> Result<()> {
    let harness = Harness::new()?;
    let executable = harness.paths.root().join("runtime/bin/runtime");
    // The fake's own lifeline is off, so only the monitor can stop it.
    let fake = pv_fake::install_with(
        &pv_fake::binary()?,
        &executable,
        &Scenario {
            persona: Persona::LongRunning,
            lifeline_fd: None,
            settings: FakeSettings::default(),
        },
    )?;
    let (read, write) = rustix::pipe::pipe()?;
    fcntl_setfd(&write, FdFlags::CLOEXEC)?;
    let mut start = harness.start_for(fake.executable(), &[])?;
    start.lifeline_fd = Some(read.as_raw_fd());
    let started = harness.start(start).await?;
    drop(read);
    wait_for_fake_start(&fake).await?;

    // The controller dies.
    drop::<OwnedFd>(write);
    wait_until(|| Ok(!state::fs::path_exists(&harness.monitor_dir()))).await?;
    wait_until(|| harness.reservation_is_free()).await?;

    assert!(
        fake.events()?.iter().any(
            |event| matches!(&event.kind, EventKind::Signal { signal } if signal == "SIGTERM")
        )
    );
    assert!(!process_exists(started.runtime_pid)?);

    Ok(())
}

#[tokio::test]
async fn overlong_socket_paths_are_rejected_before_anything_starts() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("a".repeat(80)));
    let start = Harness::new()?.start_for(Utf8Path::new("/bin/sh"), &[])?;

    let started = daemon::start_monitor(&paths, Utf8Path::new("/nonexistent"), start).await;

    assert!(matches!(
        started,
        Err(DaemonError::MonitorSocketPathTooLong { limit: 103, .. })
    ));
    assert!(!state::fs::path_exists(&paths.monitors()));

    Ok(())
}

#[tokio::test]
async fn symlinked_logs_are_rejected_before_the_runtime_starts() -> Result<()> {
    let harness = Harness::new()?;
    let fake = harness.fake(Persona::LongRunning)?;
    let target = harness.tempdir.path().join("elsewhere.log");
    state::fs::write_sensitive_file(&target, "")?;
    let start = harness.start_for(fake.executable(), &[])?;
    symlink(&target, &start.log_path)?;

    let started = harness.start(start).await;

    assert!(matches!(
        started,
        Err(DaemonError::MonitorStartupFailed { .. })
    ));
    assert_eq!(state::fs::read_to_string(&target)?, "");
    assert!(fake.events()?.is_empty());
    assert!(!state::fs::path_exists(&harness.monitor_dir()));

    Ok(())
}

#[tokio::test]
async fn held_rotation_still_lets_the_monitor_exit() -> Result<()> {
    let harness = Harness::with_hooks(MonitorHooks {
        pauses: vec![MonitorPause::RotationCopy],
        rotation_bytes: Some(1),
        rotation_interval_ms: Some(20),
        ..MonitorHooks::default()
    })?;
    let shell = Utf8Path::new("/bin/sh");
    harness
        .start(harness.start_for(shell, &["-c", "echo started; while :; do sleep 0.1; done"])?)
        .await?;

    harness.reached(MonitorPause::RotationCopy).await?;
    daemon::stop_monitor(&harness.paths, SUBJECT, STOP).await?;
    daemon::release_monitor(&harness.paths, SUBJECT).await?;

    assert!(harness.reservation_is_free()?);
    assert!(!state::fs::path_exists(&harness.monitor_dir()));

    Ok(())
}

/// Kills the monitor once it pauses at `point`, then returns what the controller saw.
async fn kill_monitor_at(
    point: MonitorPause,
) -> Result<(Harness, InstalledFake, Result<MonitorState, DaemonError>)> {
    let harness = Harness::with_hooks(MonitorHooks {
        pauses: vec![point],
        ..MonitorHooks::default()
    })?;
    let fake = harness.fake(Persona::LongRunning)?;
    let start = harness.start_for(fake.executable(), &[])?;

    let (started, killed) = tokio::join!(harness.start(start), async {
        let paused = harness.reached(point).await?;
        let monitor = if point == MonitorPause::GateAfterPermission {
            harness.record()?["monitor"]["pid"]
                .as_u64()
                .and_then(|pid| u32::try_from(pid).ok())
                .ok_or_else(|| anyhow!("record has no monitor pid"))?
        } else {
            paused
        };
        kill(monitor, Signal::KILL)
    });
    killed?;

    Ok((harness, fake, started))
}

struct Harness {
    tempdir: Utf8TempDir,
    paths: PvPaths,
    monitor: Utf8PathBuf,
}

impl Harness {
    fn new() -> Result<Self> {
        Self::with_hooks(MonitorHooks::default())
    }

    fn with_hooks(mut hooks: MonitorHooks) -> Result<Self> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        state::fs::ensure_layout(&paths)?;
        let pauses = tempdir.path().join("pauses");
        state::fs::ensure_user_dir(&pauses)?;
        hooks.pause_dir = Some(pauses);
        state::fs::write_sensitive_file(
            &paths.home().join("pv-monitor-hooks.json"),
            &serde_json::to_string(&hooks)?,
        )?;

        Ok(Self {
            tempdir,
            paths,
            monitor: pv_fake::example_binary("pv-monitor")?,
        })
    }

    fn start_for(&self, command: &Utf8Path, arguments: &[&str]) -> Result<MonitorStart> {
        Ok(MonitorStart {
            subject: SUBJECT.to_owned(),
            command: command.to_owned(),
            arguments: arguments.iter().map(ToString::to_string).collect(),
            private_environment: BTreeMap::new(),
            log_path: self.paths.logs().join("runtime.log"),
            fallback_stop: FALLBACK,
            lifeline_fd: Some(pv_fake::lifeline_fd()?),
        })
    }

    fn fake(&self, persona: Persona) -> Result<InstalledFake> {
        pv_fake::install(&self.paths.root().join("runtime/bin/runtime"), persona)
    }

    async fn start(&self, start: MonitorStart) -> Result<MonitorState, DaemonError> {
        daemon::start_monitor(&self.paths, &self.monitor, start).await
    }

    fn monitor_dir(&self) -> Utf8PathBuf {
        self.paths.monitor_dir(SUBJECT)
    }

    fn record(&self) -> Result<serde_json::Value> {
        let record = state::fs::read_to_string(&self.monitor_dir().join("monitor.json"))?;

        Ok(serde_json::from_str(&record)?)
    }

    fn instance(&self) -> Result<String> {
        self.record()?["instance"]
            .as_str()
            .map(ToOwned::to_owned)
            .ok_or_else(|| anyhow!("record has no instance"))
    }

    /// Writes a record as a dead monitor of protocol `version` would have left it, naming `pid`
    /// as its runtime.
    fn write_record(
        &self,
        version: u32,
        pid: u32,
        start_identity: &serde_json::Value,
        boot_session_id: &str,
    ) -> Result<()> {
        let record = serde_json::json!({
            "version": version,
            "subject": SUBJECT,
            "instance": "0123456789abcdef0123456789abcdef",
            "boot_session_id": boot_session_id,
            "monitor": { "pid": pid, "start_identity": start_identity },
            "runtime": { "pid": pid, "start_identity": start_identity },
            "command": "/bin/sleep",
            "arguments": ["30"],
            "log_path": self.paths.logs().join("runtime.log"),
            "fallback_stop": { "signal": "terminate", "grace_ms": 1_000 },
        });
        state::fs::write_sensitive_file(
            &self.monitor_dir().join("monitor.json"),
            &serde_json::to_string_pretty(&record)?,
        )?;

        Ok(())
    }

    async fn raw_request(&self, line: &str) -> Result<String> {
        let stream = UnixStream::connect(self.monitor_dir().join("sock")).await?;
        let (read, mut write) = stream.into_split();
        write.write_all(format!("{line}\n").as_bytes()).await?;
        let mut reply = String::new();
        BufReader::new(read).read_line(&mut reply).await?;

        Ok(reply.trim_end().to_owned())
    }

    async fn wait_for_state(&self, done: impl Fn(&MonitorState) -> bool) -> Result<MonitorState> {
        for _attempt in 0..WAIT_ATTEMPTS {
            let state = daemon::monitor_state(&self.paths, SUBJECT).await?;
            if done(&state) {
                return Ok(state);
            }
            sleep(POLL_INTERVAL).await;
        }

        Err(anyhow!("monitor never reached the expected state"))
    }

    async fn wait_for_log(&self, line: &str) -> Result<()> {
        let log = self.paths.logs().join("runtime.log");
        wait_until(|| {
            Ok(state::fs::read_to_string(&log)
                .is_ok_and(|content| content.lines().any(|logged| logged == line)))
        })
        .await
    }

    /// Waits for the monitor or gate to pause at `point`, and returns the paused pid.
    async fn reached(&self, point: MonitorPause) -> Result<u32> {
        let reached = self.pause_file(point, "reached")?;
        wait_until(|| Ok(state::fs::path_exists(&reached))).await?;

        Ok(state::fs::read_to_string(&reached)?.trim().parse()?)
    }

    fn resume(&self, point: MonitorPause) -> Result<()> {
        state::fs::write_sensitive_file(&self.pause_file(point, "continue")?, "")?;

        Ok(())
    }

    fn pause_file(&self, point: MonitorPause, suffix: &str) -> Result<Utf8PathBuf> {
        let name = serde_json::to_value(point)?
            .as_str()
            .map(ToOwned::to_owned)
            .ok_or_else(|| anyhow!("pause point is not a string"))?;

        Ok(self.tempdir.path().join(format!("pauses/{name}.{suffix}")))
    }

    fn reservation_is_free(&self) -> Result<bool> {
        match MonitorReservation::acquire(&self.paths, SUBJECT) {
            Ok(_reservation) => Ok(true),
            Err(StateError::CoordinationLockHeld { .. }) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    fn snapshot(&self, assertion: impl FnOnce()) {
        let mut settings = Settings::clone_current();
        settings.add_filter(self.tempdir.path().as_str(), "<tempdir>");
        settings.add_filter(r#""(pid|seconds|microseconds)": \d+"#, r#""$1": <number>"#);
        settings.add_filter(
            r#""instance": "[0-9a-f]{32}""#,
            r#""instance": "<instance>""#,
        );
        settings.add_filter(
            r#""boot_session_id": "[0-9A-F-]{36}""#,
            r#""boot_session_id": "<boot>""#,
        );

        settings.bind(assertion)
    }
}

/// Waits for the fake to record its start, and returns its pid.
async fn wait_for_fake_start(fake: &InstalledFake) -> Result<u32> {
    for _attempt in 0..WAIT_ATTEMPTS {
        let started = fake
            .events()?
            .into_iter()
            .find(|event| matches!(event.kind, EventKind::Started { .. }));
        if let Some(event) = started {
            return Ok(u32::try_from(event.pid)?);
        }
        sleep(POLL_INTERVAL).await;
    }

    Err(anyhow!("fake {} never started", fake.executable()))
}

async fn wait_until(done: impl Fn() -> Result<bool>) -> Result<()> {
    for _attempt in 0..WAIT_ATTEMPTS {
        if done()? {
            return Ok(());
        }
        sleep(POLL_INTERVAL).await;
    }

    Err(anyhow!("condition never held"))
}

fn kill(pid: u32, signal: Signal) -> Result<()> {
    kill_process(pid_of(pid)?, signal)?;

    Ok(())
}

fn process_exists(pid: u32) -> Result<bool> {
    match test_kill_process(pid_of(pid)?) {
        Ok(()) => Ok(true),
        Err(rustix::io::Errno::SRCH) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn pid_of(pid: u32) -> Result<Pid> {
    Pid::from_raw(i32::try_from(pid)?).ok_or_else(|| anyhow!("pid {pid} is not positive"))
}

#[expect(
    clippy::disallowed_methods,
    reason = "monitor tests plant a symlinked log the monitor must refuse"
)]
fn symlink(target: &Utf8Path, link: &Utf8Path) -> Result<()> {
    std::os::unix::fs::symlink(target, link)?;

    Ok(())
}

struct ChildGuard(std::process::Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _kill_result = self.0.kill();
        let _wait_result = self.0.wait();
    }
}
