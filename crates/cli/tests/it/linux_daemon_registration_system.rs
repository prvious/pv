#![cfg(target_os = "linux")]

use platform::clock::{self, real};
use std::io::{self, Write as _};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, ensure};
use camino::{Utf8Path, Utf8PathBuf};
use cli::{Environment, ProcessEnvironment, run_with_environment};
use platform::{DaemonRegistrationConfig, PlatformError};
use pv_fake::{FakeSettings, Persona, Scenario};
use serde_json::Value;
use state::{
    AppReleaseLayout, Database, GatewayPort, JobStatus, LinkProjectInput, MonitorReservation,
    PortRequest, ProjectMode, PvPaths, fs,
};

const REDIS_SUBJECT: &str = "resources/redis/7.2";

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

    let mut fixture = RegistrationFixture {
        paths,
        unit,
        cleaned: false,
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
    seed_resources(&fixture.paths)?;
    let layout = AppReleaseLayout::new(fixture.paths.clone());
    layout.install_release_binary(env!("CARGO_PKG_VERSION"), binary)?;
    layout.activate_release(env!("CARGO_PKG_VERSION"))?;
    pv_fake::seed_dns_port(&fixture.paths)?;
    let config = DaemonRegistrationConfig::for_paths(&fixture.paths);
    // This file has never been loaded: lifecycle ownership must resolve it first.
    let foreign = "[Service]\nExecStart=/bin/true\n";
    fs::write_sensitive_file(&fixture.unit, foreign)?;
    let result = environment.enable_daemon_registration(&fixture.unit);
    assert!(
        matches!(
            result,
            Err(PlatformError::DaemonRegistrationConflict { .. })
        ),
        "{result:?}"
    );
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
    let (first_job, monitor) = wait_for_redis_monitor(&fixture.paths, None)?;
    fixture.monitor_record = Some(monitor.clone());

    environment.stop_daemon_registration()?;
    ensure!(
        environment
            .daemon_process_for_stop(&fixture.paths)?
            .is_none(),
        "the daemon is still running after its service stopped"
    );
    let (_, preserved_monitor) = wait_for_redis_monitor(&fixture.paths, None)?;
    assert_eq!(preserved_monitor, monitor);
    environment.restart_daemon_registration()?;
    daemon::wait_until_healthy_blocking(fixture.paths.clone())?;
    let (_, preserved_monitor) = wait_for_redis_monitor(&fixture.paths, Some(&first_job))?;
    assert_eq!(preserved_monitor, monitor);

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
    ensure!(daemon::live_runtime(&fixture.paths, REDIS_SUBJECT)?.is_none());
    ensure!(!fs::path_entry_exists(
        &fixture.paths.monitor_dir(REDIS_SUBJECT)
    )?);

    drop(MonitorReservation::acquire(&fixture.paths, REDIS_SUBJECT)?);
    require_recorded_processes_exited(&monitor)?;
    Ok(())
}

fn seed_resources(paths: &PvPaths) -> Result<()> {
    let mut database = Database::open(paths)?;
    for (resource, track, version, binary, persona) in [
        (
            "redis",
            "7.2",
            "7.2.5-pv1",
            "redis-server",
            Persona::RedisServer,
        ),
        ("caddy", "2", "2.11.4-pv1", "caddy", Persona::Caddy),
    ] {
        let track_path = paths.resources().join(resource).join(track);
        let release = track_path.join("releases").join(version);
        pv_fake::install_with(
            &pv_fake::binary()?,
            &release.join("bin").join(binary),
            &Scenario {
                persona,
                lifeline_fd: None,
                settings: FakeSettings::default(),
            },
        )?;
        fs::symlink_file(
            Utf8Path::new(&format!("releases/{version}")),
            &track_path.join("current"),
        )?;
        database.record_managed_resource_track_installed(resource, track, version, &release)?;
    }
    let ca = platform::generate_local_ca()?;
    fs::write_sensitive_file(&paths.ca_certificate(), &ca.certificate_pem)?;
    fs::write_sensitive_file(&paths.ca_private_key(), &ca.private_key_pem)?;
    for (gateway_port, port) in [GatewayPort::Http, GatewayPort::Https]
        .into_iter()
        .zip(pv_fake::ports::<2>()?)
    {
        database.assign_port(PortRequest::gateway(gateway_port, port, port, port), |_| {
            true
        })?;
    }
    let project = paths.root().join("system-test-project");
    let config = project.join("pv.yml");
    fs::write_sensitive_file(&config, "serve: false\nredis:\n  version: \"7.2\"\n")?;
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

fn wait_for_redis_monitor(paths: &PvPaths, previous_job: Option<&str>) -> Result<(String, Value)> {
    let deadline = real::now() + Duration::from_secs(15);
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
                && daemon::live_runtime(paths, REDIS_SUBJECT)?.is_some()
            {
                let path = paths.monitor_dir(REDIS_SUBJECT).join("monitor.json");
                return Ok((
                    job.id.clone(),
                    serde_json::from_str(&fs::read_to_string(&path)?)?,
                ));
            }
        }
        ensure!(
            real::now() < deadline,
            "Redis monitor did not become live; daemon log: {}",
            fs::read_to_string(&paths.daemon_log()).unwrap_or_default()
        );
        clock::poll_blocking(Duration::from_millis(50));
    }
}

struct RegistrationFixture {
    paths: PvPaths,
    unit: Utf8PathBuf,
    cleaned: bool,
    monitor_record: Option<Value>,
}

fn require_recorded_processes_exited(record: &Value) -> Result<()> {
    let deadline = real::now() + Duration::from_secs(2);
    for process in ["monitor", "runtime"] {
        let pid: u32 = serde_json::from_value(record[process]["pid"].clone())?;
        let identity: platform::ProcessStartIdentity =
            serde_json::from_value(record[process]["start_identity"].clone())?;
        while platform::inspect_process_start_identity(pid)? == Some(identity)
            && !platform::process_is_zombie(pid)?
        {
            ensure!(
                real::now() < deadline,
                "fixture {process} {pid} remained live"
            );
            clock::poll_blocking(Duration::from_millis(10));
        }
    }
    Ok(())
}

impl RegistrationFixture {
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
