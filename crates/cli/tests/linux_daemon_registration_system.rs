#![cfg(target_os = "linux")]

use std::io::{self, Write as _};
#[expect(
    clippy::disallowed_types,
    reason = "the opt-in system test arms and cancels its independent native cleanup timer"
)]
type SystemTestCommand = std::process::Command;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, ensure};
use camino::{Utf8Path, Utf8PathBuf};
use cli::{Environment, ProcessEnvironment, run_with_environment};
use platform::{DaemonRegistrationConfig, DaemonRegistrationStatus, PlatformError};
use pv_fake::{FakeSettings, Persona, Scenario};
use serde_json::Value;
use state::{
    Database, JobStatus, LinkProjectInput, MonitorReservation, PortRequest, ProjectMode, PvPaths,
    fs,
};

#[test]
#[ignore = "requires PV_LINUX_SYSTEM_TESTS=1, PV_LINUX_SYSTEM_PV and a disposable user's systemd manager"]
fn systemd_user_registration_preserves_monitors_and_refuses_foreign_units() -> Result<()> {
    let environment = ProcessEnvironment;
    if environment.var_os("PV_LINUX_SYSTEM_TESTS").as_deref() != Some("1".as_ref()) {
        return Ok(());
    }
    let binary = environment
        .var_os("PV_LINUX_SYSTEM_PV")
        .ok_or_else(|| anyhow!("PV_LINUX_SYSTEM_PV must name the built pv executable"))?;
    let binary = Utf8PathBuf::from_path_buf(binary.into())
        .map_err(|path| anyhow!("non-UTF-8 PV executable: {path:?}"))?;
    ensure!(binary.is_absolute(), "PV_LINUX_SYSTEM_PV must be absolute");
    let paths = PvPaths::default_home()?;
    let unit = Utf8PathBuf::from_path_buf(environment.daemon_registration_path())
        .map_err(|path| anyhow!("non-UTF-8 unit path: {path:?}"))?;
    ensure!(
        !fs::path_entry_exists(paths.root())?,
        "refusing an existing PV home"
    );
    ensure!(
        !fs::path_entry_exists(&unit)?,
        "refusing an existing user unit"
    );
    ensure!(
        environment.inspect_daemon_registration()?.is_none(),
        "refusing a loaded PV unit"
    );

    let mut fixture = RegistrationFixture {
        paths,
        unit,
        cleaned: false,
        cleanup_timer: None,
        monitor_record: None,
    };
    let result = exercise_registration(&mut fixture, &binary);
    let cleanup = fixture.cleanup();
    match (result, cleanup) {
        (Err(error), Err(cleanup)) => {
            Err(error.context(format!("cleanup also failed: {cleanup:#}")))
        }
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn exercise_registration(fixture: &mut RegistrationFixture, binary: &Utf8Path) -> Result<()> {
    let environment = ProcessEnvironment;
    fs::ensure_layout(&fixture.paths)?;
    seed_redis(&fixture.paths)?;
    fs::copy_file_atomically(binary, &fixture.paths.active_pv_binary())?;
    pv_fake::seed_dns_port(&fixture.paths)?;
    let config = DaemonRegistrationConfig::for_paths(&fixture.paths);
    fixture.arm_cleanup_timer()?;
    // This file has never been loaded: lifecycle ownership must resolve it first.
    let foreign = "[Service]\nExecStart=/bin/true\n";
    fs::write_sensitive_file(&fixture.unit, foreign)?;
    for result in [
        environment.restart_daemon_registration(),
        environment.stop_daemon_registration(),
        environment.enable_daemon_registration(&fixture.unit),
        environment.remove_daemon_registration(&fixture.unit),
    ] {
        assert!(
            matches!(
                result,
                Err(PlatformError::DaemonRegistrationConflict { .. })
            ),
            "{result:?}"
        );
    }
    assert_eq!(fs::read_to_string(&fixture.unit)?, foreign);
    fs::delete_file(&fixture.unit)?;
    platform::write_daemon_registration_file(&fixture.unit, &config)?;
    environment.enable_daemon_registration(&fixture.unit)?;
    let enabled_link = fixture
        .unit
        .parent()
        .context("unit parent")?
        .join("default.target.wants/pv-daemon.service");
    assert_eq!(fs::read_link(&enabled_link)?, fixture.unit);

    // A current file must restore persistent enablement after an external disable.
    fs::delete_file(&enabled_link)?;
    environment.enable_daemon_registration(&fixture.unit)?;
    assert_eq!(fs::read_link(&enabled_link)?, fixture.unit);
    environment.restart_daemon_registration()?;
    daemon::wait_until_healthy_blocking(fixture.paths.clone())?;
    let first = status()?;
    assert_eq!(first.active_state, "active");
    assert_eq!(first.fragment_path, fixture.unit);
    let (first_job, monitor) = wait_for_redis_monitor(&fixture.paths, None)?;
    fixture.monitor_record = Some(monitor.clone());

    environment.restart_daemon_registration()?;
    daemon::wait_until_healthy_blocking(fixture.paths.clone())?;
    let restarted = status()?;
    assert_ne!(restarted.main_pid, first.main_pid);
    let (_, adopted_monitor) = wait_for_redis_monitor(&fixture.paths, Some(&first_job))?;
    assert_eq!(adopted_monitor, monitor);

    // Reloading a deleted fragment must retain evidence of its still-live daemon.
    fs::delete_file(&fixture.unit)?;
    assert!(matches!(
        environment.enable_daemon_registration(&fixture.unit),
        Err(PlatformError::DaemonRegistrationConflict { .. })
    ));
    let deleted = status()?;
    assert_eq!(deleted.main_pid, restarted.main_pid);
    assert_eq!(deleted.active_state, "active");
    assert!(matches!(
        environment.stop_daemon_registration(),
        Err(PlatformError::DaemonRegistrationConflict { .. })
    ));
    fs::write_sensitive_file(&fixture.unit, &config.render()?)?;
    environment.enable_daemon_registration(&fixture.unit)?;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = run_with_environment(
        ["pv", "daemon:disable"],
        &environment,
        &mut stdout,
        &mut stderr,
    )?;
    ensure!(
        code == ExitCode::SUCCESS,
        "disable failed: {}",
        String::from_utf8_lossy(&stderr)
    );
    ensure!(!fs::path_entry_exists(&fixture.unit)?);
    ensure!(!fs::path_entry_exists(&enabled_link)?);
    ensure!(daemon::live_runtime(&fixture.paths, "redis-7.2")?.is_none());
    ensure!(!fs::path_entry_exists(
        &fixture.paths.monitor_dir("redis-7.2")
    )?);

    drop(MonitorReservation::acquire(&fixture.paths, "redis-7.2")?);
    require_recorded_processes_exited(&monitor)?;
    Ok(())
}

fn seed_redis(paths: &PvPaths) -> Result<()> {
    let release = paths.resources().join("redis/7.2/releases/7.2.5-pv1");
    pv_fake::install_with(
        &pv_fake::binary()?,
        &release.join("bin/redis-server"),
        &Scenario {
            persona: Persona::RedisServer,
            lifeline_fd: None,
            settings: FakeSettings::default(),
        },
    )?;
    let project = paths.root().join("system-test-project");
    let config = project.join("pv.yml");
    fs::write_sensitive_file(&config, "serve: false\nredis:\n  version: \"7.2\"\n")?;
    let mut database = Database::open(paths)?;
    database.record_managed_resource_track_installed("redis", "7.2", "7.2.5-pv1", &release)?;
    database.link_project_with_mode(
        LinkProjectInput {
            path: project.clone(),
            original_path: project,
            primary_hostname: "system-test.test".to_owned(),
            config_path: config,
            desired_php_track: None,
        },
        ProjectMode::ResourceOnly,
    )?;
    let port = pv_fake::port()?;
    database.assign_port(
        PortRequest::resource_port("redis", "7.2", "redis", port, port, port),
        |_| true,
    )?;
    Ok(())
}

fn status() -> Result<DaemonRegistrationStatus> {
    ProcessEnvironment
        .inspect_daemon_registration()?
        .context("PV unit was not reported")
}

fn wait_for_redis_monitor(paths: &PvPaths, previous_job: Option<&str>) -> Result<(String, Value)> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let jobs = Database::open(paths)?.recent_jobs()?;
        if let Some(job) = jobs.iter().find(|job| job.scope == "system")
            && Some(job.id.as_str()) != previous_job
        {
            ensure!(
                job.status != JobStatus::Failed,
                "startup reconciliation failed: {:?}",
                job.error
            );
            if job.status == JobStatus::Succeeded
                && daemon::live_runtime(paths, "redis-7.2")?.is_some()
            {
                let path = paths.monitor_dir("redis-7.2").join("monitor.json");
                return Ok((
                    job.id.clone(),
                    serde_json::from_str(&fs::read_to_string(&path)?)?,
                ));
            }
        }
        ensure!(
            Instant::now() < deadline,
            "Redis monitor did not become live; daemon log: {}",
            fs::read_to_string(&paths.daemon_log()).unwrap_or_default()
        );
        thread::sleep(Duration::from_millis(50));
    }
}

struct RegistrationFixture {
    paths: PvPaths,
    unit: Utf8PathBuf,
    cleaned: bool,
    cleanup_timer: Option<String>,
    monitor_record: Option<Value>,
}

fn require_recorded_processes_exited(record: &Value) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(2);
    for process in ["monitor", "runtime"] {
        let pid: u32 = serde_json::from_value(record[process]["pid"].clone())?;
        let identity: platform::ProcessStartIdentity =
            serde_json::from_value(record[process]["start_identity"].clone())?;
        while platform::inspect_process_start_identity(pid)? == Some(identity)
            && !platform::process_is_zombie(pid)?
        {
            ensure!(
                Instant::now() < deadline,
                "fixture {process} {pid} remained live"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
    Ok(())
}

impl RegistrationFixture {
    fn arm_cleanup_timer(&mut self) -> Result<()> {
        let prepared = self.paths.config().join("system-test/pv-daemon.service");
        let script = self.paths.config().join("system-test/cleanup.sh");
        fs::write_sensitive_file(
            &prepared,
            &DaemonRegistrationConfig::for_paths(&self.paths).render()?,
        )?;
        // The user manager cannot inherit the test's lifeline. This timer outlasts nextest's
        // 120-second limit and 10-second shutdown grace, and still fires after test SIGKILL.
        fs::write_sensitive_file(
            &script,
            r#"set -eu
export XDG_CONFIG_HOME="$4"
if cp -- "$1" "$2" && /usr/bin/systemctl --user daemon-reload && "$3" daemon:disable; then
    exit 0
fi
# Last-resort cleanup is confined to the fresh fixture unit's own cgroup.
/usr/bin/systemctl --user stop pv-daemon.service || :
/usr/bin/systemctl --user kill --kill-who=all --signal=KILL pv-daemon.service
exit 1
"#,
        )?;
        let config_home = self.unit.ancestors().nth(3).context("XDG config root")?;
        let name = format!("pv-registration-test-{}", std::process::id());
        let output = SystemTestCommand::new("/usr/bin/systemd-run")
            .args([
                "--user",
                "--collect",
                "--on-active=150s",
                "--unit",
                &name,
                "/bin/sh",
            ])
            .arg(&script)
            .arg(&prepared)
            .arg(&self.unit)
            .arg(self.paths.active_pv_binary())
            .arg(config_home)
            .output()?;
        ensure!(
            output.status.success(),
            "could not arm fixture cleanup: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        self.cleanup_timer = Some(name);
        Ok(())
    }

    fn cleanup(&mut self) -> Result<()> {
        if self.cleaned {
            return Ok(());
        }
        let environment = ProcessEnvironment;
        let config = DaemonRegistrationConfig::for_paths(&self.paths);
        fs::write_sensitive_file(&self.unit, &config.render()?)?;
        environment.enable_daemon_registration(&self.unit)?;
        environment.stop_daemon_registration()?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(daemon::stop_recorded_runtimes(self.paths.clone()))?;
        if let Some(record) = &self.monitor_record {
            require_recorded_processes_exited(record)?;
        }
        environment.remove_daemon_registration(&self.unit)?;
        if let Some(name) = &self.cleanup_timer {
            let output = SystemTestCommand::new("/usr/bin/systemctl")
                .args(["--user", "stop", &format!("{name}.timer")])
                .output()?;
            ensure!(
                output.status.success(),
                "could not cancel fixture cleanup: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        fs::delete_dir_all(self.paths.root())?;
        self.cleaned = true;
        Ok(())
    }
}

impl Drop for RegistrationFixture {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            let _reported = writeln!(
                io::stderr().lock(),
                "systemd fixture cleanup failed: {error:#}"
            );
        }
    }
}
