use std::cell::RefCell;
use std::ffi::OsString;
use std::io::{self, BufRead, BufReader, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use cli::{Environment, run_with_environment};
use insta::assert_debug_snapshot;
use platform::{LAUNCH_AGENT_LABEL, LaunchAgentConfig};
use serde_json::json;
use state::{PvPaths, StateError};

#[path = "support/runtime.rs"]
mod runtime;
use runtime::{ForeignMonitorRecord, RuntimeFixture, gateway_spec};

#[expect(
    clippy::disallowed_types,
    reason = "disable tests spawn unrelated processes to prove what disable never signals"
)]
type TestCommand = std::process::Command;

struct ChildGuard(std::process::Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _kill_result = self.0.kill();
        let _wait_result = self.0.wait();
    }
}

#[derive(Debug)]
struct TestEnvironment {
    home: PathBuf,
    current_dir: RefCell<PathBuf>,
    current_exe: PathBuf,
    launch_agent_path: PathBuf,
    operations: RefCell<Vec<String>>,
    bootout_error: Option<BootoutError>,
    bootout_daemon_record: Option<(Utf8PathBuf, String)>,
    bootout_signal: Option<mpsc::Sender<()>>,
    bootstrap_requires_daemon_exit: Option<u32>,
}

impl TestEnvironment {
    fn new(
        home: &Utf8Path,
        current_dir: &Utf8Path,
        current_exe: &Utf8Path,
        launch_agent_path: &Utf8Path,
    ) -> Self {
        Self {
            home: home.as_std_path().to_path_buf(),
            current_dir: RefCell::new(current_dir.as_std_path().to_path_buf()),
            current_exe: current_exe.as_std_path().to_path_buf(),
            launch_agent_path: launch_agent_path.as_std_path().to_path_buf(),
            operations: RefCell::new(Vec::new()),
            bootout_error: None,
            bootout_daemon_record: None,
            bootout_signal: None,
            bootstrap_requires_daemon_exit: None,
        }
    }

    fn with_unloaded_launch_agent(mut self) -> Self {
        self.bootout_error = Some(BootoutError::AlreadyUnloaded);
        self
    }

    fn with_bootout_exit_status_five(mut self) -> Self {
        self.bootout_error = Some(BootoutError::ExitStatusFive);
        self
    }

    fn operations(&self) -> Vec<String> {
        self.operations.borrow().clone()
    }
}

#[derive(Clone, Copy, Debug)]
enum BootoutError {
    AlreadyUnloaded,
    ExitStatusFive,
}

impl Environment for TestEnvironment {
    fn daemon_process_for_stop(
        &self,
        paths: &PvPaths,
    ) -> Result<Option<daemon::DaemonProcess>, daemon::DaemonError> {
        // Protocol fixtures serve replies in this test process. Native process
        // fixtures publish a record and exercise the real ownership checks.
        if state::fs::path_entry_exists(&paths.daemon_process_record())? {
            daemon::daemon_process_for_stop(paths)
        } else {
            Ok(None)
        }
    }

    fn inspect_low_ports(&self) -> Result<platform::LowPortInspection, platform::PlatformError> {
        Err(platform::PlatformError::PrivilegedHelperUnavailable)
    }

    fn var_os(&self, _key: &str) -> Option<OsString> {
        None
    }

    fn home_dir(&self) -> Option<PathBuf> {
        Some(self.home.clone())
    }

    fn current_dir(&self) -> io::Result<PathBuf> {
        Ok(self.current_dir.borrow().clone())
    }

    fn current_exe(&self) -> io::Result<PathBuf> {
        Ok(self.current_exe.clone())
    }

    fn stdin_is_terminal(&self) -> bool {
        false
    }

    fn open_url(&self, _url: &str) -> io::Result<()> {
        Ok(())
    }

    fn launch_agent_path(&self) -> PathBuf {
        self.launch_agent_path.clone()
    }

    fn bootstrap_launch_agent(&self, plist_path: &Utf8Path) -> Result<(), platform::PlatformError> {
        if let Some(pid) = self.bootstrap_requires_daemon_exit
            && platform::inspect_process_start_identity(pid)?.is_some()
            && !platform::process_is_zombie(pid)?
        {
            return Err(platform::PlatformError::LaunchAgent(
                "bootstrap attempted before the captured daemon exited".to_owned(),
            ));
        }
        self.operations
            .borrow_mut()
            .push(format!("bootstrap {plist_path}"));

        Ok(())
    }

    fn bootout_launch_agent(&self) -> Result<(), platform::PlatformError> {
        if let Some((path, record)) = &self.bootout_daemon_record {
            state::fs::write_sensitive_file(path, record)
                .map_err(|error| platform::PlatformError::LaunchAgent(error.to_string()))?;
        }
        if let Some(signal) = &self.bootout_signal {
            signal
                .send(())
                .map_err(|error| platform::PlatformError::LaunchAgent(error.to_string()))?;
        }
        self.operations
            .borrow_mut()
            .push(format!("bootout {LAUNCH_AGENT_LABEL}"));
        match self.bootout_error {
            Some(BootoutError::AlreadyUnloaded) => {
                return Err(platform::PlatformError::LaunchAgent(
                    "launch agent is not loaded".to_string(),
                ));
            }
            Some(BootoutError::ExitStatusFive) => {
                return Err(platform::PlatformError::LaunchAgentCommandStatus {
                    command: format!("/bin/launchctl bootout gui/501/{LAUNCH_AGENT_LABEL}"),
                    status: "exit status: 5".to_string(),
                });
            }
            None => {}
        }

        Ok(())
    }

    fn kickstart_launch_agent(&self) -> Result<(), platform::PlatformError> {
        self.operations
            .borrow_mut()
            .push(format!("kickstart {LAUNCH_AGENT_LABEL}"));

        Ok(())
    }
}

#[test]
fn daemon_enable_installs_pv_launch_agent_and_starts_it() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let current_exe = tempdir.path().join("pv");
    let launch_agent_path = tempdir
        .path()
        .join("Library/LaunchAgents/com.prvious.pv.daemon.plist");
    let environment = TestEnvironment::new(&home, &current_dir, &current_exe, &launch_agent_path);
    let paths = PvPaths::for_home(&home);
    let daemon = DaemonFixture::start(&paths, 2)?;

    let output = run_pv(&["daemon:enable"], &environment)?;
    let _daemon_requests = daemon.finish()?;
    let plist = read_required_file(&launch_agent_path)?;
    let parsed = LaunchAgentConfig::parse(&plist);

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stdout.contains("LaunchAgent installed"));
    assert!(output.stderr.is_empty());
    assert_eq!(
        parsed,
        Some(LaunchAgentConfig::new(
            paths.active_pv_binary(),
            paths.logs().join("launchd.out.log"),
            paths.logs().join("launchd.err.log"),
        ))
    );

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((output, environment.operations(), plist));
    });

    Ok(())
}

#[test]
fn daemon_enable_waits_for_health_and_submits_reconciliation() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let current_exe = tempdir.path().join("pv");
    let launch_agent_path = tempdir
        .path()
        .join("Library/LaunchAgents/com.prvious.pv.daemon.plist");
    let environment = TestEnvironment::new(&home, &current_dir, &current_exe, &launch_agent_path);
    let paths = PvPaths::for_home(&home);
    let daemon = DaemonFixture::start(&paths, 2)?;

    let output = run_pv(&["daemon:enable"], &environment)?;
    let daemon_requests = daemon.finish()?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert_eq!(
        daemon_requests,
        vec![
            format!(
                r#"{{"protocol_version":{},"command":"health"}}"#,
                daemon::PROTOCOL_VERSION
            ),
            format!(
                r#"{{"protocol_version":{},"command":"run_job","kind":"reconcile","scope":"system"}}"#,
                daemon::PROTOCOL_VERSION
            ),
        ]
    );

    Ok(())
}

#[test]
fn daemon_restart_replaces_stale_pv_owned_launch_agent() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let current_exe = tempdir.path().join("pv");
    let launch_agent_path = tempdir
        .path()
        .join("Library/LaunchAgents/com.prvious.pv.daemon.plist");
    let environment = TestEnvironment::new(&home, &current_dir, &current_exe, &launch_agent_path);
    let paths = PvPaths::for_home(&home);
    let stale = LaunchAgentConfig::new(
        tempdir.path().join("old-pv"),
        paths.logs().join("launchd.out.log"),
        paths.logs().join("launchd.err.log"),
    );
    write_file(&launch_agent_path, &stale.render()?)?;
    let daemon = DaemonFixture::start(&paths, 2)?;

    let output = run_pv(&["daemon:restart"], &environment)?;
    let _daemon_requests = daemon.finish()?;
    let plist_after_restart = read_required_file(&launch_agent_path)?;
    let parsed = LaunchAgentConfig::parse(&plist_after_restart);

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stdout.contains("Daemon restarted"));
    assert!(output.stderr.is_empty());
    assert_eq!(
        parsed,
        Some(LaunchAgentConfig::new(
            paths.active_pv_binary(),
            paths.logs().join("launchd.out.log"),
            paths.logs().join("launchd.err.log"),
        ))
    );
    assert_eq!(
        environment.operations(),
        vec![
            format!("bootout {LAUNCH_AGENT_LABEL}"),
            format!("bootstrap {launch_agent_path}"),
            format!("kickstart {LAUNCH_AGENT_LABEL}"),
        ]
    );

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((output, environment.operations(), plist_after_restart));
    });

    Ok(())
}

#[test]
fn daemon_disable_removes_only_pv_owned_launch_agent() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let current_exe = tempdir.path().join("pv");
    let launch_agent_path = tempdir
        .path()
        .join("Library/LaunchAgents/com.prvious.pv.daemon.plist");
    let environment = TestEnvironment::new(&home, &current_dir, &current_exe, &launch_agent_path);
    let paths = PvPaths::for_home(&home);
    let config = LaunchAgentConfig::new(
        &current_exe,
        paths.logs().join("launchd.out.log"),
        paths.logs().join("launchd.err.log"),
    );
    write_file(&launch_agent_path, &config.render()?)?;

    let output = run_pv(&["daemon:disable"], &environment)?;
    let plist_after_disable = read_optional_file(&launch_agent_path)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stdout.contains("LaunchAgent removed"));
    assert!(output.stderr.is_empty());
    assert!(plist_after_disable.is_none());

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((output, environment.operations(), plist_after_disable));
    });

    Ok(())
}

#[test]
fn daemon_disable_stops_live_runtimes_without_a_plist_or_database() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(&home);
    let environment = TestEnvironment::new(
        &home,
        tempdir.path(),
        &paths.active_pv_binary(),
        &tempdir.path().join("absent.plist"),
    )
    .with_unloaded_launch_agent();
    let gateway = gateway_spec(&paths);
    let mut worker = gateway.clone();
    worker.name = "worker".to_owned();
    worker.resource_name = "frankenphp".to_owned();
    worker.track = "8.5-test".to_owned();
    worker.command = paths
        .resources()
        .join("frankenphp/8.5/fixture/bin/frankenphp");
    worker.pid_path = paths.worker_pid("8.5-test");
    worker.metadata_path = paths.worker_runtime_metadata("8.5-test");
    let mut resource = gateway.clone();
    resource.name = "mailpit".to_owned();
    resource.resource_name = "mailpit".to_owned();
    resource.track = "1".to_owned();
    resource.command = paths.resources().join("mailpit/1/fixture/bin/mailpit");
    resource.pid_path = paths.resource_pid("mailpit", "1");
    resource.metadata_path = paths.resource_runtime_metadata("mailpit", "1");
    let mut mysql = resource.clone();
    mysql.name = "mysql".to_owned();
    mysql.resource_name = "mysql".to_owned();
    mysql.track = "8.4".to_owned();
    mysql.command = paths.resources().join("mysql/8.4/fixture/bin/mysqld");
    mysql.pid_path = paths.resource_pid("mysql", "8.4");
    mysql.metadata_path = paths.resource_runtime_metadata("mysql", "8.4");
    let mut fixtures = Vec::new();
    for spec in [gateway, worker, resource, mysql] {
        fixtures.push(RuntimeFixture::start(&paths, spec)?);
    }
    // Real MySQL creates this ancillary socket beside its runtime directory.
    let _mysql_socket = UnixListener::bind(paths.run().join("resources/mysql-8.4.sock"))?;
    // A runtime that stopped earlier leaves only its config proofs.
    let retained_proof = paths.resource_runtime_metadata("redis", "7");
    state::fs::write_sensitive_file(&retained_proof, r#"{"staged_config_fingerprint":"a"}"#)?;
    let mut watch = platform::ProcessExitWatch::new(fixtures[0].descendant_pid()?)?;
    // Maintenance stop must work even when the database cannot be opened.
    state::fs::write_sensitive_file(paths.db(), "invalid database")?;
    let output = run_pv(&["daemon:disable"], &environment)?;
    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(!state::fs::path_entry_exists(&retained_proof)?);
    for monitor in state::fs::read_dir_paths(&paths.monitors())? {
        assert!(
            !state::fs::path_is_directory(&monitor)?,
            "monitor {monitor} was not released"
        );
    }
    // The descendant belongs to the fake, not to this test process. Native
    // observation must still report its exact successful status after it exits.
    assert!(
        watch
            .try_exit_status()?
            .is_some_and(|status| status.success())
    );
    for fixture in &mut fixtures {
        assert!(!platform::process_group_has_live_members(fixture.pid()?)?);
        assert!(fixture.records_absent()?);
        fixture.cleanup()?;
    }
    assert_eq!(state::fs::read_to_string(paths.db())?, "invalid database");
    Ok(())
}

#[test]
fn daemon_disable_waits_for_process_death_after_its_socket_is_gone() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(&home);
    let launch_agent_path = tempdir.path().join("owned.plist");
    let environment = TestEnvironment::new(
        &home,
        tempdir.path(),
        &paths.active_pv_binary(),
        &launch_agent_path,
    );
    let config = LaunchAgentConfig::new(
        paths.active_pv_binary(),
        paths.logs().join("out"),
        paths.logs().join("err"),
    );
    write_file(&launch_agent_path, &config.render()?)?;
    let mut daemon_spec = gateway_spec(&paths);
    daemon_spec.command = paths.bin().join("releases/fixture/pv");
    daemon_spec.arguments = vec!["daemon:run".to_owned()];
    daemon_spec.pid_path = paths.run().join("fixture-daemon.pid");
    daemon_spec.metadata_path = paths.run().join("fixture-daemon.json");
    let mut daemon = RuntimeFixture::start(&paths, daemon_spec)?;
    let pid = daemon.pid()?;
    state::fs::write_sensitive_file(
        &paths.daemon_process_record(),
        &serde_json::to_string(&json!({
            "pid": pid,
            "start_identity": platform::inspect_process_start_identity(pid)?,
        }))?,
    )?;
    let mut runtime = RuntimeFixture::start(&paths, gateway_spec(&paths))?;
    let daemon_record = state::fs::read_to_string(&paths.daemon_process_record())?;
    let bootstrap_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    assert!(
        matches!(bootstrap_runtime.block_on(daemon::RunningDaemon::start(paths.clone())),
        Err(daemon::DaemonError::RuntimeCleanupUnproven { pid: recorded_pid, .. }) if recorded_pid == pid)
    );
    assert_eq!(
        state::fs::read_to_string(&paths.daemon_process_record())?,
        daemon_record
    );
    let output = run_pv(&["daemon:disable"], &environment)?;
    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(runtime.records_exist()?);
    assert!(platform::process_group_has_live_members(runtime.pid()?)?);
    assert!(state::fs::path_entry_exists(&launch_agent_path)?);
    daemon.cleanup()?;
    // Once the captured daemon actually exits, its stale record must not block stop.
    let stopped = run_pv(&["daemon:disable"], &environment)?;
    assert_eq!(stopped.exit_code, ExitCode::SUCCESS, "{stopped:?}");
    assert!(runtime.records_absent()?);
    runtime.cleanup()?;
    Ok(())
}

#[test]
fn daemon_disable_keeps_dead_postgres_records_when_cleanup_is_unproven() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(&home);
    let environment = TestEnvironment::new(
        &home,
        tempdir.path(),
        &paths.active_pv_binary(),
        &tempdir.path().join("absent.plist"),
    );
    let mut spec = gateway_spec(&paths);
    spec.resource_name = "postgres".to_owned();
    spec.track = "18".to_owned();
    spec.pid_path = paths.resource_pid("postgres", "18");
    spec.metadata_path = paths.resource_runtime_metadata("postgres", "18");
    let mut runtime = RuntimeFixture::start(&paths, spec)?;
    // Killed, Postgres exits by signal, which proves nothing about its backends.
    runtime.cleanup()?;
    let metadata_path = paths.resource_runtime_metadata("postgres", "18");
    let metadata = state::fs::read_to_string(&metadata_path)?;
    let output = run_pv(&["daemon:disable"], &environment)?;
    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert_eq!(state::fs::read_to_string(&metadata_path)?, metadata);
    assert!(state::fs::path_entry_exists(
        &paths
            .monitor_dir("resources/postgres/18")
            .join("monitor.json")
    )?);
    Ok(())
}

#[test]
fn daemon_disable_removes_previous_boot_postgres_records_without_signalling_reused_pid()
-> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(&home);
    let environment = TestEnvironment::new(
        &home,
        tempdir.path(),
        &paths.active_pv_binary(),
        &tempdir.path().join("absent.plist"),
    );
    // A process outside PV now holds the pid that a previous boot's Postgres recorded.
    let foreign = ChildGuard(
        TestCommand::new("/bin/sleep")
            .arg("30")
            .process_group(0)
            .spawn()?,
    );
    let foreign_pid = foreign.0.id();
    let previous_boot =
        platform::BootSessionId::try_from("00000000-0000-0000-0000-000000000001".to_owned())?;
    assert_ne!(previous_boot, platform::current_boot_session_id()?);
    let pid_path = paths.resource_pid("postgres", "18");
    let metadata_path = paths.resource_runtime_metadata("postgres", "18");
    state::fs::write_sensitive_file(&pid_path, &format!("{foreign_pid}\n"))?;
    let metadata = json!({
        "name": "postgres",
        "pid": foreign_pid,
        "command": "/bin/sleep",
        "arguments": ["30"],
        "resource_name": "postgres",
        "track": "18",
        "log_path": paths.logs().join("postgres.log"),
        "started_at": "2026-10-09T00:00:00Z",
        "boot_session_id": previous_boot,
        "process_start_identity": platform::inspect_process_start_identity(foreign_pid)?,
    });
    state::fs::write_sensitive_file(&metadata_path, &serde_json::to_string(&metadata)?)?;

    let output = run_pv(&["daemon:disable"], &environment)?;
    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(!state::fs::path_entry_exists(&pid_path)?);
    assert!(!state::fs::path_entry_exists(&metadata_path)?);
    assert!(platform::process_group_has_live_members(foreign_pid)?);
    drop(foreign);
    Ok(())
}

#[test]
fn daemon_replacement_waits_for_delayed_exit_before_bootstrap() -> anyhow::Result<()> {
    for publish_during_bootout in [false, true] {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(&home);
        let launch_agent_path = tempdir.path().join("owned.plist");
        let stale = LaunchAgentConfig::new(
            tempdir.path().join("old-pv"),
            paths.logs().join("out"),
            paths.logs().join("err"),
        );
        write_file(&launch_agent_path, &stale.render()?)?;
        let mut spec = gateway_spec(&paths);
        spec.command = paths.bin().join("releases/fixture/pv");
        spec.arguments = vec!["daemon:run".to_owned()];
        spec.pid_path = paths.run().join("fixture-daemon.pid");
        spec.metadata_path = paths.run().join("fixture-daemon.json");
        let mut previous = RuntimeFixture::start(&paths, spec)?;
        let pid = previous.pid()?;
        let record = serde_json::to_string(&json!({
            "pid": pid, "start_identity": platform::inspect_process_start_identity(pid)?,
        }))?;
        let mut environment = TestEnvironment::new(
            &home,
            tempdir.path(),
            &paths.active_pv_binary(),
            &launch_agent_path,
        );
        if publish_during_bootout {
            environment.bootout_daemon_record = Some((paths.daemon_process_record(), record));
        } else {
            state::fs::write_sensitive_file(&paths.daemon_process_record(), &record)?;
        }
        let (signal, receiver) = mpsc::channel();
        let cleanup = thread::Builder::new()
            .name("daemon-fixture-cleanup".to_owned())
            .spawn(move || -> anyhow::Result<()> {
                receiver.recv_timeout(Duration::from_secs(5))?;
                thread::sleep(Duration::from_millis(500));
                previous.cleanup()
            })?;
        environment.bootout_signal = Some(signal);
        environment.bootstrap_requires_daemon_exit = Some(pid);
        let healthy = DaemonFixture::start(&paths, 2)?;

        let output = run_pv(&["daemon:restart"], &environment)?;
        cleanup
            .join()
            .map_err(|_panic| anyhow::anyhow!("daemon fixture cleanup thread failed"))??;
        assert_eq!(
            output.exit_code,
            ExitCode::SUCCESS,
            "publish during bootout: {publish_during_bootout}"
        );
        let _requests = healthy.finish()?;
        assert_eq!(
            environment.operations(),
            [
                format!("bootout {LAUNCH_AGENT_LABEL}"),
                format!("bootstrap {launch_agent_path}"),
                format!("kickstart {LAUNCH_AGENT_LABEL}"),
            ]
        );
    }
    Ok(())
}

#[test]
fn daemon_replacement_refuses_to_bootstrap_while_previous_daemon_is_alive() -> anyhow::Result<()> {
    for command in ["daemon:enable", "daemon:restart"] {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(&home);
        let launch_agent_path = tempdir.path().join("owned.plist");
        let environment = TestEnvironment::new(
            &home,
            tempdir.path(),
            &paths.active_pv_binary(),
            &launch_agent_path,
        );
        let stale = LaunchAgentConfig::new(
            tempdir.path().join("old-pv"),
            paths.logs().join("out"),
            paths.logs().join("err"),
        );
        let original_plist = stale.render()?;
        write_file(&launch_agent_path, &original_plist)?;
        let mut spec = gateway_spec(&paths);
        spec.command = paths.bin().join("releases/fixture/pv");
        spec.arguments = vec!["daemon:run".to_owned()];
        spec.pid_path = paths.run().join("fixture-daemon.pid");
        spec.metadata_path = paths.run().join("fixture-daemon.json");
        let mut daemon = RuntimeFixture::start(&paths, spec)?;
        let pid = daemon.pid()?;
        let record = serde_json::to_string(&json!({
            "pid": pid, "start_identity": platform::inspect_process_start_identity(pid)?,
        }))?;
        state::fs::write_sensitive_file(&paths.daemon_process_record(), &record)?;

        let output = run_pv(&[command], &environment)?;
        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert_eq!(
            environment.operations(),
            [format!("bootout {LAUNCH_AGENT_LABEL}")]
        );
        assert_eq!(read_required_file(&launch_agent_path)?, original_plist);
        assert_eq!(read_required_file(&paths.daemon_process_record())?, record);
        daemon.cleanup()?;
    }
    Ok(())
}

#[test]
fn daemon_disable_keeps_foreign_runtime_records_and_owned_plist() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(&home);
    let launch_agent_path = tempdir.path().join("owned.plist");
    let environment = TestEnvironment::new(
        &home,
        tempdir.path(),
        &paths.active_pv_binary(),
        &launch_agent_path,
    );
    let config = LaunchAgentConfig::new(
        paths.active_pv_binary(),
        paths.logs().join("out"),
        paths.logs().join("err"),
    );
    write_file(&launch_agent_path, &config.render()?)?;
    let mut fixture = RuntimeFixture::start(&paths, gateway_spec(&paths))?;
    let monitor_record = ForeignMonitorRecord::write(&paths, "gateway")?;
    let output = run_pv(&["daemon:disable"], &environment)?;
    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(fixture.records_exist()?);
    assert!(platform::process_group_has_live_members(fixture.pid()?)?);
    assert!(state::fs::path_entry_exists(&launch_agent_path)?);
    monitor_record.restore()?;
    fixture.cleanup()?;
    Ok(())
}

#[test]
fn lifecycle_admission_blocks_start_and_disable_before_mutation() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(&home);
    let environment = TestEnvironment::new(
        &home,
        tempdir.path(),
        &paths.active_pv_binary(),
        &tempdir.path().join("absent.plist"),
    );
    let admission = state::RuntimeLifecycleLock::acquire_exclusive(&paths)?;
    for arguments in [
        &["setup", "--no-path"][..],
        &["daemon:enable"][..],
        &["daemon:restart"][..],
        &["update"][..],
        &["daemon:disable"][..],
    ] {
        assert_eq!(
            run_pv(arguments, &environment)?.exit_code,
            ExitCode::FAILURE
        );
        assert!(!state::fs::path_entry_exists(paths.root())?);
        assert!(environment.operations().is_empty());
    }
    drop(admission);
    let _mutation = state::RuntimeLifecycleLock::acquire_shared(&paths)?;
    assert_eq!(
        run_pv(&["daemon:disable"], &environment)?.exit_code,
        ExitCode::FAILURE
    );
    assert!(environment.operations().is_empty());
    Ok(())
}

#[test]
fn daemon_disable_removes_pv_owned_plist_when_launch_agent_is_already_unloaded()
-> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let current_exe = tempdir.path().join("pv");
    let launch_agent_path = tempdir
        .path()
        .join("Library/LaunchAgents/com.prvious.pv.daemon.plist");
    let environment = TestEnvironment::new(&home, &current_dir, &current_exe, &launch_agent_path)
        .with_unloaded_launch_agent();
    let paths = PvPaths::for_home(&home);
    let config = LaunchAgentConfig::new(
        &current_exe,
        paths.logs().join("launchd.out.log"),
        paths.logs().join("launchd.err.log"),
    );
    write_file(&launch_agent_path, &config.render()?)?;

    let output = run_pv(&["daemon:disable"], &environment)?;
    let plist_after_disable = read_optional_file(&launch_agent_path)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stdout.contains("LaunchAgent removed"));
    assert!(output.stderr.is_empty());
    assert!(plist_after_disable.is_none());

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((output, environment.operations(), plist_after_disable));
    });

    Ok(())
}

#[test]
fn daemon_disable_attempts_bootout_when_launch_agent_plist_is_missing() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let current_exe = tempdir.path().join("pv");
    let launch_agent_path = tempdir
        .path()
        .join("Library/LaunchAgents/com.prvious.pv.daemon.plist");
    let environment = TestEnvironment::new(&home, &current_dir, &current_exe, &launch_agent_path);

    let output = run_pv(&["daemon:disable"], &environment)?;
    let plist_after_disable = read_optional_file(&launch_agent_path)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stdout.contains("LaunchAgent already absent"));
    assert!(output.stderr.is_empty());
    assert_eq!(
        environment.operations(),
        vec![format!("bootout {LAUNCH_AGENT_LABEL}")]
    );
    assert!(plist_after_disable.is_none());

    Ok(())
}

#[test]
fn daemon_disable_does_not_suppress_status_five_without_unloaded_message() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let current_exe = tempdir.path().join("pv");
    let launch_agent_path = tempdir
        .path()
        .join("Library/LaunchAgents/com.prvious.pv.daemon.plist");
    let environment = TestEnvironment::new(&home, &current_dir, &current_exe, &launch_agent_path)
        .with_bootout_exit_status_five();
    let paths = PvPaths::for_home(&home);
    let config = LaunchAgentConfig::new(
        &current_exe,
        paths.logs().join("launchd.out.log"),
        paths.logs().join("launchd.err.log"),
    );
    write_file(&launch_agent_path, &config.render()?)?;

    let output = run_pv(&["daemon:disable"], &environment)?;
    let plist_after_disable = read_optional_file(&launch_agent_path)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stdout.is_empty());
    assert!(output.stderr.contains("exit status: 5"));
    assert!(plist_after_disable.is_some());

    Ok(())
}

#[test]
fn daemon_disable_refuses_non_pv_owned_launch_agent() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let current_exe = tempdir.path().join("pv");
    let launch_agent_path = tempdir
        .path()
        .join("Library/LaunchAgents/com.prvious.pv.daemon.plist");
    let environment = TestEnvironment::new(&home, &current_dir, &current_exe, &launch_agent_path);
    let conflict =
        "<plist><dict><key>Label</key><string>com.prvious.pv.daemon</string></dict></plist>\n";
    write_file(&launch_agent_path, conflict)?;

    let output = run_pv(&["daemon:disable"], &environment)?;
    let plist_after_disable = read_required_file(&launch_agent_path)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stdout.is_empty());
    assert!(output.stderr.contains("not PV-owned"));
    assert_eq!(plist_after_disable, conflict);

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((output, environment.operations(), plist_after_disable));
    });

    Ok(())
}

#[derive(Debug)]
struct RunOutput {
    exit_code: ExitCode,
    stdout: String,
    stderr: String,
}

fn run_pv(args: &[&str], environment: &impl Environment) -> anyhow::Result<RunOutput> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let args = std::iter::once("pv").chain(args.iter().copied());
    let exit_code = run_with_environment(args, environment, &mut stdout, &mut stderr)?;

    Ok(RunOutput {
        exit_code,
        stdout: String::from_utf8(stdout)?,
        stderr: String::from_utf8(stderr)?,
    })
}

fn read_required_file(path: &Utf8Path) -> anyhow::Result<String> {
    read_optional_file(path)?
        .ok_or_else(|| anyhow::anyhow!("expected fixture file to exist: {path}"))
}

fn read_optional_file(path: &Utf8Path) -> anyhow::Result<Option<String>> {
    match state::fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

fn write_file(path: &Utf8Path, content: &str) -> anyhow::Result<()> {
    state::fs::write_sensitive_file(path, content)?;

    Ok(())
}

#[derive(Debug)]
struct DaemonFixture {
    _fixture_lock: MutexGuard<'static, ()>,
    requests: Arc<Mutex<Vec<String>>>,
    thread: thread::JoinHandle<anyhow::Result<()>>,
}

impl DaemonFixture {
    fn start(paths: &PvPaths, expected_requests: usize) -> anyhow::Result<Self> {
        let fixture_lock = daemon_fixture_lock();
        state::fs::ensure_layout(paths)?;
        delete_optional_file(&paths.daemon_socket())?;
        let listener = UnixListener::bind(paths.daemon_socket().as_std_path())?;

        listener.set_nonblocking(true)?;

        let requests = Arc::new(Mutex::new(Vec::new()));
        let thread_requests = Arc::clone(&requests);
        let thread = spawn_daemon_fixture_thread(move || {
            for _request_index in 0..expected_requests {
                let (mut stream, _address) = accept_with_timeout(&listener)?;
                let mut request = String::new();
                let mut reader = BufReader::new(stream.try_clone()?);

                reader.read_line(&mut request)?;
                lock(&thread_requests).push(request.trim().to_string());
                if request.contains(r#""command":"health""#) {
                    write_daemon_line(
                        &mut stream,
                        json!({
                            "type": "response",
                            "protocol_version": daemon::PROTOCOL_VERSION,
                            "status": "ok",
                            "message": "daemon healthy",
                        }),
                    )?;
                } else {
                    write_daemon_line(
                        &mut stream,
                        json!({
                            "type": "response",
                            "protocol_version": daemon::PROTOCOL_VERSION,
                            "status": "accepted",
                            "message": "job accepted",
                            "job_id": "job_enable_1",
                        }),
                    )?;
                }
            }

            Ok(())
        });

        Ok(Self {
            _fixture_lock: fixture_lock,
            requests,
            thread,
        })
    }

    fn finish(self) -> anyhow::Result<Vec<String>> {
        self.thread
            .join()
            .map_err(|_error| anyhow::anyhow!("daemon fixture thread panicked"))??;

        Ok(lock(&self.requests).clone())
    }
}

fn daemon_fixture_lock() -> MutexGuard<'static, ()> {
    static DAEMON_FIXTURE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    lock(DAEMON_FIXTURE_LOCK.get_or_init(|| Mutex::new(())))
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI daemon tests run a synchronous fixture daemon on a short-lived thread"
)]
fn spawn_daemon_fixture_thread(
    operation: impl FnOnce() -> anyhow::Result<()> + Send + 'static,
) -> thread::JoinHandle<anyhow::Result<()>> {
    thread::spawn(operation)
}

fn accept_with_timeout(
    listener: &UnixListener,
) -> anyhow::Result<(UnixStream, std::os::unix::net::SocketAddr)> {
    let started_at = Instant::now();

    loop {
        match listener.accept() {
            Ok((stream, address)) => {
                stream.set_nonblocking(false)?;

                return Ok((stream, address));
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if started_at.elapsed() > Duration::from_secs(3) {
                    return Err(anyhow::anyhow!(
                        "timed out waiting for daemon client request"
                    ));
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn write_daemon_line(stream: &mut UnixStream, value: serde_json::Value) -> anyhow::Result<()> {
    writeln!(stream, "{value}")?;

    Ok(())
}

fn delete_optional_file(path: &Utf8Path) -> anyhow::Result<()> {
    match state::fs::delete_file(path) {
        Ok(()) => Ok(()),
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn with_normalized_tempdir(tempdir: &Utf8Path, assertion: impl FnOnce()) {
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(assertion);
}
