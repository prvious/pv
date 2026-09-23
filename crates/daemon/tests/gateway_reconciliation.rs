use anyhow::{Context, Error, Result, bail, ensure};
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::{tempdir, tempdir_in};
use daemon::gateway::{
    CaddyCliCommand, GatewayPfRoutingState, build_runtime_plan, gateway_process_spec,
    promote_validated_config_for_test, reconcile_gateway_runtimes_with_pf_state_for_test,
    reconcile_project_gateway_runtimes_for_test, validate_config, worker_process_spec,
};
use daemon::{
    AdoptedProcess, CaddyAdminError, CaddyAdminOperation, DaemonError, ProcessSupervisor,
};
use insta::{Settings, allow_duplicates, assert_debug_snapshot};
use platform::ProcessStartIdentity;
use rcgen::generate_simple_self_signed;
use resources::{PHP_TRACK_DEFAULT_INI, php_track_defaults};
use rusqlite::Connection;
use rustix::process::{
    Pid, Signal, getpgid, kill_process_group, test_kill_process, test_kill_process_group,
};
use serde_json::{Value, json};
use state::{
    Database, GatewayPort, LinkProjectInput, PortOwner, PortRequest, ProjectMode,
    ProjectPhpRuntimeInput, PvPaths, RUNTIME_PORT_FALLBACK_END, RUNTIME_PORT_FALLBACK_START,
    RuntimeObservedStatus, RuntimeSubject, StateError, fs,
};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{ErrorKind, Write};
use std::net::TcpListener;
use std::process::Output;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::time::{sleep, timeout};

const GATEWAY_RECONCILIATION_SUMMARY: &str = "Gateway runtime reconciled";
const FAKE_FRANKENPHP_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/gateway/fake-frankenphp.sh"
));
const FAKE_FRANKENPHP_SERVER_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/gateway/fake-frankenphp-server.py"
));
const FAKE_CADDY_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/gateway/fake-caddy.sh"
));
const FAKE_CADDY_SERVER_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/gateway/fake-caddy-server.py"
));
const FAKE_CADDY_NO_ADMIN_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/gateway/fake-caddy-no-admin.sh"
));
const FAKE_CADDY_NO_ADMIN_SERVER_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/gateway/fake-caddy-no-admin-server.py"
));
const FAKE_CADDY_ADMIN_ONLY_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/gateway/fake-caddy-admin-only.sh"
));
const FAKE_CADDY_ADMIN_ONLY_SERVER_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/gateway/fake-caddy-admin-only-server.py"
));
const FAKE_CADDY_LEGACY_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/gateway/fake-caddy-legacy.sh"
));
const FAKE_CADDY_LEGACY_SERVER_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/gateway/fake-caddy-legacy-server.py"
));
const FAKE_STATEFUL_RUNTIME_SERVER_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/gateway/fake-stateful-runtime-server.py"
));
const FAKE_STATEFUL_CADDY_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/gateway/fake-stateful-caddy.sh"
));
const FAKE_STATEFUL_FRANKENPHP_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/gateway/fake-stateful-frankenphp.sh"
));

async fn reconcile_gateway_runtimes(paths: &PvPaths) -> Result<String, DaemonError> {
    ensure_fake_caddy(paths).map_err(|error| DaemonError::UnexpectedProtocolResponse {
        reason: error.to_string(),
    })?;
    reconcile_gateway_runtimes_with_pf_state_for_test(
        paths,
        Duration::from_secs(60),
        GatewayPfRoutingState::Inactive,
    )
    .await
}

async fn reconcile_gateway_runtimes_with_readiness_timeout(
    paths: &PvPaths,
    readiness_timeout: Duration,
) -> Result<String, DaemonError> {
    ensure_fake_caddy(paths).map_err(|error| DaemonError::UnexpectedProtocolResponse {
        reason: error.to_string(),
    })?;
    reconcile_gateway_runtimes_with_pf_state_for_test(
        paths,
        readiness_timeout,
        GatewayPfRoutingState::Inactive,
    )
    .await
}

#[tokio::test]
async fn gateway_reconciliation_stops_before_workers_when_caddy_is_missing() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let release_path = tempdir.path().join("fake-frankenphp-release");
    let fake_frankenphp = release_path.join("bin/frankenphp");

    write_fake_frankenphp(&fake_frankenphp)?;

    let mut database = Database::open(&paths)?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-frankenphp-pv1",
        &release_path,
    )?;
    let ports = available_loopback_ports(2)?;
    seed_runtime_ports(&paths, &mut database, ports[0], ports[1], &[])?;
    drop(database);

    let summary = reconcile_gateway_runtimes_with_pf_state_for_test(
        &paths,
        Duration::from_secs(1),
        GatewayPfRoutingState::Inactive,
    )
    .await?;

    assert_eq!(summary, "Gateway runtime skipped; Caddy is not installed");
    assert!(!paths.gateway_pid().exists());
    assert!(!paths.worker_pid("8.4").exists());

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_does_not_fallback_to_another_caddy_track() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let caddy_release = tempdir.path().join("fake-caddy-3-release");
    let frankenphp_release = tempdir.path().join("fake-frankenphp-release");

    write_fake_caddy(&caddy_release.join("bin/caddy"))?;
    write_fake_frankenphp(&frankenphp_release.join("bin/frankenphp"))?;

    let mut database = Database::open(&paths)?;
    database.record_managed_resource_track_installed(
        "caddy",
        "3",
        "fake-caddy-3-pv1",
        &caddy_release,
    )?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-frankenphp-pv1",
        &frankenphp_release,
    )?;
    let ports = available_loopback_ports(2)?;
    seed_runtime_ports(&paths, &mut database, ports[0], ports[1], &[])?;
    drop(database);

    let summary = reconcile_gateway_runtimes_with_pf_state_for_test(
        &paths,
        Duration::from_secs(1),
        GatewayPfRoutingState::Inactive,
    )
    .await?;

    assert_eq!(summary, "Gateway runtime skipped; Caddy is not installed");
    assert!(!paths.gateway_pid().exists());
    assert!(!paths.worker_pid("8.4").exists());

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_rejects_invalid_caddy_two_without_fallback() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let caddy_release = tempdir.path().join("fake-caddy-3-release");
    let invalid_caddy_release = tempdir.path().join("invalid-caddy-2-release");
    let frankenphp_release = tempdir.path().join("fake-frankenphp-release");

    write_fake_caddy(&caddy_release.join("bin/caddy"))?;
    write_fake_frankenphp(&frankenphp_release.join("bin/frankenphp"))?;

    let mut database = Database::open(&paths)?;
    database.record_managed_resource_track_installed(
        "caddy",
        "3",
        "fake-caddy-3-pv1",
        &caddy_release,
    )?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "invalid-caddy-2-pv1",
        &invalid_caddy_release,
    )?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-frankenphp-pv1",
        &frankenphp_release,
    )?;
    let ports = available_loopback_ports(2)?;
    seed_runtime_ports(&paths, &mut database, ports[0], ports[1], &[])?;
    drop(database);

    let result = reconcile_gateway_runtimes_with_pf_state_for_test(
        &paths,
        Duration::from_secs(1),
        GatewayPfRoutingState::Inactive,
    )
    .await;

    assert!(matches!(
        result,
        Err(DaemonError::Resources(
            resources::ResourcesError::InvalidArtifactLayout { resource, .. }
        )) if resource == "caddy"
    ));
    assert!(!paths.gateway_pid().exists());
    assert!(!paths.worker_pid("8.4").exists());

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_rolls_back_after_fresh_admin_startup_failure() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let caddy_release = tempdir.path().join("fake-caddy-no-admin-release");
    let fake_caddy = caddy_release.join("bin/caddy");
    let previous_root_config = "previous gateway config\n";

    write_fake_caddy_without_admin(&fake_caddy)?;
    let mut database = Database::open(&paths)?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-no-admin-pv1",
        &caddy_release,
    )?;
    let ports = available_loopback_ports(2)?;
    seed_runtime_ports(&paths, &mut database, ports[0], ports[1], &[])?;
    drop(database);
    fs::write_sensitive_file(&paths.gateway_root_config(), previous_root_config)?;

    let result = reconcile_gateway_runtimes_with_pf_state_for_test(
        &paths,
        Duration::from_millis(100),
        GatewayPfRoutingState::Inactive,
    )
    .await;

    assert!(matches!(
        result,
        Err(DaemonError::CaddyAdmin(
            daemon::CaddyAdminError::AdminReadinessTimedOut { .. }
        ))
    ));
    assert_eq!(
        fs::read_to_string(&paths.gateway_root_config())?,
        previous_root_config
    );
    assert!(!paths.gateway_pid().exists());
    assert!(!paths.gateway_runtime_metadata().exists());

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_rolls_back_after_fresh_service_readiness_failure() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let caddy_release = tempdir.path().join("fake-caddy-admin-only-release");
    let fake_caddy = caddy_release.join("bin/caddy");
    let previous_root_config = "previous gateway config\n";

    write_fake_caddy_admin_only(&fake_caddy)?;
    let mut database = Database::open(&paths)?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-admin-only-pv1",
        &caddy_release,
    )?;
    let ports = available_loopback_ports(2)?;
    seed_runtime_ports(&paths, &mut database, ports[0], ports[1], &[])?;
    drop(database);
    fs::write_sensitive_file(&paths.gateway_root_config(), previous_root_config)?;

    let result = reconcile_gateway_runtimes_with_pf_state_for_test(
        &paths,
        Duration::from_millis(500),
        GatewayPfRoutingState::Inactive,
    )
    .await;

    assert!(matches!(result, Err(DaemonError::ReadinessTimedOut { .. })));
    assert_eq!(
        fs::read_to_string(&paths.gateway_root_config())?,
        previous_root_config
    );
    assert!(!paths.gateway_pid().exists());
    assert!(!paths.gateway_runtime_metadata().exists());

    runtime_guard.cleanup().await?;

    Ok(())
}

#[expect(
    clippy::disallowed_types,
    reason = "regression tests spawn a nested test process to control inherited env without unsafe mutation"
)]
type TestProcessCommand = std::process::Command;

#[tokio::test]
async fn gateway_reconciliation_starts_gateway_and_one_worker_per_php_track() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let project_root = tempdir.path().join("acme");
    let release_path = tempdir.path().join("fake-frankenphp-release");
    let decoy_release_path = tempdir.path().join("fake-frankenphp-83-release");
    let fake_frankenphp = release_path.join("bin/frankenphp");
    let decoy_frankenphp = decoy_release_path.join("bin/frankenphp");

    write_fake_frankenphp(&fake_frankenphp)?;
    write_fake_frankenphp(&decoy_frankenphp)?;
    create_project(
        &project_root,
        r#"php: "8.4"
document_root: public
"#,
    )?;

    let mut database = Database::open(&paths)?;
    database.link_project(LinkProjectInput {
        path: project_root.clone(),
        original_path: project_root.clone(),
        primary_hostname: "acme.test".to_owned(),
        config_path: project_root.join("pv.yml"),
        desired_php_track: None,
        additional_hostnames: Vec::new(),
    })?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.3",
        "fake-frankenphp-83-pv1",
        &decoy_release_path,
    )?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-frankenphp-pv1",
        &release_path,
    )?;
    let ports = available_loopback_ports(3)?;
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[("8.4", ports[2])],
    )?;
    drop(database);

    let summary = reconcile_gateway_runtimes(&paths).await?;

    assert_eq!(summary, GATEWAY_RECONCILIATION_SUMMARY);
    assert!(paths.gateway_pid().exists());
    assert!(paths.worker_pid("8.4").exists());

    let database = Database::open(&paths)?;
    assert_runtime_states_snapshot(
        "gateway_reconciliation_starts_gateway_and_one_worker_per_php_track",
        database.runtime_observed_states()?,
    )?;
    assert_worker_command(&paths, "8.4", &fake_frankenphp)?;

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    stop_runtime_from_pid_file(&paths.worker_pid("8.4")).await?;

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_fixture_cleanup_is_scoped_to_recorded_home_and_process_groups() -> Result<()> {
    // Keep both homes short enough for macOS Unix-domain worker socket paths.
    let tempdir = tempdir_in("/tmp")?;
    let paths_a = PvPaths::for_home(tempdir.path().join("home-a"));
    let paths_b = PvPaths::for_home(tempdir.path().join("home-b"));
    let ports = available_loopback_ports(6)?;

    for (paths, project_name, hostname, assigned_ports) in [
        (&paths_a, "project-a", "acme.test", &ports[0..3]),
        (&paths_b, "project-b", "other.test", &ports[3..6]),
    ] {
        let project_root = tempdir.path().join(project_name);
        let release_path = paths.home().join("fake-frankenphp-release");
        write_fake_frankenphp(&release_path.join("bin/frankenphp"))?;
        create_project(
            &project_root,
            r#"php: "8.4"
document_root: public
"#,
        )?;
        let mut database = Database::open(paths)?;
        database.link_project(LinkProjectInput {
            path: project_root.clone(),
            original_path: project_root.clone(),
            primary_hostname: hostname.to_owned(),
            config_path: project_root.join("pv.yml"),
            desired_php_track: None,
            additional_hostnames: Vec::new(),
        })?;
        database.record_managed_resource_track_installed(
            "frankenphp",
            "8.4",
            "fake-frankenphp-pv1",
            &release_path,
        )?;
        seed_runtime_ports(
            paths,
            &mut database,
            assigned_ports[0],
            assigned_ports[1],
            &[("8.4", assigned_ports[2])],
        )?;
    }

    let mut guard_a = GatewayRuntimeGuard::new(paths_a.clone(), &["8.4"]);
    let mut guard_b = GatewayRuntimeGuard::new(paths_b.clone(), &["8.4"]);
    assert_eq!(
        reconcile_gateway_runtimes(&paths_a)
            .await
            .context("home A reconciliation failed")?,
        GATEWAY_RECONCILIATION_SUMMARY
    );
    assert_eq!(
        reconcile_gateway_runtimes(&paths_b)
            .await
            .context("home B reconciliation failed")?,
        GATEWAY_RECONCILIATION_SUMMARY
    );

    let gateway_a_pid =
        guard_a.capture(paths_a.gateway_pid(), paths_a.gateway_runtime_metadata())?;
    let worker_a_pid = guard_a.capture(
        paths_a.worker_pid("8.4"),
        paths_a.worker_runtime_metadata("8.4"),
    )?;

    let supervisor_b = ProcessSupervisor::new(paths_b.clone());
    let gateway_b = supervisor_b
        .adopt_recorded(&paths_b.gateway_pid(), &paths_b.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("home B Gateway was not adoptable"))?;
    let gateway_b_pid = gateway_b.pid();
    let worker_b_pid = supervisor_b
        .adopt_recorded(
            &paths_b.worker_pid("8.4"),
            &paths_b.worker_runtime_metadata("8.4"),
        )?
        .ok_or_else(|| anyhow::anyhow!("home B worker was not adoptable"))?
        .pid();
    let gateway_a_pid_record = fs::read_to_string(&paths_a.gateway_pid())?;
    let gateway_b_pid_record = fs::read_to_string(&paths_b.gateway_pid())?;
    let gateway_b_metadata_record = fs::read_to_string(&paths_b.gateway_runtime_metadata())?;

    fs::write_sensitive_file(&paths_a.gateway_pid(), &gateway_b_pid_record)?;
    fs::write_sensitive_file(
        &paths_a.gateway_runtime_metadata(),
        &gateway_b_metadata_record,
    )?;
    let stale_cleanup = stop_recorded_runtime(
        &paths_a,
        &paths_a.gateway_pid(),
        &paths_a.gateway_runtime_metadata(),
    )
    .await;
    assert!(stale_cleanup.is_err());
    assert_gateway_home_unchanged(&paths_b, gateway_b_pid, worker_b_pid, &ports[3..6]).await?;

    let mut spliced_metadata: Value = serde_json::from_str(&gateway_b_metadata_record)?;
    let Some(spliced_object) = spliced_metadata.as_object_mut() else {
        bail!("Gateway metadata was not an object");
    };
    spliced_object.insert(
        "config_path".to_owned(),
        Value::String(paths_a.gateway_root_config().to_string()),
    );
    spliced_object.insert(
        "log_path".to_owned(),
        Value::String(paths_a.gateway_supervisor_log().to_string()),
    );
    spliced_object.insert(
        "arguments".to_owned(),
        json!([
            "run",
            "--config",
            paths_a.gateway_root_config().as_str(),
            "--adapter",
            "caddyfile"
        ]),
    );
    fs::write_sensitive_file(
        &paths_a.gateway_runtime_metadata(),
        &serde_json::to_string(&spliced_metadata)?,
    )?;
    let spliced_cleanup = stop_recorded_runtime(
        &paths_a,
        &paths_a.gateway_pid(),
        &paths_a.gateway_runtime_metadata(),
    )
    .await;
    assert!(spliced_cleanup.is_err());
    assert_gateway_home_unchanged(&paths_b, gateway_b_pid, worker_b_pid, &ports[3..6]).await?;

    fs::write_sensitive_file(&paths_a.gateway_runtime_metadata(), "{")?;
    let corrupt_cleanup = stop_recorded_runtime(
        &paths_a,
        &paths_a.gateway_pid(),
        &paths_a.gateway_runtime_metadata(),
    )
    .await;
    assert!(corrupt_cleanup.is_err());
    assert_gateway_home_unchanged(&paths_b, gateway_b_pid, worker_b_pid, &ports[3..6]).await?;

    fs::write_sensitive_file(&paths_a.gateway_pid(), &gateway_a_pid_record)?;
    // The captured handles remain authoritative even though the local metadata is malformed.
    guard_a.cleanup().await?;

    assert_process_group_absent(gateway_a_pid)?;
    assert_process_group_absent(worker_a_pid)?;
    let _gateway_http = TcpListener::bind(("127.0.0.1", ports[0]))?;
    let _gateway_https = TcpListener::bind(("127.0.0.1", ports[1]))?;
    let _worker = TcpListener::bind(("127.0.0.1", ports[2]))?;
    assert_gateway_home_unchanged(&paths_b, gateway_b_pid, worker_b_pid, &ports[3..6]).await?;

    let mut database_b = Database::open(&paths_b)?;
    state::testing::transaction(&mut database_b, |transaction| {
        transaction.execute_batch(
            "DELETE FROM managed_resource_tracks
             WHERE resource_name IN ('caddy', 'frankenphp');",
        )
    })?;
    drop(database_b);
    guard_b.cleanup().await?;
    assert_process_group_absent(gateway_b_pid)?;
    assert_process_group_absent(worker_b_pid)?;

    Ok(())
}

async fn assert_gateway_home_unchanged(
    paths: &PvPaths,
    gateway_pid: u32,
    worker_pid: u32,
    ports: &[u16],
) -> Result<()> {
    let supervisor = ProcessSupervisor::new(paths.clone());
    assert_eq!(
        supervisor
            .adopt_recorded(&paths.gateway_pid(), &paths.gateway_runtime_metadata())?
            .map(|runtime| runtime.pid()),
        Some(gateway_pid)
    );
    assert_eq!(
        supervisor
            .adopt_recorded(
                &paths.worker_pid("8.4"),
                &paths.worker_runtime_metadata("8.4")
            )?
            .map(|runtime| runtime.pid()),
        Some(worker_pid)
    );
    for port in ports {
        TcpStream::connect(("127.0.0.1", *port)).await?;
    }

    Ok(())
}

#[tokio::test]
async fn gateway_fixture_cleanup_keeps_records_while_group_descendants_remain() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let release_path = paths.home().join("leader-exit-caddy-release");
    let executable = release_path.join("bin/caddy");
    let descendant_pid_path = paths.run().join("leader-exit-caddy-descendant.pid");
    let leader_release_path = paths.run().join("leader-exit-caddy.release");
    fs::write_sensitive_file(
        &executable,
        "#!/bin/sh\nset -eu\nsleep 30 &\nprintf '%s\\n' \"$!\" > \"$PV_TEST_DESCENDANT_PID_PATH\"\nwhile [ ! -e \"$PV_TEST_LEADER_RELEASE_PATH\" ]; do sleep 0.01; done\n",
    )?;
    set_executable(&executable)?;
    fs::write_sensitive_file(&paths.gateway_root_config(), "fixture")?;
    let mut database = Database::open(&paths)?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "leader-exit-caddy-pv1",
        &release_path,
    )?;

    let supervisor = ProcessSupervisor::new(paths.clone());
    let mut spec = gateway_process_spec(&paths, &CaddyCliCommand::caddy(executable));
    spec.private_environment.insert(
        "PV_TEST_DESCENDANT_PID_PATH".to_owned(),
        descendant_pid_path.to_string(),
    );
    spec.private_environment.insert(
        "PV_TEST_LEADER_RELEASE_PATH".to_owned(),
        leader_release_path.to_string(),
    );
    let mut process = supervisor.start(spec).await?;
    let pid = process.pid();
    let mut process_group_guard = match CapturedProcessGroupGuard::new(pid) {
        Ok(guard) => guard,
        Err(capture_error) => {
            return match process.stop(Duration::from_secs(1)).await {
                Ok(()) => Err(capture_error),
                Err(cleanup_error) => Err(anyhow::anyhow!(
                    "identity capture failed: {capture_error:#}; fixture cleanup failed: {cleanup_error:#}"
                )),
            };
        }
    };
    let adopted = supervisor
        .adopt_recorded(&paths.gateway_pid(), &paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("Gateway fixture was not adoptable before leader exit"))?;
    let operation_result = async {
        timeout(Duration::from_secs(5), async {
            while !descendant_pid_path.exists() {
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .context("timed out waiting for the Gateway descendant PID")?;
        let descendant_pid = fs::read_to_string(&descendant_pid_path)?
            .trim()
            .parse::<u32>()?;
        process_group_guard.capture(descendant_pid)?;
        fs::write_sensitive_file(&leader_release_path, "release\n")?;
        timeout(Duration::from_secs(5), async {
            loop {
                if process.has_exited()? {
                    return Ok::<_, anyhow::Error>(());
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await??;
        if !process_group_guard.member_is_live()? {
            bail!("gateway descendant {descendant_pid} exited before cleanup");
        }

        let pid_record = fs::read_to_string(&paths.gateway_pid())?;
        let metadata_record = fs::read_to_string(&paths.gateway_runtime_metadata())?;
        let adopted_cleanup = adopted.stop(Duration::from_secs(1)).await;
        if !matches!(
            &adopted_cleanup,
            Err(DaemonError::RuntimeProcessIdentityChanged { pid: error_pid })
                if *error_pid == pid
        ) {
            bail!("captured cleanup did not report the exited group leader: {adopted_cleanup:?}");
        }
        if !process_group_guard.member_is_live()? {
            bail!("captured cleanup signaled gateway descendant {descendant_pid} without current ownership proof");
        }
        let recorded_cleanup = stop_recorded_runtime(
            &paths,
            &paths.gateway_pid(),
            &paths.gateway_runtime_metadata(),
        )
        .await;
        let Err(recorded_cleanup) = recorded_cleanup else {
            bail!("recorded cleanup accepted records for an exited group leader");
        };
        if !matches!(
            recorded_cleanup.downcast_ref::<DaemonError>(),
            Some(DaemonError::RuntimeProcessIdentityChanged { pid: error_pid })
                if *error_pid == pid
        ) {
            bail!("recorded cleanup did not report the exited group leader: {recorded_cleanup:#}");
        }
        let records_unchanged = fs::read_to_string(&paths.gateway_pid())? == pid_record
            && fs::read_to_string(&paths.gateway_runtime_metadata())? == metadata_record;
        if !records_unchanged {
            bail!("cleanup changed records after it could no longer verify their leader");
        }
        if !process_group_guard.member_is_live()? {
            bail!("cleanup signaled gateway descendant {descendant_pid} without current ownership proof");
        }

        Ok::<_, anyhow::Error>(())
    }
    .await;

    let process_group_cleanup = process_group_guard.cleanup();
    let leader_cleanup = timeout(Duration::from_secs(1), async {
        loop {
            if process.has_exited()? {
                return Ok::<_, anyhow::Error>(());
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    let leader_cleanup = match leader_cleanup {
        Ok(result) => result,
        Err(_elapsed) => Err(anyhow::anyhow!(
            "timed out reaping Gateway fixture leader {pid}"
        )),
    };
    let cleanup_result = match (process_group_cleanup, leader_cleanup) {
        (Ok(()), Ok(())) => (|| {
            fs::remove_file_if_exists(&paths.gateway_pid())?;
            fs::remove_file_if_exists(&paths.gateway_runtime_metadata())?;
            Ok(())
        })(),
        (Err(group_error), Ok(())) => Err(group_error),
        (Ok(()), Err(leader_error)) => Err(leader_error),
        (Err(group_error), Err(leader_error)) => Err(anyhow::anyhow!(
            "process-group cleanup failed: {group_error:#}; leader cleanup failed: {leader_error:#}"
        )),
    };

    match (operation_result, cleanup_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(operation_error), Ok(())) => Err(operation_error),
        (Ok(()), Err(cleanup_error)) => Err(cleanup_error),
        (Err(operation_error), Err(cleanup_error)) => Err(anyhow::anyhow!(
            "operation failed: {operation_error:#}; fixture cleanup failed: {cleanup_error:#}"
        )),
    }
}

#[tokio::test]
async fn gateway_reconciliation_recovers_after_bounded_worker_wave_is_cancelled() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let track = "8.4";
    let extension_sets = [
        Vec::new(),
        vec!["redis"],
        vec!["xdebug"],
        vec!["apcu"],
        vec!["redis", "xdebug"],
    ];
    let mut runtime_keys = extension_sets
        .iter()
        .map(|extensions| {
            state::php_runtime_key(
                track,
                &extensions
                    .iter()
                    .map(|extension| (*extension).to_owned())
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    runtime_keys.sort();
    let hostnames = [
        "acme.test",
        "api.acme.test",
        "broken.test",
        "changed.acme.test",
        "other.test",
    ];
    let mut database = Database::open(&paths)?;
    let ports = available_loopback_ports(runtime_keys.len() + 2)?;
    for (index, (extensions, hostname)) in extension_sets.iter().zip(hostnames).enumerate() {
        let extensions = extensions.join(", ");
        let project_root = create_project_with_config(
            tempdir.path(),
            &format!("project-{index}"),
            &format!("php:\n  version: \"{track}\"\n  extensions: [{extensions}]\n"),
        )?;
        database.link_project(LinkProjectInput {
            path: project_root.clone(),
            original_path: project_root.clone(),
            primary_hostname: hostname.to_owned(),
            config_path: project_root.join("pv.yml"),
            desired_php_track: Some(track.to_owned()),
            additional_hostnames: Vec::new(),
        })?;
    }
    drop(database);
    let release_path =
        seed_installed_php_with_extensions(&paths, track, &["apcu", "redis", "xdebug"])?;
    seed_installed_frankenphp_with_extensions(
        &paths,
        track,
        &release_path,
        &["apcu", "redis", "xdebug"],
    )?;
    write_fake_frankenphp(&release_path.join("bin/frankenphp"))?;
    let mut database = Database::open(&paths)?;
    let worker_ports = runtime_keys
        .iter()
        .zip(&ports[2..])
        .map(|(runtime_key, port)| (runtime_key.as_str(), *port))
        .collect::<Vec<_>>();
    seed_runtime_ports(&paths, &mut database, ports[0], ports[1], &worker_ports)?;
    drop(database);
    ensure_fake_caddy(&paths)?;

    let gates = runtime_keys
        .iter()
        .map(|track| {
            Utf8PathBuf::from(format!(
                "{}.readiness-gate",
                paths.worker_root_config(track)
            ))
        })
        .collect::<Vec<_>>();
    let probes = runtime_keys
        .iter()
        .map(|track| {
            Utf8PathBuf::from(format!(
                "{}.readiness-probed",
                paths.worker_root_config(track)
            ))
        })
        .collect::<Vec<_>>();
    for gate in &gates {
        fs::write_sensitive_file(gate, "wait\n")?;
    }

    let reconciliation_paths = paths.clone();
    let mut reconciliation = tokio::spawn(async move {
        reconcile_gateway_runtimes_with_readiness_timeout(
            &reconciliation_paths,
            Duration::from_secs(5),
        )
        .await
    });
    let mut reconciliation_completed = false;
    let inspection = async {
        tokio::select! {
            result = &mut reconciliation => {
                reconciliation_completed = true;
                bail!("worker readiness reconciliation finished before the first wave was gated: {result:#?}");
            }
            result = wait_for_existing_path_count(&probes, 4) => result?,
        }

        ensure!(
            runtime_keys[..4]
                .iter()
                .all(|runtime_key| paths.worker_pid(runtime_key).exists()),
            "first worker wave did not publish all runtime records"
        );
        ensure!(
            !paths.worker_pid(&runtime_keys[4]).exists(),
            "fifth worker started before a first-wave slot opened"
        );
        ensure!(!probes[4].exists(), "fifth worker probed too early");
        ensure!(!paths.gateway_pid().exists(), "Gateway started before workers");
        fs::remove_file(&gates[0])?;
        tokio::select! {
            result = &mut reconciliation => {
                reconciliation_completed = true;
                bail!("worker reconciliation finished with siblings still gated: {result:#?}");
            }
            result = wait_for_existing_path_count(&probes, 5) => result?,
        }
        let worker_pids = runtime_keys
            .iter()
            .map(|runtime_key| {
                required_runtime_metadata_pid(&paths.worker_runtime_metadata(runtime_key))
            })
            .collect::<Result<Vec<_>>>()?;
        for (index, runtime_key) in runtime_keys.iter().enumerate() {
            let metadata: Value = serde_json::from_str(&fs::read_to_string(
                &paths.worker_runtime_metadata(runtime_key),
            )?)?;
            if index == 0 {
                ensure!(metadata["replacement_required"] != true);
                ensure!(metadata["applied_config_fingerprint"].is_string());
                ensure!(metadata["staged_config_fingerprint"].is_null());
            } else {
                ensure!(metadata["replacement_required"] == true);
                ensure!(metadata["applied_config_fingerprint"].is_null());
                ensure!(metadata["staged_config_fingerprint"].is_string());
            }
        }

        Ok::<_, anyhow::Error>(worker_pids)
    }
    .await;
    if reconciliation_completed {
        inspection?;
        bail!("worker readiness reconciliation finished before inspection completed");
    }
    reconciliation.abort();
    let cancellation = reconciliation.await;
    let worker_pids = inspection?;
    let cancellation = match cancellation {
        Ok(result) => bail!("worker readiness reconciliation was not cancelled: {result:?}"),
        Err(error) => error,
    };
    assert!(cancellation.is_cancelled());
    for gate in gates.iter().skip(1) {
        fs::remove_file(gate)?;
    }
    assert!(!paths.gateway_pid().exists());

    let summary = timeout(
        Duration::from_secs(10),
        reconcile_gateway_runtimes_with_readiness_timeout(&paths, Duration::from_secs(5)),
    )
    .await
    .context("cancelled worker readiness reconciliation did not recover")??;
    assert_eq!(summary, GATEWAY_RECONCILIATION_SUMMARY);
    for (index, (runtime_key, worker_pid)) in runtime_keys.iter().zip(&worker_pids).enumerate() {
        let recovered_pid =
            required_runtime_metadata_pid(&paths.worker_runtime_metadata(runtime_key))?;
        if index == 0 {
            assert_eq!(recovered_pid, *worker_pid);
        } else {
            assert_ne!(recovered_pid, *worker_pid);
        }
        let metadata: Value = serde_json::from_str(&fs::read_to_string(
            &paths.worker_runtime_metadata(runtime_key),
        )?)?;
        assert_ne!(metadata["replacement_required"], true);
        assert!(metadata["applied_config_fingerprint"].is_string());
        assert!(metadata["staged_config_fingerprint"].is_null());
    }

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    for runtime_key in runtime_keys {
        stop_runtime_from_pid_file(&paths.worker_pid(&runtime_key)).await?;
    }

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn matching_worker_recovers_after_post_load_readiness_is_cancelled() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let track = "8.4";
    let project_root = create_project_with_config(
        tempdir.path(),
        "acme",
        "php: \"8.4\"\ndocument_root: public\n",
    )?;
    let caddy_release = tempdir.path().join("fake-caddy-release");
    write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;

    let release_path = seed_installed_php_with_extensions(&paths, track, &[])?;
    seed_installed_frankenphp_with_extensions(&paths, track, &release_path, &[])?;
    write_stateful_fake_frankenphp(&release_path.join("bin/frankenphp"))?;
    let ports = available_loopback_ports(4)?;
    let mut database = Database::open(&paths)?;
    database.link_project(LinkProjectInput {
        path: project_root.clone(),
        original_path: project_root.clone(),
        primary_hostname: "acme.test".to_owned(),
        config_path: project_root.join("pv.yml"),
        desired_php_track: Some(track.to_owned()),
        additional_hostnames: Vec::new(),
    })?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[(track, ports[2])],
    )?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let gateway_pid = required_runtime_metadata_pid(&paths.gateway_runtime_metadata())?;
    let worker_pid = required_runtime_metadata_pid(&paths.worker_runtime_metadata(track))?;
    let gateway_root = read_test_bytes(paths.gateway_root_config())?;
    let gateway_load_count = fake_admin_load_bodies(&paths.gateway_root_config())?.len();
    let load_accepted_marker = tempdir.path().join("matching-worker-load-accepted");
    write_fake_admin_control(
        &paths.worker_root_config(track),
        json!({"load_accepted_marker": load_accepted_marker.as_str()}),
    )?;
    let mut database = Database::open(&paths)?;
    database.release_port(PortOwner::PhpWorker {
        php_runtime_key: track.to_owned(),
    })?;
    database.assign_port(
        PortRequest::php_worker(track, ports[3], ports[3], ports[3]),
        |_port| true,
    )?;
    drop(database);

    let reconciliation_paths = paths.clone();
    let mut reconciliation = tokio::spawn(async move {
        reconcile_gateway_runtimes_with_readiness_timeout(
            &reconciliation_paths,
            Duration::from_secs(5),
        )
        .await
    });
    let mut reconciliation_completed = false;
    let inspection = async {
        tokio::select! {
            result = &mut reconciliation => {
                reconciliation_completed = true;
                let requests = fake_admin_requests(&paths.worker_root_config(track))?;
                bail!("matching worker reconciliation finished before post-load readiness was gated: result={result:#?}, requests={requests:#?}");
            }
            result = wait_for_existing_path_count(std::slice::from_ref(&load_accepted_marker), 1) => result?,
        }

        let pending_metadata: Value =
            serde_json::from_str(&fs::read_to_string(&paths.worker_runtime_metadata(track))?)?;
        ensure!(pending_metadata["replacement_required"] == true);
        ensure!(pending_metadata["applied_config_fingerprint"].is_null());
        ensure!(process_is_alive(worker_pid)?);
        ensure!(
            required_runtime_metadata_pid(&paths.gateway_runtime_metadata())? == gateway_pid
        );
        ensure!(read_test_bytes(paths.gateway_root_config())? == gateway_root);
        ensure!(
            fake_admin_load_bodies(&paths.gateway_root_config())?.len() == gateway_load_count
        );

        Ok::<_, anyhow::Error>(())
    }
    .await;
    if reconciliation_completed {
        inspection?;
        bail!("matching worker reconciliation finished before inspection completed");
    }
    reconciliation.abort();
    let cancellation = reconciliation.await;
    inspection?;
    let cancellation = match cancellation {
        Ok(result) => bail!("matching worker reconciliation was not cancelled: {result:?}"),
        Err(error) => error,
    };
    assert!(cancellation.is_cancelled());
    write_fake_admin_control(&paths.worker_root_config(track), json!({}))?;

    let summary = timeout(
        Duration::from_secs(10),
        reconcile_gateway_runtimes_with_readiness_timeout(&paths, Duration::from_secs(5)),
    )
    .await
    .context("cancelled matching worker reconciliation did not recover")??;
    let replacement_worker_pid =
        required_runtime_metadata_pid(&paths.worker_runtime_metadata(track))?;
    let replacement_metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.worker_runtime_metadata(track))?)?;

    wait_for_process_exit(worker_pid).await?;
    assert_eq!(summary, GATEWAY_RECONCILIATION_SUMMARY);
    assert_ne!(replacement_worker_pid, worker_pid);
    assert_ne!(replacement_metadata["replacement_required"], true);
    assert!(replacement_metadata["applied_config_fingerprint"].is_string());
    assert_eq!(
        required_runtime_metadata_pid(&paths.gateway_runtime_metadata())?,
        gateway_pid
    );
    assert_eq!(read_test_bytes(paths.gateway_root_config())?, gateway_root);
    assert_eq!(
        fake_admin_load_bodies(&paths.gateway_root_config())?.len(),
        gateway_load_count + 1
    );

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    stop_runtime_from_pid_file(&paths.worker_pid(track)).await?;

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn runtime_move_accepts_prior_proof_after_artifact_change() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let moving_root = create_project_with_config(tempdir.path(), "moving", "php: \"8.4\"\n")?;
    let peer_root = create_project_with_config(tempdir.path(), "peer", "php: \"8.4\"\n")?;
    link_project_record(&paths, &moving_root, "acme.test", Some("8.4"))?;
    link_project_record(&paths, &peer_root, "api.acme.test", Some("8.4"))?;
    let caddy_release = tempdir.path().join("caddy");
    let source_release = tempdir.path().join("frankenphp-84");
    let replaced_release = tempdir.path().join("frankenphp-84b");
    let destination_release = tempdir.path().join("frankenphp-85");
    write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;
    write_stateful_fake_frankenphp(&source_release.join("bin/frankenphp"))?;
    write_stateful_fake_frankenphp(&replaced_release.join("bin/frankenphp"))?;
    write_stateful_fake_frankenphp(&destination_release.join("bin/frankenphp"))?;
    let ports = available_loopback_ports(4)?;
    let mut database = Database::open(&paths)?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-84-pv1",
        &source_release,
    )?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.5",
        "fake-85-pv1",
        &destination_release,
    )?;
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[("8.4", ports[2]), ("8.5", ports[3])],
    )?;
    let moving = database
        .projects()?
        .into_iter()
        .find(|project| project.path == moving_root)
        .ok_or_else(|| anyhow::anyhow!("missing moving Project"))?;
    drop(database);
    reconcile_gateway_runtimes(&paths).await?;
    let file_name = format!("{}.Caddyfile", moving.id);
    let source_fragment = paths.worker_projects_config_dir("8.4").join(&file_name);
    let gateway_fragment = paths.gateway_projects_config_dir().join(&file_name);
    let previous_gateway = fs::read_to_string(&gateway_fragment)?;

    Database::open(&paths)?.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-84b-pv1",
        &replaced_release,
    )?;
    fs::write_sensitive_file(&moving.config_path, "php: \"8.5\"\n")?;

    reconcile_gateway_runtimes(&paths).await?;
    let moved_worker_pid = required_runtime_metadata_pid(&paths.worker_runtime_metadata("8.4"))?;
    assert!(process_is_alive(moved_worker_pid)?);
    assert!(!source_fragment.exists());
    let destination_fragment = paths.worker_projects_config_dir("8.5").join(&file_name);
    assert!(destination_fragment.exists());
    assert_ne!(fs::read_to_string(&gateway_fragment)?, previous_gateway);
    assert!(process_is_alive(required_runtime_metadata_pid(
        &paths.gateway_runtime_metadata()
    )?)?);

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    stop_runtime_from_pid_file(&paths.worker_pid("8.4")).await?;
    stop_runtime_from_pid_file(&paths.worker_pid("8.5")).await?;

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_runtime_move_retains_source_until_gateway_commit() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let moving_root = create_project_with_config(tempdir.path(), "moving", "php: \"8.4\"\n")?;
    let peer_root = create_project_with_config(tempdir.path(), "peer", "php: \"8.4\"\n")?;
    link_project_record(&paths, &moving_root, "acme.test", Some("8.4"))?;
    link_project_record(&paths, &peer_root, "api.acme.test", Some("8.4"))?;
    let caddy_release = tempdir.path().join("caddy");
    let source_release = tempdir.path().join("frankenphp-84");
    let destination_release = tempdir.path().join("frankenphp-85");
    write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;
    write_stateful_fake_frankenphp(&source_release.join("bin/frankenphp"))?;
    write_stateful_fake_frankenphp(&destination_release.join("bin/frankenphp"))?;
    let ports = available_loopback_ports(4)?;
    let mut database = Database::open(&paths)?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-84-pv1",
        &source_release,
    )?;
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[("8.4", ports[2]), ("8.5", ports[3])],
    )?;
    let moving = database
        .projects()?
        .into_iter()
        .find(|project| project.path == moving_root)
        .ok_or_else(|| anyhow::anyhow!("missing moving Project"))?;
    drop(database);
    reconcile_gateway_runtimes(&paths).await?;
    let source_pid = required_runtime_metadata_pid(&paths.worker_runtime_metadata("8.4"))?;
    let gateway_pid = required_runtime_metadata_pid(&paths.gateway_runtime_metadata())?;
    let file_name = format!("{}.Caddyfile", moving.id);
    let source_fragment = paths.worker_projects_config_dir("8.4").join(&file_name);
    let gateway_fragment = paths.gateway_projects_config_dir().join(&file_name);
    let previous_source = fs::read_to_string(&source_fragment)?;
    let previous_gateway = fs::read_to_string(&gateway_fragment)?;
    fs::write_sensitive_file(&moving.config_path, "php: \"8.5\"\n")?;

    let missing_destination = reconcile_gateway_runtimes(&paths).await;
    assert!(matches!(
        missing_destination,
        Err(DaemonError::UnexpectedProtocolResponse { .. })
    ));
    assert_eq!(fs::read_to_string(&source_fragment)?, previous_source);
    assert_eq!(fs::read_to_string(&gateway_fragment)?, previous_gateway);
    assert!(process_is_alive(source_pid)?);

    Database::open(&paths)?.record_managed_resource_track_installed(
        "frankenphp",
        "8.5",
        "fake-85-pv1",
        &destination_release,
    )?;
    let source_metadata = fs::read_to_string(&paths.worker_runtime_metadata("8.4"))?;
    let connection = Connection::open(paths.db().as_std_path())?;
    connection.execute_batch(
        "CREATE TRIGGER reject_source_observation BEFORE INSERT ON observed_states
         WHEN NEW.subject_kind = 'runtime' AND NEW.subject_id = 'php_worker:8.4'
         BEGIN SELECT RAISE(FAIL, 'fixture rejected source observation'); END;",
    )?;
    let mut unverified_metadata: Value = serde_json::from_str(&source_metadata)?;
    unverified_metadata["applied_config_fingerprint"] = Value::Null;
    fs::write_sensitive_file(
        &paths.worker_runtime_metadata("8.4"),
        &serde_json::to_string(&unverified_metadata)?,
    )?;
    let observation_failure = reconcile_gateway_runtimes(&paths).await;
    connection.execute_batch("DROP TRIGGER reject_source_observation")?;
    fs::write_sensitive_file(&paths.worker_runtime_metadata("8.4"), &source_metadata)?;
    let Err(DaemonError::RuntimeReconciliationFailures { failures }) = observation_failure else {
        bail!("expected source-proof and recording failures, got {observation_failure:?}");
    };
    assert_eq!(failures.len(), 2);
    assert!(
        failures
            .iter()
            .all(|failure| failure.runtime_key() == "8.4")
    );
    assert!(failures.iter().any(|failure| matches!(
        failure.error(),
        DaemonError::UnexpectedProtocolResponse { .. }
    )));
    assert!(
        failures
            .iter()
            .any(|failure| matches!(failure.error(), DaemonError::State(StateError::Sqlite(_))))
    );
    let independent_metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.worker_runtime_metadata("8.5"))?)?;
    assert!(independent_metadata["applied_config_fingerprint"].is_string());
    assert_eq!(fs::read_to_string(&source_fragment)?, previous_source);
    assert_eq!(fs::read_to_string(&gateway_fragment)?, previous_gateway);

    fs::write_sensitive_file(
        &peer_root.join("pv.yml"),
        "php: \"8.4\"\ndocument_root: .\n",
    )?;
    write_fake_admin_control(
        &paths.worker_root_config("8.4"),
        json!({"load_statuses": [422]}),
    )?;
    let source_rejected =
        reconcile_gateway_runtimes_with_readiness_timeout(&paths, Duration::from_secs(1)).await;
    assert!(matches!(
        source_rejected,
        Err(DaemonError::CaddyAdmin(CaddyAdminError::LoadRejected {
            status: 422,
            ..
        }))
    ));
    assert_eq!(fs::read_to_string(&source_fragment)?, previous_source);
    assert_eq!(fs::read_to_string(&gateway_fragment)?, previous_gateway);
    write_fake_admin_control(
        &paths.worker_root_config("8.4"),
        json!({
            "late_accept": [true], "late_apply_delay_ms": [2000],
            "load_delay_ms": [2000], "load_statuses": [200]
        }),
    )?;
    let uncertain_source =
        reconcile_gateway_runtimes_with_readiness_timeout(&paths, Duration::from_millis(150)).await;
    assert!(matches!(
        uncertain_source,
        Err(DaemonError::CaddyAdmin(
            CaddyAdminError::RequestOutcomeUnknown {
                operation: CaddyAdminOperation::Load,
                ..
            }
        ))
    ));
    let staged_source_metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.worker_runtime_metadata("8.4"))?)?;
    assert_eq!(staged_source_metadata["replacement_required"], true);
    assert!(staged_source_metadata["applied_config_fingerprint"].is_null());
    assert!(staged_source_metadata["staged_config_fingerprint"].is_string());
    assert_eq!(fs::read_to_string(&source_fragment)?, previous_source);
    assert_eq!(fs::read_to_string(&gateway_fragment)?, previous_gateway);

    write_fake_frankenphp(&source_release.join("bin/frankenphp"))?;
    let source_failure_marker = Utf8PathBuf::from(format!(
        "{}.readiness-fail",
        paths.worker_root_config("8.4")
    ));
    fs::write_sensitive_file(
        &source_failure_marker,
        "fail
",
    )?;
    write_fake_admin_control(
        &paths.gateway_root_config(),
        json!({"load_statuses": [422]}),
    )?;
    let failed_replacement =
        reconcile_gateway_runtimes_with_readiness_timeout(&paths, Duration::from_secs(1)).await;
    assert!(matches!(
        failed_replacement,
        Err(DaemonError::UnexpectedProtocolResponse { ref reason })
            if reason.contains("php-worker-8.4")
                && reason.contains("exited before readiness was verified")
    ));
    let failed_replacement_pid =
        required_runtime_metadata_pid(&paths.worker_runtime_metadata("8.4"))?;
    wait_for_process_exit(failed_replacement_pid).await?;
    let failed_replacement_metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.worker_runtime_metadata("8.4"))?)?;
    assert_eq!(failed_replacement_metadata["replacement_required"], true);
    assert!(failed_replacement_metadata["applied_config_fingerprint"].is_null());
    assert!(failed_replacement_metadata["staged_config_fingerprint"].is_string());
    assert_eq!(fs::read_to_string(&source_fragment)?, previous_source);
    assert_eq!(fs::read_to_string(&gateway_fragment)?, previous_gateway);

    fs::remove_file(&source_failure_marker)?;
    let rejected =
        reconcile_gateway_runtimes_with_readiness_timeout(&paths, Duration::from_secs(1)).await;
    assert!(matches!(
        rejected,
        Err(DaemonError::CaddyAdmin(CaddyAdminError::LoadRejected {
            status: 422,
            ..
        }))
    ));
    let replacement_source_pid =
        required_runtime_metadata_pid(&paths.worker_runtime_metadata("8.4"))?;
    assert_ne!(replacement_source_pid, source_pid);
    assert_ne!(replacement_source_pid, failed_replacement_pid);
    let replacement_source_metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.worker_runtime_metadata("8.4"))?)?;
    assert_ne!(replacement_source_metadata["replacement_required"], true);
    assert!(replacement_source_metadata["applied_config_fingerprint"].is_string());
    assert!(replacement_source_metadata["staged_config_fingerprint"].is_null());
    assert_eq!(fs::read_to_string(&source_fragment)?, previous_source);
    assert_eq!(fs::read_to_string(&gateway_fragment)?, previous_gateway);
    let destination_pid = required_runtime_metadata_pid(&paths.worker_runtime_metadata("8.5"))?;
    let destination_metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.worker_runtime_metadata("8.5"))?)?;
    assert!(destination_metadata["applied_config_fingerprint"].is_string());
    assert_ne!(destination_metadata["replacement_required"], true);

    write_fake_admin_control(
        &paths.gateway_root_config(),
        json!({
            "late_accept": [true], "late_apply_delay_ms": [2000],
            "load_delay_ms": [2000], "load_statuses": [200]
        }),
    )?;
    let uncertain =
        reconcile_gateway_runtimes_with_readiness_timeout(&paths, Duration::from_millis(150)).await;
    assert!(matches!(
        uncertain,
        Err(DaemonError::CaddyAdmin(
            CaddyAdminError::RequestOutcomeUnknown {
                operation: CaddyAdminOperation::Load,
                ..
            }
        ))
    ));
    let pending_metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.gateway_runtime_metadata())?)?;
    assert_eq!(pending_metadata["replacement_required"], true);
    assert!(pending_metadata["applied_config_fingerprint"].is_null());
    assert!(pending_metadata["staged_config_fingerprint"].is_string());
    assert_eq!(fs::read_to_string(&source_fragment)?, previous_source);
    assert!(process_is_alive(replacement_source_pid)?);
    assert!(process_is_alive(destination_pid)?);

    let readiness_gate = tempdir.path().join("gateway-ready");
    write_fake_admin_control(
        &paths.gateway_root_config(),
        json!({
            "admin_response_gate": readiness_gate.as_str()
        }),
    )?;
    let requests_before_retry = fake_admin_requests(&paths.gateway_root_config())?.len();
    let release_gateway = async {
        timeout(Duration::from_secs(5), async {
            loop {
                if let Some(pid) = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
                    && pid != gateway_pid
                    && fake_admin_requests(&paths.gateway_root_config())?
                        .iter()
                        .skip(requests_before_retry)
                        .any(|request| request["method"] == "GET" && request["path"] == "/config/")
                {
                    assert_eq!(fs::read_to_string(&source_fragment)?, previous_source);
                    fs::write_sensitive_file(&readiness_gate, "ready\n")?;
                    return Ok::<(), Error>(());
                }
                sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .context("replacement Gateway did not reach readiness")?
    };
    let (retry, release) = tokio::join!(
        reconcile_gateway_runtimes_with_readiness_timeout(&paths, Duration::from_secs(5)),
        release_gateway
    );
    release?;
    retry?;
    assert!(!source_fragment.exists());
    assert!(
        paths
            .worker_projects_config_dir("8.5")
            .join(&file_name)
            .exists()
    );
    assert_ne!(fs::read_to_string(&gateway_fragment)?, previous_gateway);
    assert_eq!(
        required_runtime_metadata_pid(&paths.worker_runtime_metadata("8.4"))?,
        replacement_source_pid
    );
    assert_eq!(
        required_runtime_metadata_pid(&paths.worker_runtime_metadata("8.5"))?,
        destination_pid
    );
    let committed_metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.gateway_runtime_metadata())?)?;
    assert_ne!(committed_metadata["replacement_required"], true);
    assert!(committed_metadata["applied_config_fingerprint"].is_string());
    assert!(committed_metadata["staged_config_fingerprint"].is_null());
    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    stop_runtime_from_pid_file(&paths.worker_pid("8.4")).await?;
    stop_runtime_from_pid_file(&paths.worker_pid("8.5")).await?;

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn worker_failure_starts_missing_gateway_without_reloading_live_gateway() -> Result<()> {
    for live_gateway in [false, true] {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
        let track = "8.4";
        let base_project =
            create_project_with_config(tempdir.path(), "base", "php:\n  version: \"8.4\"\n")?;
        link_project_record(&paths, &base_project, "acme.test", Some(track))?;
        let base_id = Database::open(&paths)?
            .projects()?
            .into_iter()
            .find(|project| project.path == base_project)
            .ok_or_else(|| anyhow::anyhow!("missing base Project"))?
            .id;

        let release_path = seed_installed_php_with_extensions(&paths, track, &["redis"])?;
        seed_installed_frankenphp_with_extensions(&paths, track, &release_path, &["redis"])?;
        write_fake_frankenphp(&release_path.join("bin/frankenphp"))?;

        let base_runtime_key = state::php_runtime_key(track, &[])?;
        let redis_runtime_key = state::php_runtime_key(track, &["redis".to_owned()])?;
        let ports = available_loopback_ports(4)?;
        let mut database = Database::open(&paths)?;
        seed_runtime_ports(
            &paths,
            &mut database,
            ports[0],
            ports[1],
            &[
                (&base_runtime_key, ports[2]),
                (&redis_runtime_key, ports[3]),
            ],
        )?;
        drop(database);

        reconcile_gateway_runtimes(&paths).await?;
        let gateway_pid = required_runtime_metadata_pid(&paths.gateway_runtime_metadata())?;
        let gateway_root = fs::read_to_string(&paths.gateway_root_config())?;
        let source_fragment = paths
            .worker_projects_config_dir(&base_runtime_key)
            .join(format!("{base_id}.Caddyfile"));
        let source_content = fs::read_to_string(&source_fragment)?;

        let redis_project = create_project_with_config(
            tempdir.path(),
            "redis",
            "php:\n  version: \"8.4\"\n  extensions: [redis]\n",
        )?;
        link_project_record(&paths, &redis_project, "api.acme.test", Some(track))?;
        if !live_gateway {
            stop_recorded_runtime_preserving_files(
                &paths,
                &paths.gateway_pid(),
                &paths.gateway_runtime_metadata(),
            )
            .await?;
            let tampered = format!(
                "{}# tampered Gateway root\n",
                fs::read_to_string(&paths.gateway_root_config())?
            );
            fs::write_sensitive_file(&paths.gateway_root_config(), &tampered)?;
        }
        let redis_failure_marker = Utf8PathBuf::from(format!(
            "{}.readiness-fail",
            paths.worker_root_config(&redis_runtime_key)
        ));
        fs::write_sensitive_file(&redis_failure_marker, "fail\n")?;

        let result = reconcile_gateway_runtimes(&paths).await;
        assert!(
            matches!(
                &result,
                Err(DaemonError::UnexpectedProtocolResponse { reason })
                    if reason.contains(&format!("php-worker-{redis_runtime_key}"))
            ),
            "the worker failure must survive gateway recovery, got {result:?}"
        );
        let gateway_pid_after = required_runtime_metadata_pid(&paths.gateway_runtime_metadata())?;
        let gateway_root_after = fs::read_to_string(&paths.gateway_root_config())?;
        if live_gateway {
            assert_eq!(gateway_pid_after, gateway_pid);
            assert!(process_is_alive(gateway_pid)?);
            assert_eq!(gateway_root_after, gateway_root);
        } else {
            assert_ne!(gateway_pid_after, gateway_pid);
            assert!(process_is_alive(gateway_pid_after)?);
            assert!(!gateway_root_after.contains("# tampered Gateway root"));
        }
        assert_eq!(fs::read_to_string(&source_fragment)?, source_content);

        fs::remove_file(&redis_failure_marker)?;
        stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
        stop_runtime_from_pid_file(&paths.worker_pid(&base_runtime_key)).await?;
        if paths.worker_pid(&redis_runtime_key).exists() {
            stop_runtime_from_pid_file(&paths.worker_pid(&redis_runtime_key)).await?;
        }
        runtime_guard.cleanup().await?;
    }

    Ok(())
}

#[tokio::test]
async fn failed_worker_readiness_does_not_cancel_siblings_or_reload_gateway() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let track = "8.4";
    let base_project =
        create_project_with_config(tempdir.path(), "base", "php:\n  version: \"8.4\"\n")?;
    let peer_project = create_project_with_config(
        tempdir.path(),
        "peer",
        "php: \"8.4\"\nhostnames: [old.acme.test]\n",
    )?;
    let redis_project = create_project_with_config(
        tempdir.path(),
        "redis",
        "php:\n  version: \"8.4\"\n  extensions: [redis]\n",
    )?;
    let xdebug_project = create_project_with_config(
        tempdir.path(),
        "xdebug",
        "php:\n  version: \"8.4\"\n  extensions: [xdebug]\n",
    )?;
    link_project_record(&paths, &base_project, "acme.test", Some(track))?;
    link_project_record(&paths, &peer_project, "changed.acme.test", Some(track))?;
    let base_record = Database::open(&paths)?
        .projects()?
        .into_iter()
        .find(|project| project.path == base_project)
        .ok_or_else(|| anyhow::anyhow!("missing base Project"))?;
    let peer_record = Database::open(&paths)?
        .projects()?
        .into_iter()
        .find(|project| project.path == peer_project)
        .ok_or_else(|| anyhow::anyhow!("missing peer Project"))?;

    let release_path = seed_installed_php_with_extensions(&paths, track, &["redis", "xdebug"])?;
    seed_installed_frankenphp_with_extensions(&paths, track, &release_path, &["redis", "xdebug"])?;
    write_fake_frankenphp(&release_path.join("bin/frankenphp"))?;
    ensure_fake_caddy(&paths)?;

    let base_runtime_key = state::php_runtime_key(track, &[])?;
    let redis_runtime_key = state::php_runtime_key(track, &["redis".to_owned()])?;
    let xdebug_runtime_key = state::php_runtime_key(track, &["xdebug".to_owned()])?;
    let ports = available_loopback_ports(5)?;
    let mut database = Database::open(&paths)?;
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[
            (&base_runtime_key, ports[2]),
            (&redis_runtime_key, ports[3]),
            (&xdebug_runtime_key, ports[4]),
        ],
    )?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let gateway_pid = required_runtime_metadata_pid(&paths.gateway_runtime_metadata())?;
    let base_worker_pid =
        required_runtime_metadata_pid(&paths.worker_runtime_metadata(&base_runtime_key))?;
    let gateway_root = read_test_bytes(paths.gateway_root_config())?;
    let gateway_load_count = fake_admin_load_bodies(&paths.gateway_root_config())?.len();

    let source_fragment = paths
        .worker_projects_config_dir(&base_runtime_key)
        .join(format!("{}.Caddyfile", base_record.id));
    let peer_fragment = paths
        .worker_projects_config_dir(&base_runtime_key)
        .join(format!("{}.Caddyfile", peer_record.id));
    let old_source_content = fs::read_to_string(&source_fragment)?;
    fs::write_sensitive_file(
        &base_record.config_path,
        "php:\n  version: \"8.4\"\n  extensions: [xdebug]\n",
    )?;
    fs::write_sensitive_file(
        &peer_project.join("pv.yml"),
        "php: \"8.4\"\nhostnames: [new.acme.test]\n",
    )?;
    link_project_record(&paths, &redis_project, "api.acme.test", Some(track))?;
    link_project_record(&paths, &xdebug_project, "other.test", Some(track))?;
    let redis_failure_marker = Utf8PathBuf::from(format!(
        "{}.readiness-fail",
        paths.worker_root_config(&redis_runtime_key)
    ));
    fs::write_sensitive_file(&redis_failure_marker, "fail\n")?;

    let xdebug_gate = Utf8PathBuf::from(format!(
        "{}.readiness-gate",
        paths.worker_root_config(&xdebug_runtime_key)
    ));
    let xdebug_probe = Utf8PathBuf::from(format!(
        "{}.readiness-probed",
        paths.worker_root_config(&xdebug_runtime_key)
    ));
    fs::write_sensitive_file(&xdebug_gate, "wait\n")?;
    let release_sibling_after_failure = async {
        timeout(Duration::from_secs(5), async {
            loop {
                let redis_failed = Database::open(&paths)?
                    .runtime_observed_states()?
                    .iter()
                    .any(|record| {
                        record.subject
                            == RuntimeSubject::PhpRuntimeWorker {
                                php_runtime_key: redis_runtime_key.clone(),
                            }
                            && record.status == RuntimeObservedStatus::Failed
                    });
                if redis_failed && xdebug_probe.exists() {
                    fs::remove_file(&xdebug_gate)?;
                    return Ok::<(), Error>(());
                }
                sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .context("failing sibling did not finish while xdebug was gated")?
    };
    let (result, release_result) = tokio::join!(
        reconcile_gateway_runtimes_with_readiness_timeout(&paths, Duration::from_secs(5)),
        release_sibling_after_failure,
    );
    release_result?;
    let xdebug_worker_pid =
        required_runtime_metadata_pid(&paths.worker_runtime_metadata(&xdebug_runtime_key))?;
    let gateway_still_running = process_is_alive(gateway_pid)?;
    let base_worker_still_running = process_is_alive(base_worker_pid)?;
    let xdebug_worker_running = process_is_alive(xdebug_worker_pid)?;
    let xdebug_metadata: Value = serde_json::from_str(&fs::read_to_string(
        &paths.worker_runtime_metadata(&xdebug_runtime_key),
    )?)?;
    let xdebug_status = Database::open(&paths)?
        .runtime_observed_states()?
        .into_iter()
        .find(|record| {
            record.subject
                == RuntimeSubject::PhpRuntimeWorker {
                    php_runtime_key: xdebug_runtime_key.clone(),
                }
        })
        .map(|record| record.status);
    let gateway_pid_after = required_runtime_metadata_pid(&paths.gateway_runtime_metadata())?;
    let gateway_root_after = read_test_bytes(paths.gateway_root_config())?;
    let gateway_load_count_after = fake_admin_load_bodies(&paths.gateway_root_config())?.len();
    let redis_runtime_removed = !paths.worker_pid(&redis_runtime_key).exists()
        && !paths.worker_runtime_metadata(&redis_runtime_key).exists();

    let source_content_after_failure = if source_fragment.exists() {
        Some(fs::read_to_string(&source_fragment)?)
    } else {
        None
    };
    let peer_content_after_failure = fs::read_to_string(&peer_fragment)?;
    stop_recorded_runtime_preserving_files(
        &paths,
        &paths.gateway_pid(),
        &paths.gateway_runtime_metadata(),
    )
    .await?;
    let verified_gateway_root = fs::read_to_string(&paths.gateway_root_config())?;
    fs::write_sensitive_file(
        &paths.gateway_root_config(),
        &format!("{verified_gateway_root}# unverified change\n"),
    )?;
    let unverified_failure = reconcile_gateway_runtimes(&paths).await;
    assert!(matches!(
        unverified_failure,
        Err(DaemonError::UnexpectedProtocolResponse { .. })
    ));
    wait_for_process_exit(gateway_pid).await?;
    // A failed worker with unprovable prior bytes and no live Gateway restarts the Gateway
    // from the desired plan instead of leaving it dead; the tampered bytes are never loaded.
    let restarted_gateway_pid = required_runtime_metadata_pid(&paths.gateway_runtime_metadata())?;
    assert_ne!(restarted_gateway_pid, gateway_pid);
    assert!(process_is_alive(restarted_gateway_pid)?);
    assert!(!fs::read_to_string(&paths.gateway_root_config())?.contains("# unverified change"));
    assert_eq!(fs::read_to_string(&source_fragment)?, old_source_content);

    fs::write_sensitive_file(&paths.gateway_root_config(), &verified_gateway_root)?;
    let recovery_failure = reconcile_gateway_runtimes(&paths).await;
    assert!(matches!(
        recovery_failure,
        Err(DaemonError::UnexpectedProtocolResponse { .. })
    ));
    let recovered_gateway_pid = required_runtime_metadata_pid(&paths.gateway_runtime_metadata())?;
    assert_ne!(recovered_gateway_pid, gateway_pid);
    assert!(process_is_alive(recovered_gateway_pid)?);
    assert_eq!(
        fs::read_to_string(&paths.gateway_root_config())?,
        verified_gateway_root
    );
    // The restarted Gateway is verifiably running the desired config, so normal hygiene
    // applies: the stale fragment of the moved project is collected instead of retained.
    assert!(!source_fragment.exists());
    assert_eq!(
        required_runtime_metadata_pid(&paths.worker_runtime_metadata(&xdebug_runtime_key))?,
        xdebug_worker_pid
    );
    assert!(
        Database::open(&paths)?
            .runtime_observed_states()?
            .iter()
            .any(|record| {
                record.subject == RuntimeSubject::Gateway
                    && record.status == RuntimeObservedStatus::Degraded
            })
    );

    fs::remove_file(&redis_failure_marker)?;
    reconcile_gateway_runtimes(&paths).await?;
    assert!(!source_fragment.exists());
    let committed_peer_content = fs::read_to_string(&peer_fragment)?;
    assert!(process_is_alive(base_worker_pid)?);
    assert_eq!(
        required_runtime_metadata_pid(&paths.worker_runtime_metadata(&xdebug_runtime_key))?,
        xdebug_worker_pid
    );
    stop_runtime_from_pid_file(&paths.worker_pid(&redis_runtime_key)).await?;
    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    stop_runtime_from_pid_file(&paths.worker_pid(&base_runtime_key)).await?;
    stop_runtime_from_pid_file(&paths.worker_pid(&xdebug_runtime_key)).await?;

    assert!(
        matches!(
        &result,
        Err(DaemonError::UnexpectedProtocolResponse { reason })
            if reason.contains(&format!("php-worker-{redis_runtime_key}"))
        ),
        "unexpected worker failure: {result:?}"
    );
    assert!(gateway_still_running);
    assert!(base_worker_still_running);
    assert!(xdebug_worker_running);
    assert!(xdebug_metadata["applied_config_fingerprint"].is_string());
    assert_ne!(xdebug_metadata["replacement_required"], true);
    assert_eq!(xdebug_status, Some(RuntimeObservedStatus::Running));
    assert_eq!(gateway_pid_after, gateway_pid);
    assert_eq!(gateway_root_after, gateway_root);
    assert_eq!(gateway_load_count_after, gateway_load_count);
    assert!(redis_runtime_removed);
    assert_eq!(
        source_content_after_failure.as_deref(),
        Some(old_source_content.as_str())
    );
    assert!(peer_content_after_failure.contains("old.acme.test"));
    assert!(peer_content_after_failure.contains("new.acme.test"));
    assert!(!committed_peer_content.contains("old.acme.test"));
    assert!(committed_peer_content.contains("new.acme.test"));

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_starts_gateway_without_linked_projects() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let release_path = tempdir.path().join("fake-frankenphp-release");
    let fake_frankenphp = release_path.join("bin/frankenphp");

    write_fake_frankenphp(&fake_frankenphp)?;

    let mut database = Database::open(&paths)?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-frankenphp-pv1",
        &release_path,
    )?;
    let ports = available_loopback_ports(2)?;
    seed_runtime_ports(&paths, &mut database, ports[0], ports[1], &[])?;
    drop(database);

    let summary = reconcile_gateway_runtimes(&paths).await?;

    assert_eq!(summary, GATEWAY_RECONCILIATION_SUMMARY);
    assert!(paths.gateway_pid().exists());
    assert!(!paths.worker_pid("8.4").exists());

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn unknown_pf_state_keeps_owned_gateway_running_after_bounded_public_probe() -> Result<()> {
    assert_uncertain_pf_state_preserves_gateway(
        GatewayPfRoutingState::Unknown,
        "unknown_pf_state_keeps_owned_gateway_running_after_bounded_public_probe",
    )
    .await
}

#[tokio::test]
async fn drifted_pf_state_keeps_owned_gateway_running_after_bounded_public_probe() -> Result<()> {
    assert_uncertain_pf_state_preserves_gateway(
        GatewayPfRoutingState::Drifted,
        "drifted_pf_state_keeps_owned_gateway_running_after_bounded_public_probe",
    )
    .await
}

async fn assert_uncertain_pf_state_preserves_gateway(
    pf_routing_state: GatewayPfRoutingState,
    snapshot_name: &str,
) -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let release_path = tempdir.path().join("fake-frankenphp-release");
    let fake_frankenphp = release_path.join("bin/frankenphp");

    write_fake_frankenphp(&fake_frankenphp)?;

    let mut database = Database::open(&paths)?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-frankenphp-pv1",
        &release_path,
    )?;
    let ports = available_loopback_ports(2)?;
    seed_runtime_ports(&paths, &mut database, ports[0], ports[1], &[])?;
    drop(database);
    ensure_fake_caddy(&paths)?;

    let started_at = Instant::now();
    let summary = reconcile_gateway_runtimes_with_pf_state_for_test(
        &paths,
        Duration::from_secs(2),
        pf_routing_state,
    )
    .await?;

    assert_eq!(summary, GATEWAY_RECONCILIATION_SUMMARY);
    assert!(started_at.elapsed() < Duration::from_secs(5));
    assert!(paths.gateway_pid().exists());
    assert_runtime_states_snapshot(
        snapshot_name,
        Database::open(&paths)?.runtime_observed_states()?,
    )?;

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn fresh_gateway_ownership_probe_failure_cleans_runtime_before_rollback() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let caddy_release = tempdir.path().join("fake-caddy-release");
    write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;

    let mut database = Database::open(&paths)?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    let ports = available_loopback_ports(2)?;
    seed_runtime_ports(&paths, &mut database, ports[0], ports[1], &[])?;
    drop(database);

    let previous_root = "previous gateway config\n";
    let admin_response_gate = tempdir.path().join("admin-response-release");
    fs::write_sensitive_file(&paths.gateway_root_config(), previous_root)?;
    write_fake_admin_control(
        &paths.gateway_root_config(),
        json!({"admin_response_gate": admin_response_gate.as_str()}),
    )?;

    let corrupt_runtime_metadata = async {
        timeout(Duration::from_secs(5), async {
            loop {
                let gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?;
                let admin_ready = fake_admin_requests(&paths.gateway_root_config())?
                    .iter()
                    .any(|request| request["method"] == "GET" && request["path"] == "/config/");
                if let Some(gateway_pid) = gateway_pid
                    && admin_ready
                {
                    let supervisor = ProcessSupervisor::new(paths.clone());
                    let gateway = supervisor
                        .adopt_recorded(&paths.gateway_pid(), &paths.gateway_runtime_metadata())?
                        .ok_or_else(|| anyhow::anyhow!("fresh Gateway was not adoptable"))?;
                    fs::write_sensitive_file(&paths.gateway_runtime_metadata(), "{")?;
                    fs::write_sensitive_file(&admin_response_gate, "")?;
                    return Ok::<_, Error>((gateway_pid, gateway));
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_error| anyhow::anyhow!("fresh Gateway did not reach gated admin readiness"))?
    };
    let (result, gateway) = tokio::join!(
        reconcile_gateway_runtimes_with_pf_state_for_test(
            &paths,
            Duration::from_secs(5),
            GatewayPfRoutingState::Unknown,
        ),
        corrupt_runtime_metadata,
    );
    let (gateway_pid, gateway) = gateway?;
    runtime_guard.capture_adopted(
        paths.gateway_pid(),
        paths.gateway_runtime_metadata(),
        gateway,
    );
    let gateway_was_alive = process_is_alive(gateway_pid)?;

    assert!(matches!(result, Err(DaemonError::Json(_))));
    assert!(!gateway_was_alive);
    assert!(!paths.gateway_pid().exists());
    assert!(!paths.gateway_runtime_metadata().exists());
    assert_eq!(
        fs::read_to_string(&paths.gateway_root_config())?,
        previous_root
    );
    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn fresh_gateway_exit_is_reported_before_the_readiness_timeout() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let caddy_release = tempdir.path().join("fast-exit-caddy-release");
    write_fast_exit_runtime(&caddy_release.join("bin/caddy"))?;

    let mut database = Database::open(&paths)?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fast-exit-caddy-pv1",
        &caddy_release,
    )?;
    let ports = available_loopback_ports(2)?;
    seed_runtime_ports(&paths, &mut database, ports[0], ports[1], &[])?;
    drop(database);

    let result = timeout(
        Duration::from_secs(5),
        reconcile_gateway_runtimes_with_readiness_timeout(&paths, Duration::from_secs(60)),
    )
    .await
    .context("Gateway reconciliation waited for the readiness timeout after Caddy exited")?;

    assert!(
        matches!(
            result,
            Err(DaemonError::UnexpectedProtocolResponse { ref reason })
                if reason.contains("runtime `gateway` exited before readiness was verified")
        ),
        "unexpected Gateway exit result: {result:?}"
    );
    assert!(!paths.gateway_pid().exists());
    assert!(!paths.gateway_runtime_metadata().exists());

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn fresh_worker_exit_is_reported_before_the_readiness_timeout() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let project_root = tempdir.path().join("acme");
    let frankenphp_release = tempdir.path().join("fast-exit-frankenphp-release");
    write_fast_exit_runtime(&frankenphp_release.join("bin/frankenphp"))?;
    create_project(
        &project_root,
        r#"php: "8.4"
document_root: public
"#,
    )?;

    let mut database = Database::open(&paths)?;
    database.link_project(LinkProjectInput {
        path: project_root.clone(),
        original_path: project_root.clone(),
        primary_hostname: "acme.test".to_owned(),
        config_path: project_root.join("pv.yml"),
        desired_php_track: Some("8.4".to_owned()),
        additional_hostnames: Vec::new(),
    })?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fast-exit-frankenphp-pv1",
        &frankenphp_release,
    )?;
    let ports = available_loopback_ports(3)?;
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[("8.4", ports[2])],
    )?;
    drop(database);
    ensure_fake_caddy(&paths)?;

    let result = timeout(
        Duration::from_secs(5),
        reconcile_gateway_runtimes_with_readiness_timeout(&paths, Duration::from_secs(60)),
    )
    .await
    .context("Gateway reconciliation waited for the readiness timeout after FrankenPHP exited")?;

    assert!(
        matches!(
            result,
            Err(DaemonError::UnexpectedProtocolResponse { ref reason })
                if reason.contains("runtime `php-worker-8.4` exited before readiness was verified")
        ),
        "unexpected worker exit result: {result:?}"
    );
    assert!(!paths.worker_pid("8.4").exists());
    assert!(!paths.worker_runtime_metadata("8.4").exists());
    assert!(!paths.gateway_pid().exists());

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_restores_generated_files_after_env_only_edit() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let project_root = tempdir.path().join("acme");
    let caddy_release = tempdir.path().join("fake-caddy-release");
    let frankenphp_release = tempdir.path().join("fake-frankenphp-release");

    write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;
    write_stateful_fake_frankenphp(&frankenphp_release.join("bin/frankenphp"))?;
    create_project(
        &project_root,
        r#"php: "8.4"
document_root: public
"#,
    )?;
    let mut database = Database::open(&paths)?;
    let project = database.link_project(LinkProjectInput {
        path: project_root.clone(),
        original_path: project_root.clone(),
        primary_hostname: "acme.test".to_owned(),
        config_path: project_root.join("pv.yml"),
        desired_php_track: Some("8.4".to_owned()),
        additional_hostnames: Vec::new(),
    })?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-frankenphp-pv1",
        &frankenphp_release,
    )?;
    let ports = available_loopback_ports(3)?;
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[("8.4", ports[2])],
    )?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let first_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected gateway runtime metadata"))?;
    let first_worker_pid = runtime_metadata_pid(&paths.worker_runtime_metadata("8.4"))?
        .ok_or_else(|| anyhow::anyhow!("expected worker runtime metadata"))?;
    let gateway_validations = fake_validator_spawns(&paths.gateway_root_config())?;
    let worker_validations = fake_validator_spawns(&paths.worker_root_config("8.4"))?;

    fs::write_sensitive_file(
        &project_root.join("pv.yml"),
        r#"php: "8.4"
document_root: public
env:
  APP_URL: "${project_url}"
"#,
    )?;

    reconcile_gateway_runtimes(&paths).await?;
    let second_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected gateway runtime metadata"))?;
    let second_worker_pid = runtime_metadata_pid(&paths.worker_runtime_metadata("8.4"))?
        .ok_or_else(|| anyhow::anyhow!("expected worker runtime metadata"))?;
    let gateway_metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.gateway_runtime_metadata())?)?;
    let worker_metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.worker_runtime_metadata("8.4"))?)?;
    let edited_gateway_root = "# externally edited generated Gateway config\n";
    let edited_worker_root = "# externally edited generated worker config\n";
    let gateway_fragment_path = paths
        .gateway_projects_config_dir()
        .join(format!("{}.Caddyfile", project.project.id));
    let worker_fragment_path = paths
        .worker_projects_config_dir("8.4")
        .join(format!("{}.Caddyfile", project.project.id));
    let edited_gateway_fragment = "# externally edited generated Gateway fragment\n";
    let edited_worker_fragment = "# externally edited generated worker fragment\n";
    fs::write_sensitive_file(&paths.gateway_root_config(), edited_gateway_root)?;
    fs::write_sensitive_file(&paths.worker_root_config("8.4"), edited_worker_root)?;
    fs::write_sensitive_file(&gateway_fragment_path, edited_gateway_fragment)?;
    fs::write_sensitive_file(&worker_fragment_path, edited_worker_fragment)?;
    let defaults = php_track_defaults(&paths, "8.4")?;
    fs::remove_file(defaults.php_ini())?;
    fs::delete_dir_all(defaults.conf_dir())?;

    reconcile_gateway_runtimes(&paths).await?;
    let third_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected gateway runtime metadata"))?;
    let third_worker_pid = runtime_metadata_pid(&paths.worker_runtime_metadata("8.4"))?
        .ok_or_else(|| anyhow::anyhow!("expected worker runtime metadata"))?;

    let restored_php_ini = fs::read_to_string(defaults.php_ini());
    let restored_conf_dir = fs::path_is_directory(defaults.conf_dir());
    fs::remove_file_if_exists(defaults.php_ini())?;
    fs::ensure_user_dir(defaults.php_ini())?;
    let invalid_defaults_result = reconcile_gateway_runtimes(&paths).await;
    let runtime_states = Database::open(&paths)?.runtime_observed_states()?;

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    stop_runtime_from_pid_file(&paths.worker_pid("8.4")).await?;

    assert_eq!(second_gateway_pid, first_gateway_pid);
    assert_eq!(second_worker_pid, first_worker_pid);
    assert_eq!(third_gateway_pid, first_gateway_pid);
    assert_eq!(third_worker_pid, first_worker_pid);
    assert_eq!(
        fake_validator_spawns(&paths.gateway_root_config())?,
        gateway_validations
    );
    assert_eq!(
        fake_validator_spawns(&paths.worker_root_config("8.4"))?,
        worker_validations
    );
    assert!(fake_admin_load_bodies(&paths.gateway_root_config())?.is_empty());
    assert!(fake_admin_load_bodies(&paths.worker_root_config("8.4"))?.is_empty());
    assert_ne!(
        fs::read_to_string(&paths.gateway_root_config())?,
        edited_gateway_root
    );
    assert_ne!(
        fs::read_to_string(&paths.worker_root_config("8.4"))?,
        edited_worker_root
    );
    assert_ne!(
        fs::read_to_string(&gateway_fragment_path)?,
        edited_gateway_fragment
    );
    assert_ne!(
        fs::read_to_string(&worker_fragment_path)?,
        edited_worker_fragment
    );
    assert!(gateway_metadata["applied_config_fingerprint"].is_string());
    assert!(worker_metadata["applied_config_fingerprint"].is_string());
    assert_ne!(gateway_metadata["replacement_required"], true);
    assert_ne!(worker_metadata["replacement_required"], true);
    assert_eq!(restored_php_ini?, PHP_TRACK_DEFAULT_INI);
    assert!(restored_conf_dir?);
    let Err(DaemonError::State(StateError::Filesystem { path, .. })) = invalid_defaults_result
    else {
        bail!("expected invalid PHP defaults path, got {invalid_defaults_result:?}");
    };
    assert_eq!(path, defaults.php_ini());
    let worker_status = runtime_states
        .iter()
        .find(|record| matches!(&record.subject, RuntimeSubject::PhpWorker { php_track } if php_track == "8.4"))
        .map(|record| record.status);
    assert_eq!(worker_status, Some(RuntimeObservedStatus::Failed));

    runtime_guard.cleanup().await?;

    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn unchanged_gateway_config_is_rehardened() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let project_root = tempdir.path().join("acme");
    let caddy_release = tempdir.path().join("fake-caddy-release");
    let frankenphp_release = tempdir.path().join("fake-frankenphp-release");
    let caddy_executable = caddy_release.join("bin/caddy");

    write_stateful_fake_caddy(&caddy_executable)?;
    write_stateful_fake_frankenphp(&frankenphp_release.join("bin/frankenphp"))?;
    create_project(&project_root, "php: \"8.4\"\ndocument_root: public\n")?;
    let mut database = Database::open(&paths)?;
    let project = database.link_project(LinkProjectInput {
        path: project_root.clone(),
        original_path: project_root.clone(),
        primary_hostname: "acme.test".to_owned(),
        config_path: project_root.join("pv.yml"),
        desired_php_track: Some("8.4".to_owned()),
        additional_hostnames: Vec::new(),
    })?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-frankenphp-pv1",
        &frankenphp_release,
    )?;
    let ports = available_loopback_ports(3)?;
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[("8.4", ports[2])],
    )?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let first_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected gateway runtime metadata"))?;
    let root_config = paths.gateway_root_config();
    let fragments_dir = paths.gateway_projects_config_dir();
    let fragment = fragments_dir.join(format!("{}.Caddyfile", project.project.id));
    let gateway_dir = root_config
        .parent()
        .ok_or_else(|| anyhow::anyhow!("expected Gateway config parent"))?;
    let root_bytes = read_test_bytes(root_config.clone())?;
    let fragment_bytes = read_test_bytes(fragment.clone())?;
    let sentinel = fragments_dir.join("notes.txt");
    fs::write_sensitive_file(&sentinel, "preserve me\n")?;
    let sentinel_bytes = read_test_bytes(sentinel.clone())?;
    let validation_count = fake_validator_spawns(&root_config)?;

    set_test_mode(&root_config, 0o644)?;
    set_test_mode(&fragment, 0o644)?;
    set_test_mode(gateway_dir, 0o755)?;
    set_test_mode(&fragments_dir, 0o755)?;
    write_failing_validator(&caddy_executable)?;

    let result = reconcile_gateway_runtimes(&paths).await;
    let second_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected gateway runtime metadata"))?;
    let root_after = read_test_bytes(root_config.clone())?;
    let fragment_after = read_test_bytes(fragment.clone())?;
    let validations_after = fake_validator_spawns(&root_config)?;
    let sentinel_after = if sentinel.exists() {
        Some(read_test_bytes(sentinel)?)
    } else {
        None
    };
    let admin_loads = fake_admin_load_bodies(&root_config)?;

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    stop_runtime_from_pid_file(&paths.worker_pid("8.4")).await?;

    assert_eq!(result?, GATEWAY_RECONCILIATION_SUMMARY);
    assert_eq!(second_gateway_pid, first_gateway_pid);
    assert_eq!(root_after, root_bytes);
    assert_eq!(fragment_after, fragment_bytes);
    assert_eq!(sentinel_after, Some(sentinel_bytes));
    assert_eq!(test_mode(&root_config)?, 0o600);
    assert_eq!(test_mode(&fragment)?, 0o600);
    assert_eq!(test_mode(gateway_dir)?, 0o700);
    assert_eq!(test_mode(&fragments_dir)?, 0o700);
    assert_eq!(validations_after, validation_count);
    assert!(admin_loads.is_empty());

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn targeted_project_reconciliation_touches_only_old_and_new_workers() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let acme = create_project_with_config(
        tempdir.path(),
        "acme",
        "php: \"8.4\"\ndocument_root: public\n",
    )?;
    let other = create_project_with_config(tempdir.path(), "other", "php: \"8.4\"\n")?;
    let unrelated = create_project_with_config(tempdir.path(), "api", "php: \"8.3\"\n")?;
    let caddy_release = tempdir.path().join("fake-caddy-release");
    let frankenphp_83_release = tempdir.path().join("fake-frankenphp-83-release");
    let frankenphp_84_release = tempdir.path().join("fake-frankenphp-84-release");
    let frankenphp_85_release = tempdir.path().join("fake-frankenphp-85-release");
    write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;
    for release in [
        &frankenphp_83_release,
        &frankenphp_84_release,
        &frankenphp_85_release,
    ] {
        write_stateful_fake_frankenphp(&release.join("bin/frankenphp"))?;
    }

    let mut database = Database::open(&paths)?;
    let acme = database
        .link_project(LinkProjectInput {
            path: acme.clone(),
            original_path: acme.clone(),
            primary_hostname: "acme.test".to_owned(),
            config_path: acme.join("pv.yml"),
            desired_php_track: Some("8.4".to_owned()),
            additional_hostnames: Vec::new(),
        })?
        .project;
    let mut other_project_id = None;
    let mut unrelated_project_id = None;
    for (project, hostname, track) in [
        (&other, "other.test", "8.4"),
        (&unrelated, "api.acme.test", "8.3"),
    ] {
        let linked = database.link_project(LinkProjectInput {
            path: project.clone(),
            original_path: project.clone(),
            primary_hostname: hostname.to_owned(),
            config_path: project.join("pv.yml"),
            desired_php_track: Some(track.to_owned()),
            additional_hostnames: Vec::new(),
        })?;
        if track == "8.3" {
            unrelated_project_id = Some(linked.project.id);
        } else {
            other_project_id = Some(linked.project.id);
        }
    }
    let other_project_id =
        other_project_id.ok_or_else(|| anyhow::anyhow!("expected same-runtime Project id"))?;
    let unrelated_project_id =
        unrelated_project_id.ok_or_else(|| anyhow::anyhow!("expected unrelated Project id"))?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    for (track, release) in [
        ("8.3", &frankenphp_83_release),
        ("8.4", &frankenphp_84_release),
        ("8.5", &frankenphp_85_release),
    ] {
        database.record_managed_resource_track_installed(
            "frankenphp",
            track,
            &format!("fake-frankenphp-{track}-pv1"),
            release,
        )?;
    }
    let ports = available_loopback_ports(5)?;
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[("8.3", ports[2]), ("8.4", ports[3]), ("8.5", ports[4])],
    )?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let gateway_pid = required_runtime_metadata_pid(&paths.gateway_runtime_metadata())?;
    let worker_83_pid = required_runtime_metadata_pid(&paths.worker_runtime_metadata("8.3"))?;
    let worker_84_pid = required_runtime_metadata_pid(&paths.worker_runtime_metadata("8.4"))?;
    let gateway_loads = fake_admin_load_bodies(&paths.gateway_root_config())?.len();
    let worker_83_requests = fake_admin_requests(&paths.worker_root_config("8.3"))?.len();
    let worker_84_loads = fake_admin_load_bodies(&paths.worker_root_config("8.4"))?.len();
    let unrelated_gateway_fragment_path = paths
        .gateway_projects_config_dir()
        .join(format!("{unrelated_project_id}.Caddyfile"));
    let unrelated_worker_fragment_path = paths
        .worker_projects_config_dir("8.3")
        .join(format!("{unrelated_project_id}.Caddyfile"));
    let other_worker_fragment_path = paths
        .worker_projects_config_dir("8.4")
        .join(format!("{other_project_id}.Caddyfile"));
    let unrelated_gateway_fragment = fs::read_to_string(&unrelated_gateway_fragment_path)?;
    let unrelated_worker_fragment = fs::read_to_string(&unrelated_worker_fragment_path)?;
    let other_worker_fragment = fs::read_to_string(&other_worker_fragment_path)?;

    let initial_observations = Database::open(&paths)?.runtime_observed_states()?;
    let mut database = Database::open(&paths)?;
    for subject in [
        RuntimeSubject::Gateway,
        RuntimeSubject::PhpWorker {
            php_track: "8.4".to_owned(),
        },
    ] {
        database.record_runtime_observed_snapshot(
            subject,
            RuntimeObservedStatus::Failed,
            Some("previous readiness failure"),
        )?;
    }
    drop(database);

    fs::write_sensitive_file(&unrelated.join("pv.yml"), "php: [\n")?;
    fs::write_sensitive_file(
        &acme.config_path,
        r#"php: "8.4"
document_root: public
env:
  APP_URL: "${project_url}"
"#,
    )?;
    reconcile_project_gateway_runtimes_for_test(
        &paths,
        &acme.id,
        Duration::from_secs(5),
        GatewayPfRoutingState::Inactive,
    )
    .await?;

    assert_eq!(
        required_runtime_metadata_pid(&paths.gateway_runtime_metadata())?,
        gateway_pid
    );
    assert_eq!(
        required_runtime_metadata_pid(&paths.worker_runtime_metadata("8.4"))?,
        worker_84_pid
    );
    assert_eq!(
        fake_admin_load_bodies(&paths.gateway_root_config())?.len(),
        gateway_loads
    );
    assert_eq!(
        fake_admin_load_bodies(&paths.worker_root_config("8.4"))?.len(),
        worker_84_loads
    );
    assert_eq!(
        fake_admin_requests(&paths.worker_root_config("8.3"))?.len(),
        worker_83_requests
    );
    assert_eq!(
        fs::read_to_string(&other_worker_fragment_path)?,
        other_worker_fragment
    );
    assert_eq!(
        fs::read_to_string(&unrelated_gateway_fragment_path)?,
        unrelated_gateway_fragment
    );
    assert_eq!(
        fs::read_to_string(&unrelated_worker_fragment_path)?,
        unrelated_worker_fragment
    );

    for subject in [
        RuntimeSubject::Gateway,
        RuntimeSubject::PhpWorker {
            php_track: "8.4".to_owned(),
        },
    ] {
        let observed = Database::open(&paths)?
            .runtime_observed_states()?
            .into_iter()
            .find(|record| record.subject == subject)
            .ok_or_else(|| anyhow::anyhow!("missing runtime observation"))?;
        let initial = initial_observations
            .iter()
            .find(|record| record.subject == subject)
            .ok_or_else(|| anyhow::anyhow!("missing initial runtime observation"))?;
        assert_eq!(observed.status, initial.status);
        assert_eq!(observed.message, initial.message);
    }

    fs::write_sensitive_file(&acme.path.join("web/index.php"), "<?php\n")?;
    fs::write_sensitive_file(&acme.config_path, "php: \"8.4\"\ndocument_root: web\n")?;
    reconcile_project_gateway_runtimes_for_test(
        &paths,
        &acme.id,
        Duration::from_secs(5),
        GatewayPfRoutingState::Inactive,
    )
    .await?;

    assert_eq!(
        fake_admin_load_bodies(&paths.worker_root_config("8.4"))?.len(),
        worker_84_loads + 1
    );
    assert_eq!(
        fake_admin_load_bodies(&paths.gateway_root_config())?.len(),
        gateway_loads
    );
    assert_eq!(
        fake_admin_requests(&paths.worker_root_config("8.3"))?.len(),
        worker_83_requests
    );
    assert_eq!(
        fs::read_to_string(&other_worker_fragment_path)?,
        other_worker_fragment
    );

    fs::write_sensitive_file(
        &acme.config_path,
        "php: \"8.4\"\ndocument_root: web\nhostnames:\n  - www.acme.test\n",
    )?;
    reconcile_project_gateway_runtimes_for_test(
        &paths,
        &acme.id,
        Duration::from_secs(5),
        GatewayPfRoutingState::Inactive,
    )
    .await?;

    assert_eq!(
        fake_admin_load_bodies(&paths.gateway_root_config())?.len(),
        gateway_loads + 1
    );
    assert_eq!(
        fake_admin_load_bodies(&paths.worker_root_config("8.4"))?.len(),
        worker_84_loads + 2
    );
    assert_eq!(
        fake_admin_requests(&paths.worker_root_config("8.3"))?.len(),
        worker_83_requests
    );

    fs::write_sensitive_file(
        &acme.config_path,
        "php: \"8.5\"\ndocument_root: web\nhostnames:\n  - www.acme.test\n",
    )?;
    let mut database = Database::open(&paths)?;
    database.replace_project_php_runtime(
        &acme.id,
        Some(&ProjectPhpRuntimeInput {
            track: "8.5".to_owned(),
            requested_extensions: Vec::new(),
            loaded_extensions: Vec::new(),
            ignored_extensions: Vec::new(),
        }),
    )?;
    drop(database);
    reconcile_project_gateway_runtimes_for_test(
        &paths,
        &acme.id,
        Duration::from_secs(5),
        GatewayPfRoutingState::Inactive,
    )
    .await?;

    assert_eq!(
        required_runtime_metadata_pid(&paths.gateway_runtime_metadata())?,
        gateway_pid
    );
    assert_eq!(
        required_runtime_metadata_pid(&paths.worker_runtime_metadata("8.3"))?,
        worker_83_pid
    );
    assert_eq!(
        required_runtime_metadata_pid(&paths.worker_runtime_metadata("8.4"))?,
        worker_84_pid
    );
    assert!(paths.worker_pid("8.5").exists());
    assert_eq!(
        fake_admin_load_bodies(&paths.gateway_root_config())?.len(),
        gateway_loads + 2
    );
    assert_eq!(
        fake_admin_load_bodies(&paths.worker_root_config("8.4"))?.len(),
        worker_84_loads + 3
    );
    assert_eq!(
        fake_admin_requests(&paths.worker_root_config("8.3"))?.len(),
        worker_83_requests
    );
    assert_eq!(
        fs::read_to_string(&unrelated_gateway_fragment_path)?,
        unrelated_gateway_fragment
    );
    assert_eq!(
        fs::read_to_string(&unrelated_worker_fragment_path)?,
        unrelated_worker_fragment
    );
    assert_eq!(
        fs::read_to_string(&other_worker_fragment_path)?,
        other_worker_fragment
    );

    let loads_before_readiness_failure =
        fake_admin_load_bodies(&paths.gateway_root_config())?.len();
    reconcile_project_gateway_runtimes_for_test(
        &paths,
        &acme.id,
        Duration::from_millis(250),
        GatewayPfRoutingState::Unknown,
    )
    .await?;
    assert_eq!(
        fake_admin_load_bodies(&paths.gateway_root_config())?.len(),
        loads_before_readiness_failure + 1,
    );

    let original_key = fs::read_to_string(&paths.ca_private_key())?;
    fs::write_sensitive_file(&paths.ca_private_key(), "invalid replacement key\n")?;
    write_failing_validator(&caddy_release.join("bin/caddy"))?;
    let changed_root = reconcile_project_gateway_runtimes_for_test(
        &paths,
        &acme.id,
        Duration::from_secs(5),
        GatewayPfRoutingState::Inactive,
    )
    .await;
    assert!(
        matches!(
            changed_root,
            Err(DaemonError::UnexpectedProtocolResponse { .. })
        ),
        "expected changed CA key to reach validation: {changed_root:?}"
    );
    fs::write_sensitive_file(&paths.ca_private_key(), &original_key)?;
    write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;
    fs::write_sensitive_file(&unrelated.join("pv.yml"), "php: \"8.3\"\n")?;

    for (delete_gateway, delete_worker) in [(true, false), (false, true), (true, true)] {
        let gateway_loads_before_removal =
            fake_admin_load_bodies(&paths.gateway_root_config())?.len();
        let gateway_fragment = paths
            .gateway_projects_config_dir()
            .join(format!("{}.Caddyfile", acme.id));
        let worker_fragment = paths
            .worker_projects_config_dir("8.5")
            .join(format!("{}.Caddyfile", acme.id));
        fs::write_sensitive_file(&acme.config_path, "serve: false\nphp: \"8.5\"\n")?;
        let mut database = Database::open(&paths)?;
        database.link_project_with_mode(
            LinkProjectInput {
                path: acme.path.clone(),
                original_path: acme.original_path.clone(),
                primary_hostname: "acme.test".to_owned(),
                config_path: acme.config_path.clone(),
                desired_php_track: Some("8.5".to_owned()),
                additional_hostnames: Vec::new(),
            },
            ProjectMode::ResourceOnly,
        )?;
        drop(database);
        if delete_gateway {
            fs::remove_file(&gateway_fragment)?;
        }
        if delete_worker {
            fs::remove_file(&worker_fragment)?;
        }
        reconcile_project_gateway_runtimes_for_test(
            &paths,
            &acme.id,
            Duration::from_secs(5),
            GatewayPfRoutingState::Inactive,
        )
        .await?;
        assert_eq!(
            fake_admin_load_bodies(&paths.gateway_root_config())?.len(),
            gateway_loads_before_removal + 1,
        );
        assert!(!paths.worker_pid("8.5").exists());
        assert!(!gateway_fragment.exists());
        assert!(!worker_fragment.exists());
        fs::write_sensitive_file(&acme.config_path, "php: \"8.5\"\n")?;
        let mut database = Database::open(&paths)?;
        database.link_project_with_mode(
            LinkProjectInput {
                path: acme.path.clone(),
                original_path: acme.original_path.clone(),
                primary_hostname: "acme.test".to_owned(),
                config_path: acme.config_path.clone(),
                desired_php_track: Some("8.5".to_owned()),
                additional_hostnames: Vec::new(),
            },
            ProjectMode::Served,
        )?;
        drop(database);
        reconcile_gateway_runtimes(&paths).await?;
        assert!(gateway_fragment.exists());
        assert!(worker_fragment.exists());
    }

    let mut database = Database::open(&paths)?;
    database.record_runtime_observed_snapshot(
        RuntimeSubject::PhpWorker {
            php_track: "8.3".to_owned(),
        },
        RuntimeObservedStatus::Failed,
        Some("stale worker cleanup failed"),
    )?;
    drop(database);
    reconcile_project_gateway_runtimes_for_test(
        &paths,
        &acme.id,
        Duration::from_secs(5),
        GatewayPfRoutingState::Inactive,
    )
    .await?;
    let worker_83_status = Database::open(&paths)?
        .runtime_observed_states()?
        .into_iter()
        .find(|state| {
            state.subject
                == RuntimeSubject::PhpWorker {
                    php_track: "8.3".to_owned(),
                }
        })
        .map(|state| state.status);
    assert_eq!(worker_83_status, Some(RuntimeObservedStatus::Running));

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    for track in ["8.3", "8.4", "8.5"] {
        stop_runtime_from_pid_file(&paths.worker_pid(track)).await?;
    }

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn targeted_project_reconciliation_does_not_activate_tampered_unrelated_fragments()
-> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let target = create_project_with_config(tempdir.path(), "target", "php: \"8.4\"\n")?;
    let peer = create_project_with_config(tempdir.path(), "peer", "php: \"8.4\"\n")?;
    let caddy_release = tempdir.path().join("fake-caddy-release");
    let frankenphp_release = tempdir.path().join("fake-frankenphp-release");
    write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;
    write_stateful_fake_frankenphp(&frankenphp_release.join("bin/frankenphp"))?;

    let mut database = Database::open(&paths)?;
    let target = database
        .link_project(LinkProjectInput {
            path: target.clone(),
            original_path: target.clone(),
            primary_hostname: "acme.test".to_owned(),
            config_path: target.join("pv.yml"),
            desired_php_track: Some("8.4".to_owned()),
            additional_hostnames: Vec::new(),
        })?
        .project;
    let peer = database
        .link_project(LinkProjectInput {
            path: peer.clone(),
            original_path: peer.clone(),
            primary_hostname: "other.test".to_owned(),
            config_path: peer.join("pv.yml"),
            desired_php_track: Some("8.4".to_owned()),
            additional_hostnames: Vec::new(),
        })?
        .project;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-frankenphp-8.4-pv1",
        &frankenphp_release,
    )?;
    let ports = available_loopback_ports(3)?;
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[("8.4", ports[2])],
    )?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let gateway_fragment_path = paths
        .gateway_projects_config_dir()
        .join(format!("{}.Caddyfile", peer.id));
    let worker_fragment_path = paths
        .worker_projects_config_dir("8.4")
        .join(format!("{}.Caddyfile", peer.id));
    let gateway_fragment = fs::read_to_string(&gateway_fragment_path)?;
    let worker_fragment = fs::read_to_string(&worker_fragment_path)?;
    let gateway_root = read_test_bytes(paths.gateway_root_config())?;
    let gateway_loads = fake_admin_load_bodies(&paths.gateway_root_config())?.len();
    let worker_loads = fake_admin_load_bodies(&paths.worker_root_config("8.4"))?.len();

    write_test_bytes(&paths.gateway_root_config(), &[0xff])?;
    reconcile_project_gateway_runtimes_for_test(
        &paths,
        &target.id,
        Duration::from_secs(5),
        GatewayPfRoutingState::Inactive,
    )
    .await?;

    assert_eq!(read_test_bytes(paths.gateway_root_config())?, gateway_root);
    assert_eq!(
        fake_admin_load_bodies(&paths.gateway_root_config())?.len(),
        gateway_loads
    );
    assert_eq!(
        fake_admin_load_bodies(&paths.worker_root_config("8.4"))?.len(),
        worker_loads
    );

    fs::write_sensitive_file(&gateway_fragment_path, "# tampered Gateway fragment\n")?;
    reconcile_project_gateway_runtimes_for_test(
        &paths,
        &target.id,
        Duration::from_secs(5),
        GatewayPfRoutingState::Inactive,
    )
    .await?;

    assert_eq!(
        fs::read_to_string(&gateway_fragment_path)?,
        gateway_fragment
    );
    assert_eq!(
        fake_admin_load_bodies(&paths.gateway_root_config())?.len(),
        gateway_loads
    );
    assert_eq!(
        fake_admin_load_bodies(&paths.worker_root_config("8.4"))?.len(),
        worker_loads
    );

    fs::write_sensitive_file(&gateway_fragment_path, &gateway_fragment)?;
    fs::write_sensitive_file(&worker_fragment_path, "# tampered worker fragment\n")?;
    reconcile_project_gateway_runtimes_for_test(
        &paths,
        &target.id,
        Duration::from_secs(5),
        GatewayPfRoutingState::Inactive,
    )
    .await?;

    assert_eq!(fs::read_to_string(&worker_fragment_path)?, worker_fragment);
    assert_eq!(
        fake_admin_load_bodies(&paths.gateway_root_config())?.len(),
        gateway_loads
    );
    assert_eq!(
        fake_admin_load_bodies(&paths.worker_root_config("8.4"))?.len(),
        worker_loads
    );

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    stop_runtime_from_pid_file(&paths.worker_pid("8.4")).await?;

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn targeted_project_reconciliation_promotes_split_peer_runtime_state() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let target = create_project_with_config(tempdir.path(), "target", "php: \"8.4\"\n")?;
    let peer = create_project_with_config(tempdir.path(), "peer", "php: \"8.4\"\n")?;
    let caddy_release = tempdir.path().join("fake-caddy-release");
    let frankenphp_84_release = tempdir.path().join("fake-frankenphp-84-release");
    let frankenphp_85_release = tempdir.path().join("fake-frankenphp-85-release");
    write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;
    write_stateful_fake_frankenphp(&frankenphp_84_release.join("bin/frankenphp"))?;
    write_stateful_fake_frankenphp(&frankenphp_85_release.join("bin/frankenphp"))?;

    let mut database = Database::open(&paths)?;
    let target = database
        .link_project(LinkProjectInput {
            path: target.clone(),
            original_path: target.clone(),
            primary_hostname: "acme.test".to_owned(),
            config_path: target.join("pv.yml"),
            desired_php_track: Some("8.4".to_owned()),
            additional_hostnames: Vec::new(),
        })?
        .project;
    let peer = database
        .link_project(LinkProjectInput {
            path: peer.clone(),
            original_path: peer.clone(),
            primary_hostname: "other.test".to_owned(),
            config_path: peer.join("pv.yml"),
            desired_php_track: Some("8.4".to_owned()),
            additional_hostnames: Vec::new(),
        })?
        .project;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    for (track, release) in [
        ("8.4", &frankenphp_84_release),
        ("8.5", &frankenphp_85_release),
    ] {
        database.record_managed_resource_track_installed(
            "frankenphp",
            track,
            &format!("fake-frankenphp-{track}-pv1"),
            release,
        )?;
    }
    let ports = available_loopback_ports(4)?;
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[("8.4", ports[2]), ("8.5", ports[3])],
    )?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    fs::write_sensitive_file(&peer.config_path, "php: \"8.5\"\n")?;
    let mut database = Database::open(&paths)?;
    database.replace_project_php_runtime(
        &peer.id,
        Some(&ProjectPhpRuntimeInput {
            track: "8.5".to_owned(),
            requested_extensions: Vec::new(),
            loaded_extensions: Vec::new(),
            ignored_extensions: Vec::new(),
        }),
    )?;
    drop(database);
    write_failing_config_validator(&caddy_release.join("bin/caddy"))?;

    let failed = reconcile_project_gateway_runtimes_for_test(
        &paths,
        &peer.id,
        Duration::from_secs(5),
        GatewayPfRoutingState::Inactive,
    )
    .await;

    assert!(matches!(
        failed,
        Err(DaemonError::UnexpectedProtocolResponse { reason })
            if reason.contains("Caddy config validation failed")
    ));
    write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;
    let peer_gateway_fragment_path = paths
        .gateway_projects_config_dir()
        .join(format!("{}.Caddyfile", peer.id));
    assert!(
        fs::read_to_string(&peer_gateway_fragment_path)?
            .contains(&format!("reverse_proxy 127.0.0.1:{}", ports[2]))
    );
    assert!(
        paths
            .worker_projects_config_dir("8.4")
            .join(format!("{}.Caddyfile", peer.id))
            .exists()
    );
    assert!(
        paths
            .worker_projects_config_dir("8.5")
            .join(format!("{}.Caddyfile", peer.id))
            .exists()
    );
    fs::write_sensitive_file(&peer.config_path, "php: [\n")?;
    fs::write_sensitive_file(&target.path.join("web/index.php"), "<?php\n")?;
    fs::write_sensitive_file(&target.config_path, "php: \"8.4\"\ndocument_root: web\n")?;
    reconcile_project_gateway_runtimes_for_test(
        &paths,
        &target.id,
        Duration::from_secs(5),
        GatewayPfRoutingState::Inactive,
    )
    .await?;

    assert!(
        fs::read_to_string(&peer_gateway_fragment_path)?
            .contains(&format!("reverse_proxy 127.0.0.1:{}", ports[2]))
    );
    assert!(
        paths
            .worker_projects_config_dir("8.4")
            .join(format!("{}.Caddyfile", peer.id))
            .exists()
    );
    assert!(
        !paths
            .worker_projects_config_dir("8.5")
            .join(format!("{}.Caddyfile", peer.id))
            .exists()
    );
    assert!(!paths.worker_pid("8.5").exists());

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    stop_runtime_from_pid_file(&paths.worker_pid("8.4")).await?;

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn targeted_project_reconciliation_uses_verified_fragment_snapshot() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let target = create_project_with_config(tempdir.path(), "target", "php: \"8.4\"\n")?;
    let peer = create_project_with_config(tempdir.path(), "peer", "php: \"8.4\"\n")?;
    let caddy_release = tempdir.path().join("fake-caddy-release");
    let frankenphp_release = tempdir.path().join("fake-frankenphp-release");
    write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;
    write_stateful_fake_frankenphp(&frankenphp_release.join("bin/frankenphp"))?;

    let mut database = Database::open(&paths)?;
    let target = database
        .link_project(LinkProjectInput {
            path: target.clone(),
            original_path: target.clone(),
            primary_hostname: "acme.test".to_owned(),
            config_path: target.join("pv.yml"),
            desired_php_track: Some("8.4".to_owned()),
            additional_hostnames: Vec::new(),
        })?
        .project;
    let peer = database
        .link_project(LinkProjectInput {
            path: peer.clone(),
            original_path: peer.clone(),
            primary_hostname: "other.test".to_owned(),
            config_path: peer.join("pv.yml"),
            desired_php_track: Some("8.4".to_owned()),
            additional_hostnames: Vec::new(),
        })?
        .project;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-frankenphp-8.4-pv1",
        &frankenphp_release,
    )?;
    let ports = available_loopback_ports(3)?;
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[("8.4", ports[2])],
    )?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let gateway_loads = fake_admin_load_bodies(&paths.gateway_root_config())?.len();
    let worker_loads = fake_admin_load_bodies(&paths.worker_root_config("8.4"))?.len();
    let peer_gateway_fragment_path = paths
        .gateway_projects_config_dir()
        .join(format!("{}.Caddyfile", peer.id));
    let peer_gateway_fragment = fs::read_to_string(&peer_gateway_fragment_path)?;
    write_fake_admin_control(
        &paths.worker_root_config("8.4"),
        json!({"load_delay_ms": 500}),
    )?;
    fs::write_sensitive_file(&target.path.join("web/index.php"), "<?php\n")?;
    fs::write_sensitive_file(&target.config_path, "php: \"8.4\"\ndocument_root: web\n")?;

    let mutation_paths = paths.clone();
    let mutation_fragment_path = peer_gateway_fragment_path.clone();
    let mutate_fragment = async move {
        timeout(Duration::from_secs(5), async {
            loop {
                if fake_admin_load_bodies(&mutation_paths.worker_root_config("8.4"))?.len()
                    > worker_loads
                {
                    return Ok::<(), Error>(());
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await??;
        fs::write_sensitive_file(&mutation_fragment_path, "# raced Gateway fragment\n")?;

        Ok::<(), Error>(())
    };
    let (reconciled, mutated) = tokio::join!(
        reconcile_project_gateway_runtimes_for_test(
            &paths,
            &target.id,
            Duration::from_secs(5),
            GatewayPfRoutingState::Inactive,
        ),
        mutate_fragment,
    );
    reconciled?;
    mutated?;

    assert_eq!(
        fake_admin_load_bodies(&paths.worker_root_config("8.4"))?.len(),
        worker_loads + 1
    );
    assert_eq!(
        fake_admin_load_bodies(&paths.gateway_root_config())?.len(),
        gateway_loads
    );
    assert_eq!(
        fs::read_to_string(&peer_gateway_fragment_path)?,
        peer_gateway_fragment
    );

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    stop_runtime_from_pid_file(&paths.worker_pid("8.4")).await?;

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn targeted_project_reconciliation_preserves_old_route_until_new_worker_is_ready()
-> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let project_root = create_project_with_config(
        tempdir.path(),
        "acme",
        "php: \"8.4\"\ndocument_root: public\n",
    )?;
    let caddy_release = tempdir.path().join("fake-caddy-release");
    let frankenphp_84_release = tempdir.path().join("fake-frankenphp-84-release");
    let frankenphp_85_release = tempdir.path().join("fake-frankenphp-85-release");
    write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;
    write_stateful_fake_frankenphp(&frankenphp_84_release.join("bin/frankenphp"))?;
    fs::write_sensitive_file(
        &frankenphp_85_release.join("bin/frankenphp"),
        "#!/bin/sh\nif [ \"$1\" = validate ]; then exit 0; fi\nexit 1\n",
    )?;
    set_executable(&frankenphp_85_release.join("bin/frankenphp"))?;

    let mut database = Database::open(&paths)?;
    let project = database
        .link_project(LinkProjectInput {
            path: project_root.clone(),
            original_path: project_root,
            primary_hostname: "acme.test".to_owned(),
            config_path: tempdir.path().join("acme/pv.yml"),
            desired_php_track: Some("8.4".to_owned()),
            additional_hostnames: Vec::new(),
        })?
        .project;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-frankenphp-8.4-pv1",
        &frankenphp_84_release,
    )?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.5",
        "fake-frankenphp-8.5-pv1",
        &frankenphp_85_release,
    )?;
    let ports = available_loopback_ports(4)?;
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[("8.4", ports[2]), ("8.5", ports[3])],
    )?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let gateway_pid = required_runtime_metadata_pid(&paths.gateway_runtime_metadata())?;
    let old_worker_pid = required_runtime_metadata_pid(&paths.worker_runtime_metadata("8.4"))?;
    let gateway_root = fs::read_to_string(&paths.gateway_root_config())?;
    let old_worker_root = fs::read_to_string(&paths.worker_root_config("8.4"))?;
    let gateway_loads = fake_admin_load_bodies(&paths.gateway_root_config())?.len();

    fs::write_sensitive_file(
        &project.config_path,
        "php: \"8.5\"\ndocument_root: public\n",
    )?;
    let mut database = Database::open(&paths)?;
    database.replace_project_php_runtime(
        &project.id,
        Some(&ProjectPhpRuntimeInput {
            track: "8.5".to_owned(),
            requested_extensions: Vec::new(),
            loaded_extensions: Vec::new(),
            ignored_extensions: Vec::new(),
        }),
    )?;
    drop(database);
    let failed = reconcile_project_gateway_runtimes_for_test(
        &paths,
        &project.id,
        Duration::from_millis(250),
        GatewayPfRoutingState::Inactive,
    )
    .await;

    assert!(
        matches!(
            &failed,
            Err(DaemonError::MissingProcessIdentity { name, .. })
                if name == "php-worker-8.5"
        ),
        "unexpected worker startup failure: {failed:?}"
    );
    assert!(process_is_alive(gateway_pid)?);
    assert!(process_is_alive(old_worker_pid)?);
    assert_eq!(
        fs::read_to_string(&paths.gateway_root_config())?,
        gateway_root
    );
    assert_eq!(
        fs::read_to_string(&paths.worker_root_config("8.4"))?,
        old_worker_root
    );
    assert_eq!(
        fake_admin_load_bodies(&paths.gateway_root_config())?.len(),
        gateway_loads
    );
    assert!(!paths.worker_pid("8.5").exists());

    write_stateful_fake_frankenphp(&frankenphp_85_release.join("bin/frankenphp"))?;
    reconcile_project_gateway_runtimes_for_test(
        &paths,
        &project.id,
        Duration::from_secs(5),
        GatewayPfRoutingState::Inactive,
    )
    .await
    .context("retrying PHP runtime transition")?;

    assert_eq!(
        required_runtime_metadata_pid(&paths.gateway_runtime_metadata())?,
        gateway_pid
    );
    assert!(paths.worker_pid("8.5").exists());
    wait_for_process_exit(old_worker_pid).await?;
    assert!(!paths.worker_pid("8.4").exists());
    assert!(!paths.worker_runtime_metadata("8.4").exists());

    let new_worker_pid = required_runtime_metadata_pid(&paths.worker_runtime_metadata("8.5"))?;
    let gateway_fragment_path = paths
        .gateway_projects_config_dir()
        .join(format!("{}.Caddyfile", project.id));
    let worker_fragment_path = paths
        .worker_projects_config_dir("8.5")
        .join(format!("{}.Caddyfile", project.id));
    fs::write_sensitive_file(&project.config_path, "serve: false\nphp: \"8.5\"\n")?;
    let mut database = Database::open(&paths)?;
    database.link_project_with_mode(
        LinkProjectInput {
            path: project.path.clone(),
            original_path: project.original_path.clone(),
            primary_hostname: "ignored.test".to_owned(),
            config_path: project.config_path.clone(),
            desired_php_track: Some("8.5".to_owned()),
            additional_hostnames: Vec::new(),
        },
        ProjectMode::ResourceOnly,
    )?;
    drop(database);
    write_fake_admin_control(
        &paths.gateway_root_config(),
        json!({"load_statuses": [422]}),
    )?;

    let failed = reconcile_project_gateway_runtimes_for_test(
        &paths,
        &project.id,
        Duration::from_millis(250),
        GatewayPfRoutingState::Inactive,
    )
    .await;

    assert!(matches!(
        failed,
        Err(DaemonError::CaddyAdmin(CaddyAdminError::LoadRejected {
            status: 422,
            ..
        }))
    ));
    assert!(gateway_fragment_path.exists());
    assert!(worker_fragment_path.exists());
    assert!(process_is_alive(new_worker_pid)?);

    write_fake_admin_control(
        &paths.gateway_root_config(),
        json!({"load_statuses": [200]}),
    )?;
    reconcile_project_gateway_runtimes_for_test(
        &paths,
        &project.id,
        Duration::from_secs(5),
        GatewayPfRoutingState::Inactive,
    )
    .await
    .context("retrying resource-only transition")?;

    assert!(!gateway_fragment_path.exists());
    assert!(!worker_fragment_path.exists());
    wait_for_process_exit(new_worker_pid).await?;
    assert!(!paths.worker_pid("8.5").exists());

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_takes_full_path_when_applied_fingerprint_is_missing() -> Result<()>
{
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let caddy_release = tempdir.path().join("fake-caddy-release");
    write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;

    let ports = available_loopback_ports(2)?;
    let mut database = Database::open(&paths)?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    seed_runtime_ports(&paths, &mut database, ports[0], ports[1], &[])?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let first_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected gateway runtime metadata"))?;
    let mut metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.gateway_runtime_metadata())?)?;
    let Some(metadata) = metadata.as_object_mut() else {
        bail!("runtime metadata must be an object");
    };
    metadata.remove("applied_config_fingerprint");
    fs::write_sensitive_file(
        &paths.gateway_runtime_metadata(),
        &serde_json::to_string(metadata)?,
    )?;
    let validation_count = fake_validator_spawns(&paths.gateway_root_config())?;

    reconcile_gateway_runtimes(&paths).await?;
    let second_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected gateway runtime metadata"))?;
    let metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.gateway_runtime_metadata())?)?;

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;

    assert_eq!(second_gateway_pid, first_gateway_pid);
    assert_eq!(
        fake_validator_spawns(&paths.gateway_root_config())?,
        validation_count + 1
    );
    assert_eq!(
        fake_admin_load_bodies(&paths.gateway_root_config())?.len(),
        1
    );
    assert!(metadata["applied_config_fingerprint"].is_string());
    assert_ne!(metadata["replacement_required"], true);

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_validates_changed_or_missing_ca_key() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let caddy_release = tempdir.path().join("fake-caddy-release");
    let executable = caddy_release.join("bin/caddy");
    write_stateful_fake_caddy(&executable)?;

    let ports = available_loopback_ports(2)?;
    let mut database = Database::open(&paths)?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    seed_runtime_ports(&paths, &mut database, ports[0], ports[1], &[])?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let initial_metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.gateway_runtime_metadata())?)?;
    let initial_root = read_test_bytes(paths.gateway_root_config())?;
    let initial_current = fake_admin_current_bytes(&paths.gateway_root_config())?;
    let initial_certificate = read_test_bytes(paths.ca_certificate())?;

    write_failing_validator(&executable)?;
    let unchanged_result = reconcile_gateway_runtimes(&paths).await;
    let unchanged_metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.gateway_runtime_metadata())?)?;
    let unchanged_loads = fake_admin_load_bodies(&paths.gateway_root_config())?;

    let mut outcomes = Vec::new();
    for replacement in [Some("invalid replacement key\n"), None] {
        if let Some(replacement) = replacement {
            fs::write_sensitive_file(&paths.ca_private_key(), replacement)?;
        } else {
            fs::remove_file(&paths.ca_private_key())?;
        }
        let result = reconcile_gateway_runtimes(&paths).await;
        let metadata: Value =
            serde_json::from_str(&fs::read_to_string(&paths.gateway_runtime_metadata())?)?;
        outcomes.push((
            result,
            metadata,
            Database::open(&paths)?.runtime_observed_states()?,
            read_test_bytes(paths.gateway_root_config())?,
            fake_admin_current_bytes(&paths.gateway_root_config())?,
            read_test_bytes(paths.ca_certificate())?,
            fake_admin_load_bodies(&paths.gateway_root_config())?,
        ));
    }

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;

    assert_eq!(unchanged_result?, GATEWAY_RECONCILIATION_SUMMARY);
    assert_eq!(unchanged_metadata["pid"], initial_metadata["pid"]);
    assert_eq!(
        unchanged_metadata["applied_config_fingerprint"],
        initial_metadata["applied_config_fingerprint"]
    );
    assert!(unchanged_loads.is_empty());
    assert!(initial_metadata["applied_config_fingerprint"].is_string());

    for (result, metadata, runtime_states, root, current, certificate, loads) in outcomes {
        let Err(DaemonError::UnexpectedProtocolResponse { reason }) = result else {
            bail!("expected CA-key change to reach the rejecting validator, got {result:?}");
        };
        let mut settings = Settings::clone_current();
        settings.add_filter(tempdir.path().as_str(), "<tempdir>");
        settings.add_filter(r"\.candidate\.\d+\.\d+\.tmp", ".candidate.<id>.tmp");
        settings.bind(|| {
            allow_duplicates! {
                assert_debug_snapshot!(reason, @r#""Caddy config validation failed for <tempdir>/home/.pv/config/gateway/Caddyfile.candidate.<id>.tmp: status=exit status: 42; stdout=validator stdout\n; stderr=validator stderr\n""#);
            }
        });
        let gateway_status = runtime_states
            .iter()
            .find(|record| record.subject == RuntimeSubject::Gateway)
            .map(|record| record.status);
        assert_eq!(gateway_status, Some(RuntimeObservedStatus::Failed));
        assert_eq!(metadata["pid"], initial_metadata["pid"]);
        assert_eq!(
            metadata["applied_config_fingerprint"],
            initial_metadata["applied_config_fingerprint"]
        );
        assert_ne!(metadata["replacement_required"], true);
        assert_eq!(root, initial_root);
        assert_eq!(current, initial_current);
        assert_eq!(certificate, initial_certificate);
        assert!(loads.is_empty());
    }

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_keeps_pending_state_when_applied_fingerprint_commit_fails()
-> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let caddy_release = tempdir.path().join("fake-caddy-release");
    let admin_response_gate = tempdir.path().join("admin-response-release");
    write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;

    let ports = available_loopback_ports(2)?;
    let mut database = Database::open(&paths)?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    seed_runtime_ports(&paths, &mut database, ports[0], ports[1], &[])?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let first_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected gateway runtime metadata"))?;
    let desired_root = read_test_bytes(paths.gateway_root_config())?;
    let mut metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.gateway_runtime_metadata())?)?;
    let Some(metadata) = metadata.as_object_mut() else {
        bail!("runtime metadata must be an object");
    };
    metadata.remove("applied_config_fingerprint");
    fs::write_sensitive_file(
        &paths.gateway_runtime_metadata(),
        &serde_json::to_string(metadata)?,
    )?;
    write_fake_admin_control(
        &paths.gateway_root_config(),
        json!({"admin_response_gate": admin_response_gate.as_str()}),
    )?;

    let block_final_metadata_write = async {
        timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(content) = fs::read_to_string(&paths.gateway_runtime_metadata())
                    && let Ok(metadata) = serde_json::from_str::<Value>(&content)
                    && metadata["replacement_required"] == true
                    && metadata["applied_config_fingerprint"].is_null()
                {
                    state::testing::fail_next_sensitive_write(paths.gateway_runtime_metadata());
                    fs::write_sensitive_file(&admin_response_gate, "")?;
                    return Ok::<(), Error>(());
                }
                sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .map_err(|_error| anyhow::anyhow!("pending runtime metadata was not recorded"))?
    };
    let (result, block_result) = tokio::join!(
        reconcile_gateway_runtimes_with_pf_state_for_test(
            &paths,
            Duration::from_secs(2),
            GatewayPfRoutingState::Unknown,
        ),
        block_final_metadata_write,
    );
    block_result?;

    let pending_metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.gateway_runtime_metadata())?)?;
    assert!(
        matches!(result, Err(DaemonError::State(_))),
        "unexpected reconciliation result: {result:?}"
    );
    assert_eq!(read_test_bytes(paths.gateway_root_config())?, desired_root);
    assert_eq!(pending_metadata["replacement_required"], true);
    assert!(pending_metadata["applied_config_fingerprint"].is_null());

    reconcile_gateway_runtimes(&paths).await?;
    let replacement_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected replacement gateway runtime metadata"))?;
    let replacement_metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.gateway_runtime_metadata())?)?;

    wait_for_process_exit(first_gateway_pid).await?;
    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;

    assert_ne!(replacement_gateway_pid, first_gateway_pid);
    assert_ne!(replacement_metadata["replacement_required"], true);
    assert!(replacement_metadata["applied_config_fingerprint"].is_string());

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_replaces_legacy_runtime_identities_once() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let project_root = tempdir.path().join("acme");
    let release_path = tempdir.path().join("fake-frankenphp-release");
    let fake_frankenphp = release_path.join("bin/frankenphp");

    write_fake_frankenphp(&fake_frankenphp)?;
    create_project(
        &project_root,
        r#"php: "8.4"
document_root: public
"#,
    )?;

    let mut database = Database::open(&paths)?;
    database.link_project(LinkProjectInput {
        path: project_root.clone(),
        original_path: project_root.clone(),
        primary_hostname: "acme.test".to_owned(),
        config_path: project_root.join("pv.yml"),
        desired_php_track: Some("8.4".to_owned()),
        additional_hostnames: Vec::new(),
    })?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-frankenphp-pv1",
        &release_path,
    )?;
    let ports = available_loopback_ports(3)?;
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[("8.4", ports[2])],
    )?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let first_gateway_pid =
        runtime_guard.capture(paths.gateway_pid(), paths.gateway_runtime_metadata())?;
    let first_worker_pid = runtime_guard.capture(
        paths.worker_pid("8.4"),
        paths.worker_runtime_metadata("8.4"),
    )?;
    replace_runtime_metadata_identity(&paths.gateway_runtime_metadata(), "gateway", "core")?;
    replace_runtime_metadata_identity(&paths.worker_runtime_metadata("8.4"), "php-worker", "8.4")?;

    reconcile_gateway_runtimes(&paths).await?;
    let second_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected gateway runtime metadata"))?;
    let second_worker_pid = runtime_metadata_pid(&paths.worker_runtime_metadata("8.4"))?
        .ok_or_else(|| anyhow::anyhow!("expected worker runtime metadata"))?;

    reconcile_gateway_runtimes(&paths).await?;
    let stable_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected stable gateway runtime metadata"))?;
    let stable_worker_pid = runtime_metadata_pid(&paths.worker_runtime_metadata("8.4"))?
        .ok_or_else(|| anyhow::anyhow!("expected stable worker runtime metadata"))?;

    wait_for_process_exit(first_gateway_pid).await?;
    wait_for_process_exit(first_worker_pid).await?;
    assert_ne!(second_gateway_pid, first_gateway_pid);
    assert_ne!(second_worker_pid, first_worker_pid);
    assert_eq!(stable_gateway_pid, second_gateway_pid);
    assert_eq!(stable_worker_pid, second_worker_pid);

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    stop_runtime_from_pid_file(&paths.worker_pid("8.4")).await?;
    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_replaces_legacy_admin_off_process_before_admin_contact()
-> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let caddy_release = tempdir.path().join("fake-caddy-release");
    let legacy_release = tempdir.path().join("fake-caddy-legacy-release");
    let caddy_executable = caddy_release.join("bin/caddy");
    let legacy_executable = legacy_release.join("bin/caddy");
    let legacy_server = Utf8PathBuf::from(format!("{legacy_executable}.server.py"));

    write_fake_caddy(&caddy_executable)?;
    write_fake_caddy_legacy(&legacy_executable)?;
    set_executable(&legacy_server)?;

    let mut database = Database::open(&paths)?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    let port_reservations = reserve_loopback_ports_in_range(2, 40_000, 44_999)?;
    let ports = loopback_ports(&port_reservations)?;
    seed_runtime_ports(&paths, &mut database, ports[0], ports[1], &[])?;
    drop(database);

    fs::write_sensitive_file(
        &paths.gateway_root_config(),
        &format!("{{\n    admin off\n    http_port {}\n}}\n", ports[0]),
    )?;
    let mut legacy_spec = gateway_process_spec(&paths, &CaddyCliCommand::caddy(&legacy_executable));
    // Keep the recorded fixture identity stable while macOS schedules parallel tests.
    legacy_spec.command = legacy_server;
    legacy_spec.arguments = vec![paths.gateway_root_config().to_string()];
    legacy_spec.resource_name = "gateway".to_owned();
    legacy_spec.track = "core".to_owned();
    let supervisor = ProcessSupervisor::new(paths.clone());
    drop(port_reservations);
    let mut legacy_process = supervisor.start(legacy_spec).await?;
    assert!(
        supervisor
            .adopt_recorded(&paths.gateway_pid(), &paths.gateway_runtime_metadata(),)?
            .is_some(),
        "legacy fixture must be adoptable before reconciliation"
    );

    let summary = reconcile_gateway_runtimes_with_pf_state_for_test(
        &paths,
        Duration::from_secs(1),
        GatewayPfRoutingState::Inactive,
    )
    .await?;
    assert_eq!(summary, GATEWAY_RECONCILIATION_SUMMARY);
    timeout(Duration::from_secs(1), async {
        loop {
            if legacy_process.has_exited()? {
                return Ok::<(), DaemonError>(());
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_elapsed| anyhow::anyhow!("legacy Gateway was not reaped after replacement"))??;

    let replacement_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected replacement gateway metadata"))?;
    let root_config = fs::read_to_string(&paths.gateway_root_config())?;
    assert!(root_config.contains(&format!(
        "admin \"unix/{}|0600\"",
        paths.gateway_admin_socket()
    )));
    assert!(!root_config.contains("admin off"));

    reconcile_gateway_runtimes_with_pf_state_for_test(
        &paths,
        Duration::from_secs(1),
        GatewayPfRoutingState::Inactive,
    )
    .await?;
    let stable_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected stable gateway metadata"))?;
    assert_eq!(stable_pid, replacement_pid);

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_replaces_dead_gateway_process() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let project_root = tempdir.path().join("acme");
    let release_path = tempdir.path().join("fake-frankenphp-release");
    let fake_frankenphp = release_path.join("bin/frankenphp");

    write_fake_frankenphp(&fake_frankenphp)?;
    create_project(
        &project_root,
        r#"php: "8.4"
document_root: public
"#,
    )?;

    let mut database = Database::open(&paths)?;
    database.link_project(LinkProjectInput {
        path: project_root.clone(),
        original_path: project_root.clone(),
        primary_hostname: "acme.test".to_owned(),
        config_path: project_root.join("pv.yml"),
        desired_php_track: None,
        additional_hostnames: Vec::new(),
    })?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-frankenphp-pv1",
        &release_path,
    )?;
    let ports = available_loopback_ports(3)?;
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[("8.4", ports[2])],
    )?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let first_metadata = state::testing::read_to_string(&paths.gateway_runtime_metadata())?;
    let first_metadata_json: serde_json::Value = serde_json::from_str(&first_metadata)?;
    let first_gateway_pid = metadata_pid(&first_metadata_json)?;
    let first_gateway = ProcessSupervisor::new(paths.clone())
        .adopt_recorded(&paths.gateway_pid(), &paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("first Gateway was not adoptable"))?;
    assert_eq!(first_gateway.pid(), first_gateway_pid);
    first_gateway.stop(Duration::from_secs(1)).await?;

    reconcile_gateway_runtimes(&paths).await?;
    let second_metadata = state::testing::read_to_string(&paths.gateway_runtime_metadata())?;

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    stop_runtime_from_pid_file(&paths.worker_pid("8.4")).await?;

    assert_ne!(first_metadata, second_metadata);

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_rejects_unverified_live_gateway_listener() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let project_root = tempdir.path().join("acme");
    let release_path = tempdir.path().join("fake-frankenphp-release");
    let fake_frankenphp = release_path.join("bin/frankenphp");

    write_fake_frankenphp(&fake_frankenphp)?;
    create_project(
        &project_root,
        r#"php: "8.4"
document_root: public
"#,
    )?;

    let mut database = Database::open(&paths)?;
    database.link_project(LinkProjectInput {
        path: project_root.clone(),
        original_path: project_root.clone(),
        primary_hostname: "acme.test".to_owned(),
        config_path: project_root.join("pv.yml"),
        desired_php_track: None,
        additional_hostnames: Vec::new(),
    })?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-frankenphp-pv1",
        &release_path,
    )?;
    let ports = available_loopback_ports(3)?;
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[("8.4", ports[2])],
    )?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let first_metadata = state::testing::read_to_string(&paths.gateway_runtime_metadata())?;
    let first_metadata_json: serde_json::Value = serde_json::from_str(&first_metadata)?;
    let first_gateway_pid = metadata_pid(&first_metadata_json)?;
    assert_eq!(
        runtime_guard.capture(paths.gateway_pid(), paths.gateway_runtime_metadata())?,
        first_gateway_pid
    );
    fs::delete_file(&paths.gateway_runtime_metadata())?;

    let result = reconcile_gateway_runtimes(&paths).await;
    assert!(matches!(
        result,
        Err(DaemonError::UnexpectedProtocolResponse { reason })
            if reason.contains("is listening but no PV-owned process could be verified")
    ));

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_bounds_foreign_https_listener_probe() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let project_root = tempdir.path().join("acme");
    let release_path = tempdir.path().join("fake-frankenphp-release");
    let fake_frankenphp = release_path.join("bin/frankenphp");

    write_fake_frankenphp(&fake_frankenphp)?;
    create_project(
        &project_root,
        r#"php: "8.4"
document_root: public
"#,
    )?;

    let http_listener = TcpListener::bind("127.0.0.1:0")?;
    let https_listener = TcpListener::bind("127.0.0.1:0")?;
    let http_port = http_listener.local_addr()?.port();
    let https_port = https_listener.local_addr()?.port();
    https_listener.set_nonblocking(true)?;
    let https_server = tokio::spawn(async move {
        let (_stream, _address) = tokio::net::TcpListener::from_std(https_listener)?
            .accept()
            .await?;
        sleep(Duration::from_secs(30)).await;

        Ok::<(), std::io::Error>(())
    });

    let mut database = Database::open(&paths)?;
    database.link_project(LinkProjectInput {
        path: project_root.clone(),
        original_path: project_root.clone(),
        primary_hostname: "acme.test".to_owned(),
        config_path: project_root.join("pv.yml"),
        desired_php_track: None,
        additional_hostnames: Vec::new(),
    })?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-frankenphp-pv1",
        &release_path,
    )?;
    let worker_port = available_loopback_ports(1)?[0];
    seed_runtime_ports(
        &paths,
        &mut database,
        http_port,
        https_port,
        &[("8.4", worker_port)],
    )?;
    drop(database);

    let result = timeout(
        Duration::from_secs(5),
        reconcile_gateway_runtimes_with_readiness_timeout(&paths, Duration::from_millis(100)),
    )
    .await;

    https_server.abort();
    drop(http_listener);
    if paths.worker_pid("8.4").exists() {
        stop_runtime_from_pid_file(&paths.worker_pid("8.4")).await?;
    }
    if paths.gateway_pid().exists() {
        stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    }

    assert!(
        result.is_ok(),
        "foreign Gateway listener probe should be bounded"
    );

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn frankenphp_config_validation_timeout_stops_validator_process_group() -> Result<()> {
    let tempdir = tempdir()?;
    let validator = tempdir.path().join("hanging-validator");
    let validator_child_pid = tempdir.path().join("validator-child.pid");
    let config_path = tempdir.path().join("Caddyfile");

    write_hanging_frankenphp_validator(&validator, &validator_child_pid)?;
    fs::write_sensitive_file(&config_path, "{}\n")?;

    let result = validate_config(
        &CaddyCliCommand::frankenphp(&validator),
        &config_path,
        &BTreeMap::new(),
    )
    .await;

    assert!(matches!(
        result,
        Err(DaemonError::ProtocolTimedOut {
            phase: "FrankenPHP config validation"
        })
    ));

    let sleep_pid = state::testing::read_to_string(&validator_child_pid)?
        .trim()
        .parse::<u32>()?;
    wait_for_process_exit(sleep_pid).await?;

    Ok(())
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn config_validation_stops_descendant_that_retains_output_after_leader_exit() -> Result<()> {
    let tempdir = tempdir()?;
    let validator = tempdir.path().join("detached-output-validator");
    let validator_child_pid = tempdir.path().join("validator-child.pid");
    let config_path = tempdir.path().join("Caddyfile");

    fs::write_sensitive_file(
        &validator,
        &format!(
            r#"#!/bin/sh
set -eu

if [ "$1" = "validate" ]; then
  sleep 30 &
  echo "$!" > {}
  exit 0
fi

exit 2
"#,
            shell_single_quoted(validator_child_pid.as_str())
        ),
    )?;
    set_executable(&validator)?;
    fs::write_sensitive_file(&config_path, "{}\n")?;

    timeout(
        Duration::from_secs(5),
        validate_config(
            &CaddyCliCommand::frankenphp(&validator),
            &config_path,
            &BTreeMap::new(),
        ),
    )
    .await??;

    let child_pid = state::testing::read_to_string(&validator_child_pid)?
        .trim()
        .parse::<u32>()?;
    wait_for_process_exit(child_pid).await?;

    Ok(())
}

#[tokio::test]
async fn retired_worker_cleanup_removes_runtime_identity() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let project_root = tempdir.path().join("acme");
    let surviving_project_root = tempdir.path().join("other");
    let release_path = tempdir.path().join("fake-frankenphp-release");
    let gateway_release_path = tempdir.path().join("fake-frankenphp-gateway-release");
    let fake_frankenphp = release_path.join("bin/frankenphp");
    let gateway_frankenphp = gateway_release_path.join("bin/frankenphp");

    write_fake_frankenphp(&fake_frankenphp)?;
    write_fake_frankenphp(&gateway_frankenphp)?;
    create_project(
        &project_root,
        r#"php: "8.4"
document_root: public
"#,
    )?;
    create_project(
        &surviving_project_root,
        r#"php: "8.3"
document_root: public
"#,
    )?;

    let mut database = Database::open(&paths)?;
    let project = database.link_project(LinkProjectInput {
        path: project_root.clone(),
        original_path: project_root.clone(),
        primary_hostname: "acme.test".to_owned(),
        config_path: project_root.join("pv.yml"),
        desired_php_track: Some("8.4".to_owned()),
        additional_hostnames: Vec::new(),
    })?;
    let surviving_project = database.link_project(LinkProjectInput {
        path: surviving_project_root.clone(),
        original_path: surviving_project_root.clone(),
        primary_hostname: "other.test".to_owned(),
        config_path: surviving_project_root.join("pv.yml"),
        desired_php_track: Some("8.3".to_owned()),
        additional_hostnames: Vec::new(),
    })?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-frankenphp-pv1",
        &release_path,
    )?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.3",
        "fake-frankenphp-83-pv1",
        &gateway_release_path,
    )?;
    let ports = available_loopback_ports(4)?;
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[("8.3", ports[2]), ("8.4", ports[3])],
    )?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let retiring_worker_metadata =
        state::testing::read_to_string(&paths.worker_runtime_metadata("8.4"))?;
    let retiring_worker_metadata_json: serde_json::Value =
        serde_json::from_str(&retiring_worker_metadata)?;
    let retiring_worker_pid = metadata_pid(&retiring_worker_metadata_json)?;
    let surviving_worker_pid =
        required_runtime_metadata_pid(&paths.worker_runtime_metadata("8.3"))?;
    assert!(process_is_alive(surviving_worker_pid)?);
    let retiring_worker_config_dir = paths.worker_config_dir("8.4");

    let mut database = Database::open(&paths)?;
    state::testing::transaction(&mut database, |transaction| {
        transaction
            .execute(
                "DELETE FROM managed_resource_tracks WHERE resource_name = 'frankenphp' AND track = '8.4'",
                [],
            )
            .map(|_deleted| ())
    })?;
    database.unlink_project(&project.project.id)?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;

    wait_for_process_exit(retiring_worker_pid).await?;
    let targeted_reconcile_result = reconcile_project_gateway_runtimes_for_test(
        &paths,
        &surviving_project.project.id,
        Duration::from_secs(5),
        GatewayPfRoutingState::Inactive,
    )
    .await;
    let daemon_log_result = state::fs::read_to_string(&paths.daemon_log());

    let surviving_worker_cleanup_result =
        stop_runtime_from_pid_file(&paths.worker_pid("8.3")).await;
    let gateway_cleanup_result = stop_runtime_from_pid_file(&paths.gateway_pid()).await;

    targeted_reconcile_result?;
    let daemon_log = daemon_log_result?;
    surviving_worker_cleanup_result?;
    gateway_cleanup_result?;

    let targeted_phase_events = daemon_log
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|event| {
            event["event"] == "reconciliation_phase_completed"
                && event["job_id"] == "targeted-gateway-test"
        })
        .map(|event| {
            (
                event["phase"].as_str().unwrap_or_default().to_owned(),
                event["subject"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        targeted_phase_events,
        vec![
            ("workers".to_owned(), "target_project".to_owned()),
            ("gateway".to_owned(), "target_project".to_owned()),
            ("workers".to_owned(), "stale_workers".to_owned()),
        ]
    );

    assert!(!process_is_alive(retiring_worker_pid)?);
    assert!(!paths.worker_pid("8.4").exists());
    assert!(!paths.worker_runtime_metadata("8.4").exists());
    assert!(!paths.worker_admin_socket("8.4").exists());
    assert!(!retiring_worker_config_dir.exists());

    let database = Database::open(&paths)?;
    let assigned_ports = database.assigned_ports()?;
    assert!(!assigned_ports.iter().any(|port| matches!(
        &port.owner,
        PortOwner::PhpWorker { php_runtime_key } if php_runtime_key == "8.4"
    )));

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_preserves_project_fragments_for_invalid_project_config()
-> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let project_root = tempdir.path().join("acme");
    let caddy_release = tempdir.path().join("fake-caddy-release");
    let frankenphp_release = tempdir.path().join("fake-frankenphp-release");

    write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;
    write_stateful_fake_frankenphp(&frankenphp_release.join("bin/frankenphp"))?;
    create_project(
        &project_root,
        r#"php: "8.4"
document_root: public
hostnames:
  - api.acme.test
"#,
    )?;

    let mut database = Database::open(&paths)?;
    let project = database.link_project(LinkProjectInput {
        path: project_root.clone(),
        original_path: project_root.clone(),
        primary_hostname: "acme.test".to_owned(),
        config_path: project_root.join("pv.yml"),
        desired_php_track: Some("8.4".to_owned()),
        additional_hostnames: vec!["api.acme.test".to_owned()],
    })?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-frankenphp-pv1",
        &frankenphp_release,
    )?;
    let ports = available_loopback_ports(3)?;
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[("8.4", ports[2])],
    )?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let gateway_fragment_path = paths
        .gateway_projects_config_dir()
        .join(format!("{}.Caddyfile", project.project.id));
    let worker_fragment_path = paths
        .worker_projects_config_dir("8.4")
        .join(format!("{}.Caddyfile", project.project.id));
    let gateway_fragment = fs::read_to_string(&gateway_fragment_path)?;
    let worker_fragment = fs::read_to_string(&worker_fragment_path)?;

    fs::write_sensitive_file(&project_root.join("pv.yml"), "php: [\n")?;

    reconcile_gateway_runtimes(&paths).await?;
    let database = Database::open(&paths)?;
    let observed = database
        .project_env_observed_state(&project.project.id)?
        .ok_or_else(|| anyhow::anyhow!("expected Project env observed failure"))?;
    drop(database);
    let gateway_root_config = fs::read_to_string(&paths.gateway_root_config())?;

    assert_eq!(
        fs::read_to_string(&gateway_fragment_path)?,
        gateway_fragment
    );
    assert_eq!(fs::read_to_string(&worker_fragment_path)?, worker_fragment);
    assert!(matches!(
        observed.status,
        state::ProjectEnvObservedStatus::Failed
    ));
    assert!(gateway_root_config.contains("import "));
    assert!(!gateway_root_config.contains("PV Gateway is running"));

    let edited_gateway_fragment =
        format!("# edited preserved Gateway fragment\n{gateway_fragment}");
    let edited_worker_fragment = format!("# edited preserved worker fragment\n{worker_fragment}");
    fs::write_sensitive_file(&gateway_fragment_path, &edited_gateway_fragment)?;
    fs::write_sensitive_file(&worker_fragment_path, &edited_worker_fragment)?;
    let gateway_load_count = fake_admin_load_bodies(&paths.gateway_root_config())?.len();
    let worker_load_count = fake_admin_load_bodies(&paths.worker_root_config("8.4"))?.len();

    reconcile_gateway_runtimes(&paths).await?;

    assert_eq!(
        fake_admin_load_bodies(&paths.gateway_root_config())?.len(),
        gateway_load_count + 1
    );
    assert_eq!(
        fake_admin_load_bodies(&paths.worker_root_config("8.4"))?.len(),
        worker_load_count + 1
    );
    assert_eq!(
        fs::read_to_string(&gateway_fragment_path)?,
        edited_gateway_fragment
    );
    assert_eq!(
        fs::read_to_string(&worker_fragment_path)?,
        edited_worker_fragment
    );

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    stop_runtime_from_pid_file(&paths.worker_pid("8.4")).await?;

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_skips_invalid_project_without_preserved_fragments() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let acme_root = tempdir.path().join("acme");
    let broken_root = tempdir.path().join("broken");
    let release_path = tempdir.path().join("fake-frankenphp-release");
    let fake_frankenphp = release_path.join("bin/frankenphp");

    write_fake_frankenphp(&fake_frankenphp)?;
    create_project(
        &acme_root,
        r#"php: "8.4"
document_root: public
"#,
    )?;
    create_project(&broken_root, "php: [\n")?;

    let mut database = Database::open(&paths)?;
    let acme = database.link_project(LinkProjectInput {
        path: acme_root.clone(),
        original_path: acme_root.clone(),
        primary_hostname: "acme.test".to_owned(),
        config_path: acme_root.join("pv.yml"),
        desired_php_track: Some("8.4".to_owned()),
        additional_hostnames: Vec::new(),
    })?;
    let broken = database.link_project(LinkProjectInput {
        path: broken_root.clone(),
        original_path: broken_root.clone(),
        primary_hostname: "broken.test".to_owned(),
        config_path: broken_root.join("pv.yml"),
        desired_php_track: Some("8.4".to_owned()),
        additional_hostnames: Vec::new(),
    })?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-frankenphp-pv1",
        &release_path,
    )?;
    let ports = available_loopback_ports(3)?;
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[("8.4", ports[2])],
    )?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;

    let database = Database::open(&paths)?;
    let observed = database
        .project_env_observed_state(&broken.project.id)?
        .ok_or_else(|| anyhow::anyhow!("expected Project env observed failure"))?;
    assert!(matches!(
        observed.status,
        state::ProjectEnvObservedStatus::Failed
    ));
    assert!(
        paths
            .gateway_projects_config_dir()
            .join(format!("{}.Caddyfile", acme.project.id))
            .exists()
    );
    assert!(
        !paths
            .gateway_projects_config_dir()
            .join(format!("{}.Caddyfile", broken.project.id))
            .exists()
    );

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    stop_runtime_from_pid_file(&paths.worker_pid("8.4")).await?;

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_uses_persisted_track_after_config_becomes_invalid() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let project_root = tempdir.path().join("acme");
    let release_84_path = tempdir.path().join("fake-frankenphp-84-release");
    let release_83_path = tempdir.path().join("fake-frankenphp-83-release");
    let fake_frankenphp_84 = release_84_path.join("bin/frankenphp");
    let fake_frankenphp_83 = release_83_path.join("bin/frankenphp");

    write_fake_frankenphp(&fake_frankenphp_84)?;
    write_fake_frankenphp(&fake_frankenphp_83)?;
    create_project(
        &project_root,
        r#"php: "8.4"
document_root: public
"#,
    )?;

    let mut database = Database::open(&paths)?;
    let project = database.link_project(LinkProjectInput {
        path: project_root.clone(),
        original_path: project_root.clone(),
        primary_hostname: "acme.test".to_owned(),
        config_path: project_root.join("pv.yml"),
        desired_php_track: Some("8.4".to_owned()),
        additional_hostnames: Vec::new(),
    })?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-frankenphp-84-pv1",
        &release_84_path,
    )?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.3",
        "fake-frankenphp-83-pv1",
        &release_83_path,
    )?;
    let ports = available_loopback_ports(4)?;
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[("8.4", ports[2]), ("8.3", ports[3])],
    )?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let worker_84_pid = runtime_metadata_pid(&paths.worker_runtime_metadata("8.4"))?
        .ok_or_else(|| anyhow::anyhow!("expected 8.4 worker metadata"))?;

    fs::write_sensitive_file(
        &project_root.join("pv.yml"),
        r#"php: "8.3"
document_root: public
"#,
    )?;
    let mut database = Database::open(&paths)?;
    database.replace_project_desired_php_track(&project.project.id, Some("8.3"))?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    wait_for_process_exit(worker_84_pid).await?;

    fs::write_sensitive_file(&project_root.join("pv.yml"), "php: [\n")?;
    reconcile_gateway_runtimes(&paths).await?;

    let worker_83_pid = runtime_metadata_pid(&paths.worker_runtime_metadata("8.3"))?
        .ok_or_else(|| anyhow::anyhow!("expected 8.3 worker metadata"))?;
    let worker_83_alive = process_is_alive(worker_83_pid)?;
    let worker_84_alive = match runtime_metadata_pid(&paths.worker_runtime_metadata("8.4"))? {
        Some(pid) => process_is_alive(pid)?,
        None => false,
    };

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    if worker_83_alive {
        stop_runtime_from_pid_file(&paths.worker_pid("8.3")).await?;
    }
    if worker_84_alive {
        stop_runtime_from_pid_file(&paths.worker_pid("8.4")).await?;
    }

    assert!(worker_83_alive);
    assert!(!worker_84_alive);

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_runtime_plan_fails_when_persisted_extension_runtime_cannot_be_reconstructed()
-> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let project_root = tempdir.path().join("acme");

    create_project(&project_root, "php: [\n")?;
    let mut database = Database::open(&paths)?;
    let project = database.link_project(LinkProjectInput {
        path: project_root.clone(),
        original_path: project_root,
        primary_hostname: "acme.test".to_owned(),
        config_path: tempdir.path().join("acme/pv.yml"),
        desired_php_track: Some("8.4".to_owned()),
        additional_hostnames: Vec::new(),
    })?;
    database.replace_project_php_runtime(
        &project.project.id,
        Some(&state::ProjectPhpRuntimeInput {
            track: "8.4".to_owned(),
            requested_extensions: vec!["redis".to_owned()],
            loaded_extensions: vec!["redis".to_owned()],
            ignored_extensions: Vec::new(),
        }),
    )?;
    drop(database);
    seed_installed_php_with_extensions(&paths, "8.4", &[])?;

    let result = build_runtime_plan(&paths);
    let database = Database::open(&paths)?;
    let observed = database
        .project_env_observed_state(&project.project.id)?
        .ok_or_else(|| anyhow::anyhow!("expected Project env observed failure"))?;

    assert!(matches!(
        result,
        Err(DaemonError::Resources(
            resources::ResourcesError::InvalidArtifactLayout { resource, .. }
        )) if resource == "php"
    ));
    assert!(matches!(
        observed.status,
        state::ProjectEnvObservedStatus::Failed
    ));
    assert!(
        observed
            .message
            .as_deref()
            .is_some_and(|message| { message.contains("persisted PHP extension `redis`") })
    );

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_runtime_plan_recovers_preserved_worker_tree_without_metadata() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let project_root = tempdir.path().join("acme");

    create_project(&project_root, "php: [\n")?;
    let mut database = Database::open(&paths)?;
    let project = database
        .link_project(LinkProjectInput {
            path: project_root.clone(),
            original_path: project_root,
            primary_hostname: "acme.test".to_owned(),
            config_path: tempdir.path().join("acme/pv.yml"),
            desired_php_track: Some("8.4".to_owned()),
            additional_hostnames: Vec::new(),
        })?
        .project;
    let ports = available_loopback_ports(3)?;
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[("8.4", ports[2])],
    )?;
    drop(database);
    fs::write_sensitive_file(
        &paths
            .gateway_projects_config_dir()
            .join(format!("{}.Caddyfile", project.id)),
        &format!("    reverse_proxy 127.0.0.1:{} {{\n", ports[2]),
    )?;
    fs::write_sensitive_file(
        &paths
            .worker_projects_config_dir("8.4")
            .join(format!("{}.Caddyfile", project.id)),
        "# preserved worker fragment\n",
    )?;

    let plan = build_runtime_plan(&paths)?;
    let worker = plan
        .workers
        .first()
        .ok_or_else(|| anyhow::anyhow!("expected preserved PHP worker"))?;
    let planned_project = worker
        .projects
        .first()
        .ok_or_else(|| anyhow::anyhow!("expected preserved Project"))?;

    assert_eq!(worker.runtime_key, "8.4");
    assert_eq!(planned_project.id, project.id);
    assert!(!planned_project.render_config);

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_preserves_fragments_for_parseable_invalid_project_config()
-> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let acme_root = tempdir.path().join("acme");
    let other_root = tempdir.path().join("other");
    let release_84_path = tempdir.path().join("fake-frankenphp-84-release");
    let release_83_path = tempdir.path().join("fake-frankenphp-83-release");
    let fake_frankenphp_84 = release_84_path.join("bin/frankenphp");
    let fake_frankenphp_83 = release_83_path.join("bin/frankenphp");

    write_fake_frankenphp(&fake_frankenphp_84)?;
    write_fake_frankenphp(&fake_frankenphp_83)?;
    create_project(
        &acme_root,
        r#"php: "8.4"
document_root: public
hostnames:
  - api.acme.test
"#,
    )?;
    create_project(
        &other_root,
        r#"php: "8.3"
document_root: public
"#,
    )?;

    let mut database = Database::open(&paths)?;
    let acme = database.link_project(LinkProjectInput {
        path: acme_root.clone(),
        original_path: acme_root.clone(),
        primary_hostname: "acme.test".to_owned(),
        config_path: acme_root.join("pv.yml"),
        desired_php_track: Some("8.4".to_owned()),
        additional_hostnames: vec!["api.acme.test".to_owned()],
    })?;
    database.link_project(LinkProjectInput {
        path: other_root.clone(),
        original_path: other_root.clone(),
        primary_hostname: "other.test".to_owned(),
        config_path: other_root.join("pv.yml"),
        desired_php_track: Some("8.3".to_owned()),
        additional_hostnames: Vec::new(),
    })?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-frankenphp-84-pv1",
        &release_84_path,
    )?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.3",
        "fake-frankenphp-83-pv1",
        &release_83_path,
    )?;
    let ports = available_loopback_ports(4)?;
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[("8.4", ports[2]), ("8.3", ports[3])],
    )?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let acme_gateway_fragment_path = paths
        .gateway_projects_config_dir()
        .join(format!("{}.Caddyfile", acme.project.id));
    let acme_worker_fragment_path = paths
        .worker_projects_config_dir("8.4")
        .join(format!("{}.Caddyfile", acme.project.id));
    let acme_gateway_fragment = fs::read_to_string(&acme_gateway_fragment_path)?;
    let acme_worker_fragment = fs::read_to_string(&acme_worker_fragment_path)?;

    fs::write_sensitive_file(
        &acme_root.join("pv.yml"),
        r#"php: "8.3"
document_root: public
hostnames:
  - other.test
"#,
    )?;

    reconcile_gateway_runtimes(&paths).await?;
    let database = Database::open(&paths)?;
    let observed = database
        .project_env_observed_state(&acme.project.id)?
        .ok_or_else(|| anyhow::anyhow!("expected Project env observed failure"))?;

    assert_eq!(
        fs::read_to_string(&acme_gateway_fragment_path)?,
        acme_gateway_fragment
    );
    assert_eq!(
        fs::read_to_string(&acme_worker_fragment_path)?,
        acme_worker_fragment
    );
    assert!(
        !paths
            .worker_projects_config_dir("8.3")
            .join(format!("{}.Caddyfile", acme.project.id))
            .exists()
    );
    assert!(matches!(
        observed.status,
        state::ProjectEnvObservedStatus::Failed
    ));

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    stop_runtime_from_pid_file(&paths.worker_pid("8.4")).await?;
    stop_runtime_from_pid_file(&paths.worker_pid("8.3")).await?;

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_preserves_active_fragments_when_validation_fails() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let project_root = tempdir.path().join("acme");
    let release_path = tempdir.path().join("fake-frankenphp-release");
    let fake_frankenphp = release_path.join("bin/frankenphp");

    write_fake_frankenphp(&fake_frankenphp)?;
    create_project(
        &project_root,
        r#"php: "8.4"
document_root: public
hostnames:
  - api.acme.test
"#,
    )?;

    let mut database = Database::open(&paths)?;
    let project = database.link_project(LinkProjectInput {
        path: project_root.clone(),
        original_path: project_root.clone(),
        primary_hostname: "acme.test".to_owned(),
        config_path: project_root.join("pv.yml"),
        desired_php_track: Some("8.4".to_owned()),
        additional_hostnames: vec!["api.acme.test".to_owned()],
    })?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-frankenphp-pv1",
        &release_path,
    )?;
    let ports = available_loopback_ports(3)?;
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[("8.4", ports[2])],
    )?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let gateway_fragment_path = paths
        .gateway_projects_config_dir()
        .join(format!("{}.Caddyfile", project.project.id));
    let worker_fragment_path = paths
        .worker_projects_config_dir("8.4")
        .join(format!("{}.Caddyfile", project.project.id));
    let gateway_fragment = fs::read_to_string(&gateway_fragment_path)?;
    let worker_fragment = fs::read_to_string(&worker_fragment_path)?;

    write_failing_config_validator(&fake_frankenphp)?;
    fs::write_sensitive_file(
        &project_root.join("pv.yml"),
        r#"php: "8.4"
document_root: public
hostnames:
  - changed.acme.test
"#,
    )?;

    let result = reconcile_gateway_runtimes(&paths).await;

    assert!(matches!(
        result,
        Err(DaemonError::UnexpectedProtocolResponse { reason })
            if reason.contains("FrankenPHP config validation failed")
    ));
    assert_eq!(
        fs::read_to_string(&gateway_fragment_path)?,
        gateway_fragment
    );
    assert_eq!(fs::read_to_string(&worker_fragment_path)?, worker_fragment);

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    stop_runtime_from_pid_file(&paths.worker_pid("8.4")).await?;

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn retained_hostname_uses_previous_document_root() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let project_root = tempdir.path().join("acme");
    create_project(
        &project_root,
        r#"php: "8.4"
document_root: public
hostnames:
  - old.acme.test
"#,
    )?;

    let mut database = Database::open(&paths)?;
    let project = database
        .link_project(LinkProjectInput {
            path: project_root.clone(),
            original_path: project_root.clone(),
            primary_hostname: "acme.test".to_owned(),
            config_path: project_root.join("pv.yml"),
            desired_php_track: Some("8.4".to_owned()),
            additional_hostnames: Vec::new(),
        })?
        .project;
    let release = paths.home().join("8.4-php-release");
    seed_installed_php_with_extensions(&paths, "8.4", &["redis"])?;
    seed_installed_frankenphp_with_extensions(&paths, "8.4", &release, &["redis"])?;
    write_fake_frankenphp(&release.join("bin/frankenphp"))?;
    let base_runtime_key = state::php_runtime_key("8.4", &[])?;
    let redis_runtime_key = state::php_runtime_key("8.4", &["redis".to_owned()])?;
    let ports = available_loopback_ports(4)?;
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[
            (&base_runtime_key, ports[2]),
            (&redis_runtime_key, ports[3]),
        ],
    )?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;

    let redis_root = create_project_with_config(
        tempdir.path(),
        "redis",
        "php:\n  version: \"8.4\"\n  extensions: [redis]\n",
    )?;
    link_project_record(&paths, &redis_root, "api.acme.test", Some("8.4"))?;
    fs::write_sensitive_file(&project_root.join("web/index.php"), "<?php\n")?;
    fs::write_sensitive_file(
        &project_root.join("pv.yml"),
        r#"php: "8.4"
document_root: web
hostnames:
  - new.acme.test
"#,
    )?;
    let redis_failure_marker = Utf8PathBuf::from(format!(
        "{}.readiness-fail",
        paths.worker_root_config(&redis_runtime_key)
    ));
    fs::write_sensitive_file(&redis_failure_marker, "fail\n")?;

    let result = reconcile_gateway_runtimes(&paths).await;
    assert!(
        matches!(
            &result,
            Err(DaemonError::UnexpectedProtocolResponse { reason })
                if reason.contains(&format!("php-worker-{redis_runtime_key}"))
        ),
        "the sibling failure must surface after worker validation succeeds, got {result:?}"
    );
    let fragment_path = paths
        .worker_projects_config_dir(&base_runtime_key)
        .join(format!("{}.Caddyfile", project.id));
    let merged = fs::read_to_string(&fragment_path)?;
    let blocks = fragment_site_blocks(&merged);
    assert_eq!(
        blocks.len(),
        2,
        "expected retained and desired site blocks: {merged:?}"
    );
    assert_eq!(merged.matches("old.acme.test").count(), 1);
    let (old_labels, old_body) = blocks
        .iter()
        .find(|(labels, _)| labels.contains("old.acme.test"))
        .ok_or_else(|| anyhow::anyhow!("retained block is missing: {merged:?}"))?;
    assert!(!old_labels.contains("new.acme.test"));
    assert!(old_body.contains("public"));
    assert!(!old_body.contains("web"));
    let (new_labels, new_body) = blocks
        .iter()
        .find(|(labels, _)| labels.contains("new.acme.test"))
        .ok_or_else(|| anyhow::anyhow!("desired block is missing: {merged:?}"))?;
    assert!(new_labels.contains("acme.test"));
    assert!(new_body.contains("web"));
    assert!(!new_body.contains("public"));

    fs::remove_file(&redis_failure_marker)?;
    reconcile_gateway_runtimes(&paths).await?;
    let committed = fs::read_to_string(&fragment_path)?;
    assert!(!committed.contains("old.acme.test"));
    assert!(committed.contains("new.acme.test"));

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    stop_runtime_from_pid_file(&paths.worker_pid(&base_runtime_key)).await?;
    stop_runtime_from_pid_file(&paths.worker_pid(&redis_runtime_key)).await?;

    runtime_guard.cleanup().await?;

    Ok(())
}

fn fragment_site_blocks(fragment: &str) -> Vec<(&str, &str)> {
    fragment
        .split_terminator("}\n")
        .filter_map(|section| {
            let (labels, body) = section.split_once(" {\n")?;
            Some((labels.trim(), body))
        })
        .collect()
}

#[tokio::test]
async fn post_commit_cleanup_continues_after_worker_failure() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let project_a_root = tempdir.path().join("a");
    let project_b_root = tempdir.path().join("b");
    let project_c_root = tempdir.path().join("c");
    create_project(
        &project_a_root,
        r#"php: "8.4"
document_root: public
hostnames:
  - old.acme.test
"#,
    )?;
    create_project(
        &project_b_root,
        r#"php: "8.5"
document_root: public
hostnames:
  - old.api.acme.test
"#,
    )?;
    create_project(&project_c_root, "php: \"8.3\"\n")?;
    link_project_record(&paths, &project_a_root, "acme.test", Some("8.4"))?;
    link_project_record(&paths, &project_b_root, "api.acme.test", Some("8.5"))?;
    link_project_record(&paths, &project_c_root, "other.test", Some("8.3"))?;
    let caddy_release = tempdir.path().join("caddy");
    let release_83 = tempdir.path().join("frankenphp-83");
    let release_84 = tempdir.path().join("frankenphp-84");
    let release_85 = tempdir.path().join("frankenphp-85");
    write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;
    write_stateful_fake_frankenphp(&release_83.join("bin/frankenphp"))?;
    write_stateful_fake_frankenphp(&release_84.join("bin/frankenphp"))?;
    write_stateful_fake_frankenphp(&release_85.join("bin/frankenphp"))?;
    let ports = available_loopback_ports(5)?;
    let mut database = Database::open(&paths)?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    for (track, release) in [
        ("8.3", &release_83),
        ("8.4", &release_84),
        ("8.5", &release_85),
    ] {
        database.record_managed_resource_track_installed(
            "frankenphp",
            track,
            &format!("fake-{track}-pv1"),
            release,
        )?;
    }
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[("8.3", ports[2]), ("8.4", ports[3]), ("8.5", ports[4])],
    )?;
    let projects = database.projects()?;
    let project_a_id = projects
        .iter()
        .find(|project| project.path == project_a_root)
        .ok_or_else(|| anyhow::anyhow!("missing project A"))?
        .id
        .clone();
    let project_b_id = projects
        .iter()
        .find(|project| project.path == project_b_root)
        .ok_or_else(|| anyhow::anyhow!("missing project B"))?
        .id
        .clone();
    drop(database);
    reconcile_gateway_runtimes(&paths).await?;
    let stale_worker_pid = required_runtime_metadata_pid(&paths.worker_runtime_metadata("8.3"))?;
    assert!(process_is_alive(stale_worker_pid)?);

    fs::write_sensitive_file(&project_a_root.join("web/index.php"), "<?php\n")?;
    fs::write_sensitive_file(&project_b_root.join("web/index.php"), "<?php\n")?;
    fs::write_sensitive_file(
        &project_a_root.join("pv.yml"),
        r#"php: "8.4"
document_root: web
hostnames:
  - new.acme.test
"#,
    )?;
    fs::write_sensitive_file(
        &project_b_root.join("pv.yml"),
        r#"php: "8.5"
document_root: web
hostnames:
  - new.api.acme.test
"#,
    )?;
    fs::write_sensitive_file(&project_c_root.join("pv.yml"), "php: \"8.5\"\n")?;
    write_fake_admin_control(
        &paths.worker_root_config("8.4"),
        json!({"load_statuses": [200, 422]}),
    )?;

    let result = reconcile_gateway_runtimes(&paths).await;
    assert!(
        matches!(
            &result,
            Err(DaemonError::CaddyAdmin(CaddyAdminError::LoadRejected { status, .. }))
                if *status == 422
        ),
        "expected the injected second-pass load failure, got {result:?}"
    );
    assert_eq!(
        fake_admin_load_bodies(&paths.worker_root_config("8.4"))?.len(),
        2,
        "expected one wave load and one failing cleanup load on 8.4",
    );

    let worker_b_fragment = paths
        .worker_projects_config_dir("8.5")
        .join(format!("{project_b_id}.Caddyfile"));
    let worker_b_content = fs::read_to_string(&worker_b_fragment)?;
    assert!(
        !worker_b_content.contains("old.api.acme.test"),
        "later retained worker was not cleaned: {worker_b_content:?}"
    );
    assert!(worker_b_content.contains("new.api.acme.test"));

    let gateway_a_fragment = paths
        .gateway_projects_config_dir()
        .join(format!("{project_a_id}.Caddyfile"));
    assert!(
        fs::read_to_string(&gateway_a_fragment)?.contains("new.acme.test"),
        "committed Gateway must not be rolled back by cleanup failure",
    );

    assert!(!paths.worker_pid("8.3").exists());
    assert!(!paths.worker_runtime_metadata("8.3").exists());
    wait_for_process_exit(stale_worker_pid).await?;
    let stale_status = Database::open(&paths)?
        .runtime_observed_states()?
        .into_iter()
        .find(|state| {
            state.subject
                == RuntimeSubject::PhpWorker {
                    php_track: "8.3".to_owned(),
                }
        })
        .map(|state| state.status);
    assert_eq!(stale_status, Some(RuntimeObservedStatus::Stopped));
    let stale_ports = Database::open(&paths)?
        .assigned_ports()?
        .into_iter()
        .filter(|port| {
            matches!(
                &port.owner,
                PortOwner::PhpWorker { php_runtime_key } if php_runtime_key == "8.3"
            )
        })
        .count();
    assert_eq!(stale_ports, 0);

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    for track in ["8.4", "8.5"] {
        let pid_path = paths.worker_pid(track);
        if pid_path.exists() {
            stop_runtime_from_pid_file(&pid_path).await?;
        }
    }

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_loads_exact_gateway_and_worker_roots_without_restarting()
-> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let project_root = tempdir.path().join("acme");
    let other_project_root = tempdir.path().join("other");
    let caddy_release = tempdir.path().join("fake-caddy-release");
    let frankenphp_84_release = tempdir.path().join("fake-frankenphp-84-release");
    let frankenphp_83_release = tempdir.path().join("fake-frankenphp-83-release");

    write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;
    write_stateful_fake_frankenphp(&frankenphp_84_release.join("bin/frankenphp"))?;
    write_stateful_fake_frankenphp(&frankenphp_83_release.join("bin/frankenphp"))?;
    create_project(
        &project_root,
        r#"php: "8.4"
document_root: public
hostnames:
  - api.acme.test
"#,
    )?;
    create_project(
        &other_project_root,
        r#"php: "8.3"
document_root: public
"#,
    )?;

    let mut database = Database::open(&paths)?;
    let project = database
        .link_project(LinkProjectInput {
            path: project_root.clone(),
            original_path: project_root.clone(),
            primary_hostname: "acme.test".to_owned(),
            config_path: project_root.join("pv.yml"),
            desired_php_track: Some("8.4".to_owned()),
            additional_hostnames: Vec::new(),
        })?
        .project;
    database.link_project(LinkProjectInput {
        path: other_project_root.clone(),
        original_path: other_project_root.clone(),
        primary_hostname: "other.test".to_owned(),
        config_path: other_project_root.join("pv.yml"),
        desired_php_track: Some("8.3".to_owned()),
        additional_hostnames: Vec::new(),
    })?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-frankenphp-84-pv1",
        &frankenphp_84_release,
    )?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.3",
        "fake-frankenphp-83-pv1",
        &frankenphp_83_release,
    )?;
    let ports = available_loopback_ports(4)?;
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[("8.4", ports[2]), ("8.3", ports[3])],
    )?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let first_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected gateway runtime metadata"))?;
    let first_worker_pid = runtime_metadata_pid(&paths.worker_runtime_metadata("8.4"))?
        .ok_or_else(|| anyhow::anyhow!("expected worker runtime metadata"))?;
    let first_unaffected_worker_pid = runtime_metadata_pid(&paths.worker_runtime_metadata("8.3"))?
        .ok_or_else(|| anyhow::anyhow!("expected unaffected worker runtime metadata"))?;
    let gateway_fragment_path = paths
        .gateway_projects_config_dir()
        .join(format!("{}.Caddyfile", project.id));
    let worker_fragment_path = paths
        .worker_projects_config_dir("8.4")
        .join(format!("{}.Caddyfile", project.id));
    let previous_gateway_fragment = fs::read_to_string(&gateway_fragment_path)?;
    let previous_worker_fragment = fs::read_to_string(&worker_fragment_path)?;

    fs::write_sensitive_file(
        &project_root.join("pv.yml"),
        r#"php: "8.4"
document_root: public
hostnames:
  - api.acme.test
  - changed.acme.test
"#,
    )?;

    reconcile_gateway_runtimes(&paths).await?;
    let second_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected gateway runtime metadata"))?;
    let second_worker_pid = runtime_metadata_pid(&paths.worker_runtime_metadata("8.4"))?
        .ok_or_else(|| anyhow::anyhow!("expected worker runtime metadata"))?;
    let second_unaffected_worker_pid = runtime_metadata_pid(&paths.worker_runtime_metadata("8.3"))?
        .ok_or_else(|| anyhow::anyhow!("expected unaffected worker runtime metadata"))?;
    let gateway_load_bodies = fake_admin_load_bodies(&paths.gateway_root_config())?;
    let worker_load_bodies = fake_admin_load_bodies(&paths.worker_root_config("8.4"))?;
    let gateway_requests = fake_admin_requests(&paths.gateway_root_config())?;
    let worker_requests = fake_admin_requests(&paths.worker_root_config("8.4"))?;
    let gateway_root = read_test_bytes(paths.gateway_root_config())?;
    let worker_root = read_test_bytes(paths.worker_root_config("8.4"))?;

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    stop_runtime_from_pid_file(&paths.worker_pid("8.4")).await?;
    stop_runtime_from_pid_file(&paths.worker_pid("8.3")).await?;

    assert_eq!(first_gateway_pid, second_gateway_pid);
    assert_eq!(first_worker_pid, second_worker_pid);
    assert_eq!(first_unaffected_worker_pid, second_unaffected_worker_pid);
    assert_ne!(
        previous_gateway_fragment,
        fs::read_to_string(&gateway_fragment_path)?
    );
    assert_ne!(
        previous_worker_fragment,
        fs::read_to_string(&worker_fragment_path)?
    );
    assert_eq!(gateway_load_bodies, vec![gateway_root.clone()]);
    assert_eq!(worker_load_bodies, vec![worker_root.clone()]);
    assert!(fake_admin_load_bodies(&paths.worker_root_config("8.3"))?.is_empty());
    assert!(gateway_load_bodies[0].ends_with(b"\n"));
    assert!(worker_load_bodies[0].ends_with(b"\n"));
    assert_eq!(
        gateway_requests
            .iter()
            .filter(|request| request["method"] == "POST" && request["path"] == "/load")
            .count(),
        1
    );
    assert!(
        gateway_requests
            .iter()
            .any(|request| { request["method"] == "GET" && request["path"] == "/config/" })
    );
    assert_eq!(
        worker_requests
            .iter()
            .filter(|request| request["method"] == "POST" && request["path"] == "/load")
            .count(),
        1
    );
    assert!(
        worker_requests
            .iter()
            .any(|request| { request["method"] == "GET" && request["path"] == "/config/" })
    );

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn worker_reconciliation_restores_previous_service_port_after_readiness_failure() -> Result<()>
{
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let project_root = tempdir.path().join("acme");
    let frankenphp_release = tempdir.path().join("fake-frankenphp-release");
    write_stateful_fake_frankenphp(&frankenphp_release.join("bin/frankenphp"))?;
    create_project(
        &project_root,
        r#"php: "8.4"
document_root: public
"#,
    )?;

    let ports = available_loopback_ports(4)?;
    let old_worker_port = ports[2];
    let new_worker_port = ports[3];
    let mut database = Database::open(&paths)?;
    let project = database
        .link_project(LinkProjectInput {
            path: project_root.clone(),
            original_path: project_root.clone(),
            primary_hostname: "acme.test".to_owned(),
            config_path: project_root.join("pv.yml"),
            desired_php_track: Some("8.4".to_owned()),
            additional_hostnames: Vec::new(),
        })?
        .project;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-frankenphp-pv1",
        &frankenphp_release,
    )?;
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[("8.4", old_worker_port)],
    )?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let first_worker_pid = runtime_metadata_pid(&paths.worker_runtime_metadata("8.4"))?
        .ok_or_else(|| anyhow::anyhow!("expected worker runtime metadata"))?;
    let previous_root = read_test_bytes(paths.worker_root_config("8.4"))?;
    let previous_fragment_path = paths
        .worker_projects_config_dir("8.4")
        .join(format!("{}.Caddyfile", project.id));
    let previous_fragment = fs::read_to_string(&previous_fragment_path)?;
    let previous_metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.worker_runtime_metadata("8.4"))?)?;
    fs::write_sensitive_file(
        &paths.worker_projects_config_dir("8.4").join("notes.txt"),
        "This file is not imported by the worker config.\n",
    )?;

    let mut database = Database::open(&paths)?;
    database.release_port(PortOwner::PhpWorker {
        php_runtime_key: "8.4".to_owned(),
    })?;
    database.assign_port(
        PortRequest::php_worker("8.4", new_worker_port, new_worker_port, new_worker_port),
        |_port| true,
    )?;
    drop(database);

    let result = reconcile_gateway_runtimes_with_pf_state_for_test(
        &paths,
        Duration::from_millis(150),
        GatewayPfRoutingState::Unknown,
    )
    .await;
    let second_worker_pid = runtime_metadata_pid(&paths.worker_runtime_metadata("8.4"))?
        .ok_or_else(|| anyhow::anyhow!("expected worker runtime metadata"))?;
    let root_after = read_test_bytes(paths.worker_root_config("8.4"))?;
    let load_bodies = fake_admin_load_bodies(&paths.worker_root_config("8.4"))?;
    let requests = fake_admin_requests(&paths.worker_root_config("8.4"))?;
    let restored_metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.worker_runtime_metadata("8.4"))?)?;

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    stop_runtime_from_pid_file(&paths.worker_pid("8.4")).await?;

    assert!(
        matches!(&result, Err(DaemonError::ReadinessTimedOut { .. })),
        "{result:?}"
    );
    assert!(previous_metadata["applied_config_fingerprint"].is_string());
    assert_eq!(
        restored_metadata["applied_config_fingerprint"],
        previous_metadata["applied_config_fingerprint"]
    );
    assert_ne!(
        restored_metadata["desired_config_fingerprint"],
        restored_metadata["applied_config_fingerprint"]
    );
    assert_ne!(restored_metadata["replacement_required"], true);
    assert_eq!(first_worker_pid, second_worker_pid);
    assert_eq!(root_after, previous_root);
    assert_eq!(
        fs::read_to_string(&previous_fragment_path)?,
        previous_fragment
    );
    assert_eq!(load_bodies.len(), 2);
    assert_eq!(load_bodies[0], previous_root);
    assert_eq!(load_bodies[1], previous_root);
    assert_eq!(
        requests
            .iter()
            .filter(|request| request["method"] == "POST" && request["path"] == "/load")
            .count(),
        2
    );
    assert!(
        requests
            .iter()
            .any(|request| { request["method"] == "GET" && request["path"] == "/config/" })
    );

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_rejection_keeps_old_runtime_and_disk_state() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let caddy_release = tempdir.path().join("fake-caddy-release");
    write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;

    let old_ports = available_loopback_ports(2)?;
    let new_ports = available_loopback_ports(2)?;
    let mut database = Database::open(&paths)?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    seed_runtime_ports(&paths, &mut database, old_ports[0], old_ports[1], &[])?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let first_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected gateway runtime metadata"))?;
    let previous_root = read_test_bytes(paths.gateway_root_config())?;
    let previous_metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.gateway_runtime_metadata())?)?;
    write_fake_admin_control(
        &paths.gateway_root_config(),
        json!({"load_statuses": [422]}),
    )?;

    let mut database = Database::open(&paths)?;
    database.release_port(PortOwner::Gateway(GatewayPort::Http))?;
    database.release_port(PortOwner::Gateway(GatewayPort::Https))?;
    seed_runtime_ports(&paths, &mut database, new_ports[0], new_ports[1], &[])?;
    drop(database);

    let result =
        reconcile_gateway_runtimes_with_readiness_timeout(&paths, Duration::from_millis(250)).await;
    let second_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected gateway runtime metadata"))?;
    let root_after = read_test_bytes(paths.gateway_root_config())?;
    let load_bodies = fake_admin_load_bodies(&paths.gateway_root_config())?;
    let requests = fake_admin_requests(&paths.gateway_root_config())?;
    let metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.gateway_runtime_metadata())?)?;

    let Err(error) = result else {
        bail!("expected rejection, got success");
    };
    assert!(
        matches!(
            error,
            DaemonError::CaddyAdmin(CaddyAdminError::LoadRejected { .. })
        ),
        "unexpected rejection error: {error:?}"
    );
    assert_eq!(first_gateway_pid, second_gateway_pid);
    assert_eq!(root_after, previous_root);
    assert_ne!(metadata["replacement_required"], true);
    assert_eq!(
        metadata["applied_config_fingerprint"],
        previous_metadata["applied_config_fingerprint"]
    );
    assert_ne!(
        metadata["desired_config_fingerprint"],
        metadata["applied_config_fingerprint"]
    );
    assert_eq!(load_bodies.len(), 1);
    assert_eq!(
        requests
            .iter()
            .filter(|request| request["method"] == "POST" && request["path"] == "/load")
            .map(|request| request["status"].as_u64())
            .collect::<Vec<_>>(),
        vec![Some(422)]
    );

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_confirms_reverted_desired_config_without_reload() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let caddy_release = tempdir.path().join("fake-caddy-release");
    write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;

    let ports = available_loopback_ports(2)?;
    let mut database = Database::open(&paths)?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    seed_runtime_ports(&paths, &mut database, ports[0], ports[1], &[])?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let initial_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected gateway runtime metadata"))?;
    let mut metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.gateway_runtime_metadata())?)?;
    metadata["desired_config_fingerprint"] = json!("sha256:v1:unapplied");
    fs::write_sensitive_file(
        &paths.gateway_runtime_metadata(),
        &serde_json::to_string(&metadata)?,
    )?;
    let initial_loads = fake_admin_load_bodies(&paths.gateway_root_config())?;

    let result = reconcile_gateway_runtimes(&paths).await;
    let final_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected gateway runtime metadata"))?;
    let final_metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.gateway_runtime_metadata())?)?;
    let final_loads = fake_admin_load_bodies(&paths.gateway_root_config())?;

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    result?;

    assert_eq!(final_gateway_pid, initial_gateway_pid);
    assert_eq!(final_loads, initial_loads);
    assert_eq!(
        final_metadata["desired_config_fingerprint"],
        final_metadata["applied_config_fingerprint"]
    );

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_rejects_load_error_reported_with_success_status() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let caddy_release = tempdir.path().join("fake-caddy-release");
    write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;

    let old_ports = available_loopback_ports(2)?;
    let new_ports = available_loopback_ports(2)?;
    let mut database = Database::open(&paths)?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    seed_runtime_ports(&paths, &mut database, old_ports[0], old_ports[1], &[])?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let first_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected gateway runtime metadata"))?;
    let previous_root = read_test_bytes(paths.gateway_root_config())?;
    write_fake_admin_control(
        &paths.gateway_root_config(),
        json!({
            "load_statuses": [200],
            "apply_load": [false],
            "load_response_body": [
                "[{\"file\":\"Caddyfile\",\"line\":2,\"message\":\"Caddyfile input is not formatted\"}]{\"error\":\"loading config: listener unavailable\"}\n"
            ],
        }),
    )?;

    let mut database = Database::open(&paths)?;
    database.release_port(PortOwner::Gateway(GatewayPort::Http))?;
    database.release_port(PortOwner::Gateway(GatewayPort::Https))?;
    seed_runtime_ports(&paths, &mut database, new_ports[0], new_ports[1], &[])?;
    drop(database);

    let result =
        reconcile_gateway_runtimes_with_readiness_timeout(&paths, Duration::from_millis(250)).await;
    let second_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected gateway runtime metadata"))?;
    let root_after = read_test_bytes(paths.gateway_root_config())?;
    let current_config = fake_admin_current_bytes(&paths.gateway_root_config())?;

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;

    assert!(matches!(
        result,
        Err(DaemonError::CaddyAdmin(
            CaddyAdminError::LoadReportedFailure {
                status: 200,
                detail,
                ..
            }
        )) if detail == "loading config: listener unavailable"
    ));
    assert_eq!(first_gateway_pid, second_gateway_pid);
    assert_eq!(root_after, previous_root);
    assert_eq!(current_config, previous_root);

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_replaces_runtime_before_newer_desired_state() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let caddy_release = tempdir.path().join("fake-caddy-release");
    write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;

    let ports = available_loopback_ports(6)?;
    let mut database = Database::open(&paths)?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    seed_runtime_ports(&paths, &mut database, ports[0], ports[1], &[])?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let first_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected gateway runtime metadata"))?;
    let previous_root = read_test_bytes(paths.gateway_root_config())?;
    write_fake_admin_control(
        &paths.gateway_root_config(),
        json!({
            "late_accept": [true],
            "late_apply_delay_ms": [2000],
            "load_delay_ms": [2000],
            "load_statuses": [200]
        }),
    )?;

    let mut database = Database::open(&paths)?;
    database.release_port(PortOwner::Gateway(GatewayPort::Http))?;
    database.release_port(PortOwner::Gateway(GatewayPort::Https))?;
    seed_runtime_ports(&paths, &mut database, ports[2], ports[3], &[])?;
    drop(database);

    let result =
        reconcile_gateway_runtimes_with_readiness_timeout(&paths, Duration::from_millis(150)).await;
    let initial_load_bodies = fake_admin_load_bodies(&paths.gateway_root_config())?;
    let first_candidate_root = initial_load_bodies
        .first()
        .ok_or_else(|| anyhow::anyhow!("fake admin did not record the candidate load"))?
        .clone();
    let unknown_runtime_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected gateway runtime metadata"))?;
    let unknown_runtime_metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.gateway_runtime_metadata())?)?;

    assert!(matches!(
        result,
        Err(DaemonError::CaddyAdmin(
            CaddyAdminError::RequestOutcomeUnknown {
                operation: daemon::CaddyAdminOperation::Load,
                ..
            }
        ))
    ));
    assert_eq!(first_gateway_pid, unknown_runtime_pid);
    assert_eq!(
        read_test_bytes(paths.gateway_root_config())?,
        first_candidate_root
    );
    assert_eq!(unknown_runtime_metadata["replacement_required"], true);
    assert!(unknown_runtime_metadata["applied_config_fingerprint"].is_null());
    assert_eq!(initial_load_bodies.len(), 1);
    assert_ne!(initial_load_bodies[0], previous_root);

    let mut database = Database::open(&paths)?;
    database.release_port(PortOwner::Gateway(GatewayPort::Http))?;
    database.release_port(PortOwner::Gateway(GatewayPort::Https))?;
    seed_runtime_ports(&paths, &mut database, ports[4], ports[5], &[])?;
    drop(database);

    let recovery_summary = reconcile_gateway_runtimes_with_pf_state_for_test(
        &paths,
        Duration::from_secs(5),
        GatewayPfRoutingState::Unknown,
    )
    .await?;
    let second_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected gateway runtime metadata"))?;
    let root_after = read_test_bytes(paths.gateway_root_config())?;
    let current_config = fake_admin_current_bytes(&paths.gateway_root_config())?;
    let load_bodies = fake_admin_load_bodies(&paths.gateway_root_config())?;
    let requests = fake_admin_requests(&paths.gateway_root_config())?;
    let replacement_metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.gateway_runtime_metadata())?)?;

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;

    assert_eq!(recovery_summary, "Gateway runtime reconciled");
    assert_ne!(first_gateway_pid, second_gateway_pid);
    wait_for_process_exit(first_gateway_pid).await?;
    assert_ne!(root_after, first_candidate_root);
    assert_eq!(current_config, root_after);
    assert_ne!(replacement_metadata["replacement_required"], true);
    assert!(replacement_metadata["applied_config_fingerprint"].is_string());
    assert_eq!(load_bodies, initial_load_bodies);
    assert_eq!(
        requests
            .iter()
            .filter(|request| request["method"] == "POST" && request["path"] == "/load")
            .map(|request| request["status"].as_u64())
            .collect::<Vec<_>>(),
        vec![Some(200)]
    );

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_preserves_accepted_config_for_unknown_pf_state() -> Result<()> {
    gateway_reconciliation_preserves_accepted_config_for_pf_state(GatewayPfRoutingState::Unknown)
        .await
}

#[tokio::test]
async fn gateway_reconciliation_preserves_accepted_config_for_drifted_pf_state() -> Result<()> {
    gateway_reconciliation_preserves_accepted_config_for_pf_state(GatewayPfRoutingState::Drifted)
        .await
}

async fn gateway_reconciliation_preserves_accepted_config_for_pf_state(
    pf_routing_state: GatewayPfRoutingState,
) -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let caddy_release = tempdir.path().join("fake-caddy-release");
    write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;

    let old_ports = available_loopback_ports(2)?;
    let new_ports = available_loopback_ports(2)?;
    let mut database = Database::open(&paths)?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    seed_runtime_ports(&paths, &mut database, old_ports[0], old_ports[1], &[])?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let first_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected gateway runtime metadata"))?;
    let previous_root = read_test_bytes(paths.gateway_root_config())?;

    let mut database = Database::open(&paths)?;
    database.release_port(PortOwner::Gateway(GatewayPort::Http))?;
    database.release_port(PortOwner::Gateway(GatewayPort::Https))?;
    seed_runtime_ports(&paths, &mut database, new_ports[0], new_ports[1], &[])?;
    drop(database);

    reconcile_gateway_runtimes_with_pf_state_for_test(
        &paths,
        Duration::from_millis(150),
        pf_routing_state,
    )
    .await?;
    let second_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected gateway runtime metadata"))?;
    let root_after = read_test_bytes(paths.gateway_root_config())?;
    let load_count = fake_admin_load_bodies(&paths.gateway_root_config())?.len();
    let validation_count = fake_validator_spawns(&paths.gateway_root_config())?;

    reconcile_gateway_runtimes_with_pf_state_for_test(
        &paths,
        Duration::from_millis(150),
        pf_routing_state,
    )
    .await?;
    let third_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected gateway runtime metadata"))?;
    let metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.gateway_runtime_metadata())?)?;

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;

    assert_eq!(first_gateway_pid, second_gateway_pid);
    assert_eq!(first_gateway_pid, third_gateway_pid);
    assert_ne!(root_after, previous_root);
    assert_eq!(
        fake_admin_load_bodies(&paths.gateway_root_config())?.len(),
        load_count + 1
    );
    assert_eq!(
        fake_validator_spawns(&paths.gateway_root_config())?,
        validation_count + 1
    );
    assert!(metadata["applied_config_fingerprint"].is_string());
    assert_ne!(metadata["replacement_required"], true);
    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_does_not_reload_after_runtime_exits_after_load() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let caddy_release = tempdir.path().join("fake-caddy-release");
    write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;
    let old_ports = available_loopback_ports(2)?;
    let new_ports = available_loopback_ports(2)?;

    let mut database = Database::open(&paths)?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    seed_runtime_ports(&paths, &mut database, old_ports[0], old_ports[1], &[])?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let supervisor = ProcessSupervisor::new(paths.clone());
    let captured_gateway = supervisor
        .adopt_recorded(&paths.gateway_pid(), &paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("Gateway was not adoptable before its forced exit"))?;
    let gateway_pid = captured_gateway.pid();
    let runtime_directory = paths
        .gateway_root_config()
        .parent()
        .unwrap_or_else(|| Utf8Path::new("."))
        .to_path_buf();
    let runtime_reaped_marker =
        runtime_directory.join(format!("fake-runtime-reaped-{gateway_pid}"));
    let watcher_stop_path =
        runtime_directory.join(format!("fake-runtime-watcher-stop-{gateway_pid}"));
    let previous_root = read_test_bytes(paths.gateway_root_config())?;
    write_fake_admin_control(
        &paths.gateway_root_config(),
        json!({"exit_after_load": true}),
    )?;

    let mut database = Database::open(&paths)?;
    database.release_port(PortOwner::Gateway(GatewayPort::Http))?;
    database.release_port(PortOwner::Gateway(GatewayPort::Https))?;
    seed_runtime_ports(&paths, &mut database, new_ports[0], new_ports[1], &[])?;
    drop(database);

    let result =
        reconcile_gateway_runtimes_with_readiness_timeout(&paths, Duration::from_millis(250)).await;
    let root_after = read_test_bytes(paths.gateway_root_config())?;
    let load_bodies = fake_admin_load_bodies(&paths.gateway_root_config())?;
    let metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.gateway_runtime_metadata())?)?;

    let cleanup_result: Result<()> = async {
        timeout(Duration::from_secs(5), async {
            loop {
                match fs::read_to_string(&runtime_reaped_marker) {
                    Ok(contents) if contents == "reaped\n" => {
                        return Ok::<_, anyhow::Error>(());
                    }
                    Ok(_) => {}
                    Err(StateError::Filesystem { source, .. })
                        if source.kind() == ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .with_context(|| {
            format!("timed out waiting for Gateway fixture {gateway_pid} to reap its children")
        })??;
        timeout(Duration::from_secs(5), async {
            loop {
                if supervisor
                    .adopt_recorded(&paths.gateway_pid(), &paths.gateway_runtime_metadata())?
                    .is_none()
                {
                    return Ok::<_, anyhow::Error>(());
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .with_context(|| format!("Gateway fixture {gateway_pid} remained adoptable"))??;

        if !pid_path_is_absent_or_matches(&paths.gateway_pid(), gateway_pid)?
            || runtime_metadata_pid(&paths.gateway_runtime_metadata())? != Some(gateway_pid)
        {
            bail!("Gateway runtime records changed after fixture {gateway_pid} exited");
        }
        let _released_ports = old_ports
            .iter()
            .chain(&new_ports)
            .map(|port| TcpListener::bind(("127.0.0.1", *port)))
            .collect::<std::io::Result<Vec<_>>>()?;
        fs::remove_file_if_exists(&paths.gateway_pid())?;
        fs::remove_file_if_exists(&paths.gateway_runtime_metadata())?;
        fs::remove_file_if_exists(&runtime_reaped_marker)?;
        fs::remove_file_if_exists(&watcher_stop_path)?;
        runtime_guard.cleanup().await
    }
    .await;
    if let Err(cleanup_error) = cleanup_result {
        return match &result {
            Err(operation_error) => Err(anyhow::anyhow!(
                "operation failed: {operation_error:#}; fixture cleanup failed: {cleanup_error:#}"
            )),
            Ok(_) => Err(cleanup_error),
        };
    }

    assert!(result.is_err());
    assert_eq!(root_after, previous_root);
    assert_eq!(load_bodies.len(), 1);
    assert_eq!(metadata["replacement_required"], true);
    assert!(metadata["applied_config_fingerprint"].is_null());

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_reports_compound_restore_failure() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let caddy_release = tempdir.path().join("fake-caddy-release");
    write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;
    let old_ports = available_loopback_ports(2)?;
    let new_ports = available_loopback_ports(2)?;

    let mut database = Database::open(&paths)?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    seed_runtime_ports(&paths, &mut database, old_ports[0], old_ports[1], &[])?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let previous_root = read_test_bytes(paths.gateway_root_config())?;
    write_fake_admin_control(
        &paths.gateway_root_config(),
        json!({"load_statuses": [200, 422]}),
    )?;

    let mut database = Database::open(&paths)?;
    database.release_port(PortOwner::Gateway(GatewayPort::Http))?;
    database.release_port(PortOwner::Gateway(GatewayPort::Https))?;
    seed_runtime_ports(&paths, &mut database, new_ports[0], new_ports[1], &[])?;
    drop(database);

    let result =
        reconcile_gateway_runtimes_with_readiness_timeout(&paths, Duration::from_millis(150)).await;
    let root_after = read_test_bytes(paths.gateway_root_config())?;
    let metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.gateway_runtime_metadata())?)?;

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;

    let Err(DaemonError::CaddyAdmin(CaddyAdminError::RestoredConfigReloadFailed {
        original_error,
        restored_error,
    })) = result
    else {
        bail!("expected a compound restore failure, got {result:?}");
    };
    assert!(matches!(
        original_error.as_ref(),
        CaddyAdminError::TaskFailed { .. }
            | CaddyAdminError::AdminReadinessTimedOut { .. }
            | CaddyAdminError::RequestTimedOut { .. }
    ));
    assert!(matches!(
        restored_error.as_ref(),
        CaddyAdminError::LoadRejected { status: 422, .. }
    ));
    assert_eq!(root_after, previous_root);
    assert_eq!(metadata["replacement_required"], true);
    assert!(metadata["applied_config_fingerprint"].is_null());

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_refuses_to_load_tampered_rollback_fragment_backup() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let project_root = tempdir.path().join("acme");
    let caddy_release = tempdir.path().join("fake-caddy-release");
    let frankenphp_release = tempdir.path().join("fake-frankenphp-release");
    write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;
    write_stateful_fake_frankenphp(&frankenphp_release.join("bin/frankenphp"))?;
    create_project(
        &project_root,
        r#"php: "8.4"
document_root: public
"#,
    )?;
    let ports = available_loopback_ports(5)?;

    let mut database = Database::open(&paths)?;
    database.link_project(LinkProjectInput {
        path: project_root.clone(),
        original_path: project_root.clone(),
        primary_hostname: "acme.test".to_owned(),
        config_path: project_root.join("pv.yml"),
        desired_php_track: Some("8.4".to_owned()),
        additional_hostnames: Vec::new(),
    })?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-frankenphp-pv1",
        &frankenphp_release,
    )?;
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[("8.4", ports[2])],
    )?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let mut database = Database::open(&paths)?;
    database.release_port(PortOwner::Gateway(GatewayPort::Http))?;
    database.release_port(PortOwner::Gateway(GatewayPort::Https))?;
    seed_runtime_ports(&paths, &mut database, ports[3], ports[4], &[])?;
    drop(database);

    let projects_directory = paths.gateway_projects_config_dir();
    let config_directory = projects_directory
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Gateway projects path has no parent"))?
        .to_path_buf();
    let config_file_name = projects_directory
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Gateway projects path has no file name"))?;
    let backup_prefix = format!("{config_file_name}.previous.");
    let tamper_backup = async {
        timeout(Duration::from_secs(2), async {
            loop {
                if let Some(backup_path) =
                    fs::read_dir_paths(&config_directory)?
                        .into_iter()
                        .find(|path| {
                            path.file_name()
                                .is_some_and(|file_name| file_name.starts_with(&backup_prefix))
                        })
                {
                    let fragment_path = fs::read_dir_paths(&backup_path)?
                        .into_iter()
                        .find(|path| path.extension() == Some("Caddyfile"))
                        .ok_or_else(|| anyhow::anyhow!("rollback backup has no fragment"))?;
                    fs::write_sensitive_file(&fragment_path, "# tampered rollback fragment\n")?;
                    return Ok::<(), Error>(());
                }
                sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .map_err(|_error| anyhow::anyhow!("rollback backup was not created"))?
    };
    let (result, tamper_result) = tokio::join!(
        reconcile_gateway_runtimes_with_readiness_timeout(&paths, Duration::from_millis(250),),
        tamper_backup,
    );
    tamper_result?;
    let metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.gateway_runtime_metadata())?)?;
    let load_bodies = fake_admin_load_bodies(&paths.gateway_root_config())?;

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    stop_runtime_from_pid_file(&paths.worker_pid("8.4")).await?;

    let Err(DaemonError::CaddyAdmin(CaddyAdminError::RestoredConfigReloadFailed {
        restored_error,
        ..
    })) = result
    else {
        bail!("expected tampered-backup compound failure, got {result:?}");
    };
    assert!(matches!(
        restored_error.as_ref(),
        CaddyAdminError::TaskFailed {
            operation: CaddyAdminOperation::Rollback,
            reason,
        } if reason.contains("does not match the recorded applied fingerprint")
    ));
    assert_eq!(load_bodies.len(), 1);
    assert_eq!(metadata["replacement_required"], true);
    assert!(metadata["applied_config_fingerprint"].is_null());

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_reports_compound_restored_readiness_failure() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let caddy_release = tempdir.path().join("fake-caddy-release");
    write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;
    let old_ports = available_loopback_ports(2)?;
    let new_ports = available_loopback_ports(2)?;

    let mut database = Database::open(&paths)?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    seed_runtime_ports(&paths, &mut database, old_ports[0], old_ports[1], &[])?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let previous_root = read_test_bytes(paths.gateway_root_config())?;
    let failed_admin_statuses = vec![503; 100];
    write_fake_admin_control(
        &paths.gateway_root_config(),
        json!({
            "admin_statuses": failed_admin_statuses,
            "load_statuses": [200, 200],
        }),
    )?;

    let mut database = Database::open(&paths)?;
    database.release_port(PortOwner::Gateway(GatewayPort::Http))?;
    database.release_port(PortOwner::Gateway(GatewayPort::Https))?;
    seed_runtime_ports(&paths, &mut database, new_ports[0], new_ports[1], &[])?;
    drop(database);

    let result =
        reconcile_gateway_runtimes_with_readiness_timeout(&paths, Duration::from_millis(150)).await;
    let root_after = read_test_bytes(paths.gateway_root_config())?;
    let current_config = fake_admin_current_bytes(&paths.gateway_root_config())?;
    let load_bodies = fake_admin_load_bodies(&paths.gateway_root_config())?;
    let requests = fake_admin_requests(&paths.gateway_root_config())?;
    let metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.gateway_runtime_metadata())?)?;

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;

    let Err(DaemonError::CaddyAdmin(CaddyAdminError::RestoredConfigReloadFailed {
        original_error,
        restored_error,
    })) = result
    else {
        bail!("expected restored-readiness compound failure, got {result:?}");
    };
    assert!(matches!(
        original_error.as_ref(),
        CaddyAdminError::TaskFailed { .. } | CaddyAdminError::AdminReadinessTimedOut { .. }
    ));
    assert!(
        matches!(
            restored_error.as_ref(),
            CaddyAdminError::AdminReadinessTimedOut { .. }
        ),
        "unexpected restored readiness error: {restored_error:#?}"
    );
    assert_eq!(root_after, previous_root);
    assert_eq!(current_config, previous_root);
    assert_eq!(load_bodies.len(), 2);
    assert_eq!(load_bodies[1], previous_root);
    assert_eq!(
        requests
            .iter()
            .filter(|request| request["method"] == "POST" && request["path"] == "/load")
            .map(|request| request["status"].as_u64())
            .collect::<Vec<_>>(),
        vec![Some(200), Some(200)]
    );
    assert!(requests.iter().any(|request| {
        request["method"] == "GET" && request["path"] == "/config/" && request["status"] == 503
    }));
    assert_eq!(metadata["replacement_required"], true);
    assert!(metadata["applied_config_fingerprint"].is_null());

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_rolls_back_config_when_runtime_readiness_fails() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let release_path = tempdir.path().join("fake-caddy-release");
    let fake_caddy = release_path.join("bin/caddy");
    let ports = available_loopback_ports(4)?;
    let old_http_port = ports[0];
    let old_https_port = ports[1];
    let new_http_port = ports[2];
    let new_https_port = ports[3];

    write_stateful_fake_caddy(&fake_caddy)?;

    let mut database = Database::open(&paths)?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &release_path,
    )?;
    seed_runtime_ports(&paths, &mut database, old_http_port, old_https_port, &[])?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let first_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected gateway runtime metadata"))?;
    let previous_root_config = fs::read_to_string(&paths.gateway_root_config())?;
    let previous_metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.gateway_runtime_metadata())?)?;
    let first_gateway = ProcessSupervisor::new(paths.clone())
        .adopt_recorded(&paths.gateway_pid(), &paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("first Gateway was not adoptable"))?;
    assert_eq!(first_gateway.pid(), first_gateway_pid);

    let mut database = Database::open(&paths)?;
    database.release_port(PortOwner::Gateway(GatewayPort::Http))?;
    database.release_port(PortOwner::Gateway(GatewayPort::Https))?;
    seed_runtime_ports(&paths, &mut database, new_http_port, new_https_port, &[])?;
    drop(database);

    let result =
        reconcile_gateway_runtimes_with_readiness_timeout(&paths, Duration::from_millis(250)).await;
    let root_config = fs::read_to_string(&paths.gateway_root_config())?;
    let load_bodies = fake_admin_load_bodies(&paths.gateway_root_config())?;
    let requests = fake_admin_requests(&paths.gateway_root_config())?;
    let restored_metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.gateway_runtime_metadata())?)?;
    let first_gateway_is_alive = process_is_alive(first_gateway_pid)?;
    if first_gateway_is_alive {
        first_gateway.stop(Duration::from_secs(1)).await?;
        fs::remove_file_if_exists(&paths.gateway_pid())?;
        fs::remove_file_if_exists(&paths.gateway_runtime_metadata())?;
    } else if paths.gateway_pid().exists() {
        stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    }

    let Err(error) = result else {
        bail!("expected readiness failure, got success");
    };
    assert!(
        matches!(error, DaemonError::ReadinessTimedOut { .. }),
        "unexpected readiness failure: {error:?}"
    );
    assert_eq!(root_config, previous_root_config);
    assert!(first_gateway_is_alive);
    assert_eq!(
        restored_metadata["applied_config_fingerprint"],
        previous_metadata["applied_config_fingerprint"]
    );
    assert!(restored_metadata["applied_config_fingerprint"].is_string());
    assert_ne!(restored_metadata["replacement_required"], true);
    assert_eq!(load_bodies.len(), 2);
    assert_ne!(load_bodies[0], previous_root_config.as_bytes());
    assert_eq!(load_bodies[1], previous_root_config.as_bytes());
    assert_eq!(
        requests
            .iter()
            .filter(|request| request["method"] == "POST" && request["path"] == "/load")
            .map(|request| request["status"].as_u64())
            .collect::<Vec<_>>(),
        vec![Some(200), Some(200)]
    );
    assert!(
        requests
            .iter()
            .any(|request| { request["method"] == "GET" && request["path"] == "/config/" })
    );

    runtime_guard.cleanup().await?;

    Ok(())
}

fn assert_worker_command(paths: &PvPaths, php_track: &str, expected: &Utf8Path) -> Result<()> {
    let metadata = fs::read_to_string(&paths.worker_runtime_metadata(php_track))?;
    let metadata: serde_json::Value = serde_json::from_str(&metadata)?;

    assert_eq!(metadata["command"], expected.as_str());

    Ok(())
}

#[tokio::test]
async fn runtime_plan_groups_linked_projects_by_php_track() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let acme = tempdir.path().join("acme");
    let other = tempdir.path().join("other/api");

    create_project(
        &acme,
        r#"php: "8.4"
document_root: public
hostnames:
  - api.acme.test
"#,
    )?;
    create_project(
        &other,
        r#"php: "8.3"
document_root: public
"#,
    )?;

    let mut database = Database::open(&paths)?;
    database.link_project(LinkProjectInput {
        path: acme.clone(),
        original_path: acme.clone(),
        primary_hostname: "acme.test".to_owned(),
        config_path: acme.join("pv.yml"),
        desired_php_track: None,
        additional_hostnames: vec!["api.acme.test".to_owned()],
    })?;
    database.link_project(LinkProjectInput {
        path: other.clone(),
        original_path: other.clone(),
        primary_hostname: "other.test".to_owned(),
        config_path: other.join("pv.yml"),
        desired_php_track: None,
        additional_hostnames: Vec::new(),
    })?;
    seed_stable_runtime_plan_ports(&mut database, &["8.4", "8.3"])?;
    drop(database);

    let plan = build_runtime_plan(&paths)?;

    assert_runtime_plan_snapshot("runtime_plan_groups_linked_projects_by_php_track", plan);

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn runtime_plan_excludes_resource_only_project_with_explicit_php() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let served = tempdir.path().join("served");
    let resource_only = tempdir.path().join("resource-only");
    create_project(&served, "php: \"8.4\"\n")?;
    create_project(&resource_only, "serve: false\nphp: \"8.4\"\n")?;

    let mut database = Database::open(&paths)?;
    let served = database
        .link_project(LinkProjectInput {
            path: served.clone(),
            original_path: served.clone(),
            primary_hostname: "served.test".to_owned(),
            config_path: served.join("pv.yml"),
            desired_php_track: Some("8.4".to_owned()),
            additional_hostnames: Vec::new(),
        })?
        .project;
    let resource_only = database
        .link_project_with_mode(
            LinkProjectInput {
                path: resource_only.clone(),
                original_path: resource_only.clone(),
                primary_hostname: "ignored.test".to_owned(),
                config_path: resource_only.join("pv.yml"),
                desired_php_track: Some("8.4".to_owned()),
                additional_hostnames: Vec::new(),
            },
            ProjectMode::ResourceOnly,
        )?
        .project;
    seed_stable_runtime_plan_ports(&mut database, &["8.4"])?;
    drop(database);

    let plan = build_runtime_plan(&paths)?;
    let project_ids = plan
        .workers
        .iter()
        .flat_map(|worker| worker.projects.iter())
        .map(|project| project.id.as_str())
        .collect::<Vec<_>>();

    assert_eq!(project_ids, [served.id.as_str()]);
    assert!(!project_ids.contains(&resource_only.id.as_str()));

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn runtime_plan_preserves_served_project_while_resource_only_transition_is_pending()
-> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let project_root = tempdir.path().join("served");
    create_project(&project_root, "php: \"8.4\"\n")?;

    let mut database = Database::open(&paths)?;
    let project = database
        .link_project(LinkProjectInput {
            path: project_root.clone(),
            original_path: project_root.clone(),
            primary_hostname: "served.test".to_owned(),
            config_path: project_root.join("pv.yml"),
            desired_php_track: Some("8.4".to_owned()),
            additional_hostnames: Vec::new(),
        })?
        .project;
    seed_stable_runtime_plan_ports(&mut database, &["8.4"])?;
    drop(database);
    fs::write_sensitive_file(&project_root.join("pv.yml"), "serve: false\nphp: \"8.4\"\n")?;

    let plan = build_runtime_plan(&paths)?;
    let planned_project = plan
        .workers
        .iter()
        .flat_map(|worker| worker.projects.iter())
        .find(|candidate| candidate.id == project.id)
        .ok_or_else(|| anyhow::anyhow!("expected persisted served Project in runtime plan"))?;

    assert!(!planned_project.render_config);
    assert_eq!(planned_project.primary_hostname, "served.test");

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_runtime_plan_groups_projects_by_php_track_and_extensions() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let acme = create_project_with_config(
        tempdir.path(),
        "acme",
        "php:\n  version: 8.4\n  extensions: [redis]\n",
    )?;
    let api = create_project_with_config(
        tempdir.path(),
        "api",
        "php:\n  version: 8.4\n  extensions: [xdebug, redis]\n",
    )?;
    let release = seed_installed_php_with_extensions(&paths, "8.4", &["redis", "xdebug"])?;
    seed_installed_frankenphp_with_extensions(&paths, "8.4", &release, &["redis", "xdebug"])?;
    link_project_record(&paths, &acme, "acme.test", Some("8.4"))?;
    link_project_record(&paths, &api, "api.test", Some("8.4"))?;

    let plan = daemon::gateway::build_runtime_plan(&paths)?;
    let runtime_keys = plan
        .workers
        .iter()
        .map(|worker| worker.runtime_key.as_str())
        .collect::<Vec<_>>();

    assert_eq!(runtime_keys, ["8.4+redis", "8.4+redis+xdebug"]);

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn runtime_plan_resolves_latest_php_track_from_cached_manifest() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let project_root = tempdir.path().join("latest-project");
    seed_php_manifest(&paths, "8.4")?;
    create_project(
        &project_root,
        r#"php: latest
document_root: public
"#,
    )?;

    let mut database = Database::open(&paths)?;
    database.link_project(LinkProjectInput {
        path: project_root.clone(),
        original_path: project_root.clone(),
        primary_hostname: "latest.test".to_owned(),
        config_path: project_root.join("pv.yml"),
        desired_php_track: None,
        additional_hostnames: Vec::new(),
    })?;
    seed_stable_runtime_plan_ports(&mut database, &["8.4"])?;
    drop(database);

    let plan = build_runtime_plan(&paths)?;

    assert_runtime_plan_snapshot(
        "runtime_plan_resolves_latest_php_track_from_cached_manifest",
        plan,
    );

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn runtime_plan_defaults_document_root_to_public_directory_without_config() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let project_root = tempdir.path().join("configless-project");
    seed_php_manifest(&paths, "8.4")?;
    create_project_without_config(&project_root, true)?;

    let mut database = Database::open(&paths)?;
    database.link_project(LinkProjectInput {
        path: project_root.clone(),
        original_path: project_root.clone(),
        primary_hostname: "configless.test".to_owned(),
        config_path: project_root.join("pv.yml"),
        desired_php_track: None,
        additional_hostnames: Vec::new(),
    })?;
    seed_stable_runtime_plan_ports(&mut database, &["8.4"])?;
    drop(database);

    let plan = build_runtime_plan(&paths)?;

    assert_runtime_plan_snapshot(
        "runtime_plan_defaults_document_root_to_public_directory_without_config",
        plan,
    );

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn runtime_plan_defaults_document_root_to_project_root_without_public_directory() -> Result<()>
{
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let project_root = tempdir.path().join("configless-static-project");
    seed_php_manifest(&paths, "8.4")?;
    create_project_without_config(&project_root, false)?;

    let mut database = Database::open(&paths)?;
    database.link_project(LinkProjectInput {
        path: project_root.clone(),
        original_path: project_root.clone(),
        primary_hostname: "static.test".to_owned(),
        config_path: project_root.join("pv.yml"),
        desired_php_track: None,
        additional_hostnames: Vec::new(),
    })?;
    seed_stable_runtime_plan_ports(&mut database, &["8.4"])?;
    drop(database);

    let plan = build_runtime_plan(&paths)?;

    assert_runtime_plan_snapshot(
        "runtime_plan_defaults_document_root_to_project_root_without_public_directory",
        plan,
    );

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn runtime_plan_uses_project_root_not_original_or_config_path() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let project_root = tempdir.path().join("canonical-project");
    let original_path = tempdir.path().join("typed-project-path");
    let stored_config_path = tempdir.path().join("stale-config-location/pv.yml");

    create_project(
        &project_root,
        r#"php: "8.4"
document_root: public
"#,
    )?;
    fs::write_sensitive_file(
        &stored_config_path,
        r#"php: "8.3"
document_root: other-public
"#,
    )?;

    let mut database = Database::open(&paths)?;
    database.link_project(LinkProjectInput {
        path: project_root.clone(),
        original_path,
        primary_hostname: "acme.test".to_owned(),
        config_path: stored_config_path,
        desired_php_track: None,
        additional_hostnames: Vec::new(),
    })?;
    seed_stable_runtime_plan_ports(&mut database, &["8.4"])?;
    drop(database);

    let plan = build_runtime_plan(&paths)?;

    assert_runtime_plan_snapshot(
        "runtime_plan_uses_project_root_not_original_or_config_path",
        plan,
    );

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn gateway_config_validation_failure_preserves_active_config_and_cleans_candidate()
-> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    fs::ensure_layout(&paths)?;
    fs::write_sensitive_file(&paths.gateway_root_config(), "previous config\n")?;
    let mut candidate_path = None;

    let result = promote_validated_config_for_test(
        &paths.gateway_root_config(),
        "new config\n",
        |candidate| {
            candidate_path = Some(candidate.to_path_buf());
            Err(DaemonError::UnexpectedProtocolResponse {
                reason: "validation failed".to_owned(),
            })
        },
    );

    assert!(matches!(
        result,
        Err(DaemonError::UnexpectedProtocolResponse { .. })
    ));
    assert_eq!(
        fs::read_to_string(&paths.gateway_root_config())?,
        "previous config\n"
    );
    let candidate_removed = candidate_path
        .as_ref()
        .is_some_and(|candidate| !candidate.exists());
    assert!(candidate_removed);

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn frankenphp_config_validation_reports_process_failures() -> Result<()> {
    let tempdir = tempdir()?;
    let validator = write_failing_validator(&tempdir.path().join("validator"))?;
    let config_path = tempdir.path().join("Caddyfile");
    fs::write_sensitive_file(&config_path, "invalid config\n")?;

    let result = validate_config(
        &CaddyCliCommand::frankenphp(validator),
        &config_path,
        &BTreeMap::new(),
    )
    .await;

    assert!(matches!(
        result,
        Err(DaemonError::UnexpectedProtocolResponse { reason })
            if reason.contains("stdout=validator stdout") && reason.contains("stderr=validator stderr")
    ));

    Ok(())
}

#[tokio::test]
async fn caddy_config_validation_reports_the_caddy_runtime_label() -> Result<()> {
    let tempdir = tempdir()?;
    let validator = write_failing_validator(&tempdir.path().join("validator"))?;
    let config_path = tempdir.path().join("Caddyfile");
    fs::write_sensitive_file(&config_path, "invalid config\n")?;

    let result = validate_config(
        &CaddyCliCommand::caddy(validator),
        &config_path,
        &BTreeMap::new(),
    )
    .await;

    assert!(matches!(
        result,
        Err(DaemonError::UnexpectedProtocolResponse { reason })
            if reason.contains("Caddy config validation failed")
                && !reason.contains("FrankenPHP config validation")
    ));

    Ok(())
}

#[tokio::test]
async fn caddy_cli_command_and_process_specs_are_stable() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let gateway_command = CaddyCliCommand::caddy(tempdir.path().join("caddy"));
    let worker_command = CaddyCliCommand::frankenphp(tempdir.path().join("frankenphp"));
    let gateway = gateway_process_spec(&paths, &gateway_command);
    let worker_plan = php_worker_plan("8.4");
    let worker = worker_process_spec(&paths, &worker_plan, &worker_command, tempdir.path())?;

    assert_eq!(
        gateway
            .private_environment
            .get("XDG_CONFIG_HOME")
            .map(String::as_str),
        Some(paths.config().as_str())
    );
    assert_eq!(
        gateway
            .private_environment
            .get("XDG_DATA_HOME")
            .map(String::as_str),
        Some(paths.certificates().as_str())
    );
    assert_eq!(
        worker
            .private_environment
            .get("XDG_CONFIG_HOME")
            .map(String::as_str),
        Some(paths.config().as_str())
    );
    assert_eq!(
        worker
            .private_environment
            .get("XDG_DATA_HOME")
            .map(String::as_str),
        Some(paths.certificates().as_str())
    );
    assert_eq!(gateway.private_environment.get("PHPRC"), None);
    assert_eq!(gateway.private_environment.get("PHP_INI_SCAN_DIR"), None);
    assert_eq!(
        worker.private_environment.get("PHPRC").map(String::as_str),
        Some(paths.resources().join("php/8.4/etc").as_str())
    );
    assert_eq!(
        worker
            .private_environment
            .get("PHP_INI_SCAN_DIR")
            .map(String::as_str),
        Some(paths.resources().join("php/8.4/etc/conf.d").as_str())
    );
    assert_eq!(gateway.resource_name, "caddy");
    assert_eq!(gateway.track, "2");
    assert_eq!(gateway.log_path, paths.gateway_supervisor_log());
    assert_eq!(worker.resource_name, "frankenphp");
    assert_eq!(worker.track, "8.4");

    assert_process_spec_snapshot(
        tempdir.path(),
        (
            gateway_command.validate_arguments(&paths.gateway_root_config()),
            gateway_command.run_arguments(&paths.gateway_root_config()),
            gateway,
            worker,
        ),
    );

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn frankenphp_config_validation_receives_xdg_environment() -> Result<()> {
    let tempdir = tempdir()?;
    let validator = tempdir.path().join("env-validator");
    let config_path = tempdir.path().join("Caddyfile");
    let xdg_config_home = tempdir.path().join("pv-config");
    let xdg_data_home = tempdir.path().join("pv-data");
    let observed_config_home = tempdir.path().join("observed-config-home");
    let observed_data_home = tempdir.path().join("observed-data-home");
    let observed_phprc = tempdir.path().join("observed-phprc");
    let observed_scan_dir = tempdir.path().join("observed-scan-dir");
    fs::write_sensitive_file(
        &validator,
        &format!(
            r#"#!/bin/sh
set -eu
printf '%s' "${{XDG_CONFIG_HOME}}" > {}
printf '%s' "${{XDG_DATA_HOME}}" > {}
printf '%s' "${{PHPRC}}" > {}
printf '%s' "${{PHP_INI_SCAN_DIR}}" > {}
exit 0
"#,
            shell_single_quoted(observed_config_home.as_str()),
            shell_single_quoted(observed_data_home.as_str()),
            shell_single_quoted(observed_phprc.as_str()),
            shell_single_quoted(observed_scan_dir.as_str()),
        ),
    )?;
    set_executable(&validator)?;
    fs::write_sensitive_file(&config_path, "{}\n")?;
    let private_environment = BTreeMap::from([
        (
            "XDG_CONFIG_HOME".to_owned(),
            xdg_config_home.as_str().to_owned(),
        ),
        (
            "XDG_DATA_HOME".to_owned(),
            xdg_data_home.as_str().to_owned(),
        ),
        (
            "PHPRC".to_owned(),
            tempdir.path().join("php/etc").as_str().to_owned(),
        ),
        (
            "PHP_INI_SCAN_DIR".to_owned(),
            tempdir.path().join("php/etc/conf.d").as_str().to_owned(),
        ),
    ]);

    validate_config(
        &CaddyCliCommand::frankenphp(&validator),
        &config_path,
        &private_environment,
    )
    .await?;

    assert_eq!(
        state::testing::read_to_string(&observed_config_home)?,
        xdg_config_home.as_str()
    );
    assert_eq!(
        state::testing::read_to_string(&observed_data_home)?,
        xdg_data_home.as_str()
    );
    assert_eq!(
        state::testing::read_to_string(&observed_phprc)?,
        tempdir.path().join("php/etc").to_string()
    );
    assert_eq!(
        state::testing::read_to_string(&observed_scan_dir)?,
        tempdir.path().join("php/etc/conf.d").to_string()
    );

    Ok(())
}

#[tokio::test]
async fn gateway_config_validation_strips_parent_php_ini_env_when_private_env_omits_it()
-> Result<()> {
    let tempdir = tempdir()?;
    let output = run_ignored_test_with_parent_php_ini_env(
        "gateway_config_validation_strips_parent_php_ini_env_inner",
        tempdir.path(),
    )?;

    assert_nested_test_succeeded(output)
}

#[tokio::test]
#[ignore]
async fn gateway_config_validation_strips_parent_php_ini_env_inner() -> Result<()> {
    let root = Utf8Path::new(".");
    let validator = root.join("env-validator");
    let config_path = root.join("Caddyfile");
    let observed_phprc = root.join("observed-phprc");
    let observed_scan_dir = root.join("observed-scan-dir");
    fs::write_sensitive_file(
        &validator,
        &format!(
            r#"#!/bin/sh
set -eu
printf '%s' "${{PHPRC-}}" > {}
printf '%s' "${{PHP_INI_SCAN_DIR-}}" > {}
exit 0
"#,
            shell_single_quoted(observed_phprc.as_str()),
            shell_single_quoted(observed_scan_dir.as_str()),
        ),
    )?;
    set_executable(&validator)?;
    fs::write_sensitive_file(&config_path, "{}\n")?;
    let command = CaddyCliCommand::caddy(&validator);
    let paths = PvPaths::for_home(root.join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let private_environment = gateway_process_spec(&paths, &command).private_environment;

    validate_config(&command, &config_path, &private_environment).await?;

    assert_eq!(state::testing::read_to_string(&observed_phprc)?, "");
    assert_eq!(state::testing::read_to_string(&observed_scan_dir)?, "");

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn worker_config_validation_keeps_private_php_ini_env_after_parent_removal() -> Result<()> {
    let tempdir = tempdir()?;
    let output = run_ignored_test_with_parent_php_ini_env(
        "worker_config_validation_keeps_private_php_ini_env_after_parent_removal_inner",
        tempdir.path(),
    )?;

    assert_nested_test_succeeded(output)
}

#[tokio::test]
#[ignore]
async fn worker_config_validation_keeps_private_php_ini_env_after_parent_removal_inner()
-> Result<()> {
    let root = Utf8Path::new(".");
    let validator = root.join("env-validator");
    let config_path = root.join("Caddyfile");
    let observed_phprc = root.join("observed-phprc");
    let observed_scan_dir = root.join("observed-scan-dir");
    fs::write_sensitive_file(
        &validator,
        &format!(
            r#"#!/bin/sh
set -eu
printf '%s' "${{PHPRC-}}" > {}
printf '%s' "${{PHP_INI_SCAN_DIR-}}" > {}
exit 0
"#,
            shell_single_quoted(observed_phprc.as_str()),
            shell_single_quoted(observed_scan_dir.as_str()),
        ),
    )?;
    set_executable(&validator)?;
    fs::write_sensitive_file(&config_path, "{}\n")?;
    let command = CaddyCliCommand::frankenphp(&validator);
    let paths = PvPaths::for_home(root.join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let expected_phprc = paths.resources().join("php/8.4/etc").to_string();
    let expected_scan_dir = paths.resources().join("php/8.4/etc/conf.d").to_string();
    let worker_plan = php_worker_plan("8.4");
    let private_environment =
        worker_process_spec(&paths, &worker_plan, &command, root)?.private_environment;

    validate_config(&command, &config_path, &private_environment).await?;

    assert_eq!(
        state::testing::read_to_string(&observed_phprc)?,
        expected_phprc
    );
    assert_eq!(
        state::testing::read_to_string(&observed_scan_dir)?,
        expected_scan_dir
    );

    runtime_guard.cleanup().await?;

    Ok(())
}

fn create_project(project_root: &Utf8Path, config_source: &str) -> Result<()> {
    fs::write_sensitive_file(&project_root.join("public/index.php"), "<?php\n")?;
    fs::write_sensitive_file(&project_root.join("pv.yml"), config_source)?;

    Ok(())
}

fn create_project_with_config(
    workspace_root: &Utf8Path,
    project_name: &str,
    config_source: &str,
) -> Result<camino::Utf8PathBuf> {
    let project_root = workspace_root.join(project_name);

    create_project(&project_root, config_source)?;

    Ok(project_root)
}

fn create_project_without_config(project_root: &Utf8Path, public_directory: bool) -> Result<()> {
    let index_path = if public_directory {
        project_root.join("public/index.php")
    } else {
        project_root.join("index.php")
    };
    fs::write_sensitive_file(&index_path, "<?php\n")?;

    Ok(())
}

fn php_worker_plan(runtime_key: &str) -> daemon::gateway::PhpWorkerRuntimePlan {
    daemon::gateway::PhpWorkerRuntimePlan {
        php_track: "8.4".to_owned(),
        runtime_key: runtime_key.to_owned(),
        loaded_modules: Vec::new(),
        port: RUNTIME_PORT_FALLBACK_START,
        admin_socket_path: Utf8PathBuf::from("/tmp/pv-worker-admin.sock"),
        projects: Vec::new(),
    }
}

fn write_failing_validator(path: &Utf8Path) -> Result<camino::Utf8PathBuf> {
    fs::write_sensitive_file(
        path,
        "#!/bin/sh\necho validator stdout\necho validator stderr >&2\nexit 42\n",
    )?;
    set_executable(path)?;

    Ok(path.to_path_buf())
}

fn write_failing_config_validator(path: &Utf8Path) -> Result<()> {
    fs::write_sensitive_file(
        path,
        r#"#!/bin/sh
set -eu

if [ "$1" = "validate" ]; then
  echo validation failed >&2
  exit 42
fi

exit 2
"#,
    )?;
    set_executable(path)?;

    Ok(())
}

fn write_hanging_frankenphp_validator(path: &Utf8Path, child_pid_path: &Utf8Path) -> Result<()> {
    fs::write_sensitive_file(
        path,
        &format!(
            r#"#!/bin/sh
set -eu

if [ "$1" = "validate" ]; then
  sleep 30 &
  echo "$!" > {}
  wait "$!"
fi

exit 2
"#,
            shell_single_quoted(child_pid_path.as_str())
        ),
    )?;
    set_executable(path)?;

    Ok(())
}

fn write_fake_frankenphp(path: &Utf8Path) -> Result<()> {
    write_runtime_fixture(path, FAKE_FRANKENPHP_SCRIPT, FAKE_FRANKENPHP_SERVER_SCRIPT)
}

fn ensure_fake_caddy(paths: &PvPaths) -> Result<()> {
    let release_path = paths.home().join("fake-caddy-release");
    let executable = release_path.join("bin/caddy");
    let mut database = Database::open(paths)?;
    if database
        .managed_resource_tracks()?
        .iter()
        .any(|record| record.resource_name == "caddy" && record.track == "2")
    {
        return Ok(());
    }

    write_fake_caddy(&executable)?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &release_path,
    )?;

    Ok(())
}

fn write_fake_caddy(path: &Utf8Path) -> Result<()> {
    write_fake_caddy_fixture(path, FAKE_CADDY_SCRIPT, FAKE_CADDY_SERVER_SCRIPT)
}

fn write_stateful_fake_caddy(path: &Utf8Path) -> Result<()> {
    write_runtime_fixture(
        path,
        FAKE_STATEFUL_CADDY_SCRIPT,
        FAKE_STATEFUL_RUNTIME_SERVER_SCRIPT,
    )
}

fn write_stateful_fake_frankenphp(path: &Utf8Path) -> Result<()> {
    write_runtime_fixture(
        path,
        FAKE_STATEFUL_FRANKENPHP_SCRIPT,
        FAKE_STATEFUL_RUNTIME_SERVER_SCRIPT,
    )
}

fn write_fast_exit_runtime(path: &Utf8Path) -> Result<()> {
    fs::write_sensitive_file(
        path,
        r#"#!/bin/sh
set -eu

if [ "$1" = "validate" ]; then
  exit 0
fi

if [ "$1" = "run" ]; then
  sleep 1
  exit 42
fi

exit 2
"#,
    )?;
    set_executable(path)?;

    Ok(())
}

fn fake_admin_control_path(config_path: &Utf8Path) -> Utf8PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| Utf8Path::new("."))
        .join("fake-admin-control.json")
}

fn write_fake_admin_control(config_path: &Utf8Path, control: Value) -> Result<()> {
    fs::write_sensitive_file(
        &fake_admin_control_path(config_path),
        &serde_json::to_string(&control)?,
    )?;

    Ok(())
}

fn fake_admin_load_bodies(config_path: &Utf8Path) -> Result<Vec<Vec<u8>>> {
    let directory = config_path.parent().unwrap_or_else(|| Utf8Path::new("."));
    let mut paths = fs::read_dir_paths(directory)?
        .into_iter()
        .filter(|path| {
            path.file_name()
                .is_some_and(|file_name| file_name.starts_with("fake-admin-load-"))
        })
        .collect::<Vec<_>>();
    paths.sort_unstable();

    paths.into_iter().map(read_test_bytes).collect()
}

fn fake_validator_spawns(config_path: &Utf8Path) -> Result<usize> {
    let path = config_path
        .parent()
        .unwrap_or_else(|| Utf8Path::new("."))
        .join("fake-validator-spawns.log");
    if !path.exists() {
        return Ok(0);
    }

    Ok(fs::read_to_string(&path)?.lines().count())
}

fn fake_admin_requests(config_path: &Utf8Path) -> Result<Vec<Value>> {
    let path = config_path
        .parent()
        .unwrap_or_else(|| Utf8Path::new("."))
        .join("fake-admin-requests.jsonl");
    if !path.exists() {
        return Ok(Vec::new());
    }

    fs::read_to_string(&path)?
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| Ok(serde_json::from_str(line)?))
        .collect()
}

fn fake_admin_current_bytes(config_path: &Utf8Path) -> Result<Vec<u8>> {
    let path = config_path
        .parent()
        .unwrap_or_else(|| Utf8Path::new("."))
        .join("fake-admin-current.bin");
    read_test_bytes(path)
}

#[expect(
    clippy::disallowed_methods,
    reason = "stateful admin fixture assertions must compare exact request bytes"
)]
fn read_test_bytes(path: Utf8PathBuf) -> Result<Vec<u8>> {
    Ok(std::fs::read(path)?)
}

#[expect(
    clippy::disallowed_methods,
    reason = "runtime recovery tests must write generated config bytes that are not UTF-8"
)]
fn write_test_bytes(path: &Utf8Path, content: &[u8]) -> Result<()> {
    std::fs::write(path, content)?;

    Ok(())
}

fn write_fake_caddy_without_admin(path: &Utf8Path) -> Result<()> {
    write_fake_caddy_fixture(
        path,
        FAKE_CADDY_NO_ADMIN_SCRIPT,
        FAKE_CADDY_NO_ADMIN_SERVER_SCRIPT,
    )
}

fn write_fake_caddy_admin_only(path: &Utf8Path) -> Result<()> {
    write_fake_caddy_fixture(
        path,
        FAKE_CADDY_ADMIN_ONLY_SCRIPT,
        FAKE_CADDY_ADMIN_ONLY_SERVER_SCRIPT,
    )
}

fn write_fake_caddy_legacy(path: &Utf8Path) -> Result<()> {
    write_fake_caddy_fixture(
        path,
        FAKE_CADDY_LEGACY_SCRIPT,
        FAKE_CADDY_LEGACY_SERVER_SCRIPT,
    )
}

fn write_fake_caddy_fixture(
    path: &Utf8Path,
    shell_script: &str,
    server_script: &str,
) -> Result<()> {
    write_runtime_fixture(path, shell_script, server_script)
}

fn write_runtime_fixture(path: &Utf8Path, shell_script: &str, server_script: &str) -> Result<()> {
    let server_path = Utf8PathBuf::from(format!("{path}.server.py"));

    fs::write_sensitive_file(&server_path, server_script)?;
    fs::write_sensitive_file(path, shell_script)?;
    set_executable(path)?;

    Ok(())
}

fn shell_single_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn run_ignored_test_with_parent_php_ini_env(
    test_name: &str,
    working_dir: &Utf8Path,
) -> Result<Output> {
    let mut command = TestProcessCommand::new(current_test_binary()?);
    command
        .args(["--exact", test_name, "--ignored", "--nocapture"])
        .current_dir(working_dir)
        .env("PHPRC", "parent-phprc")
        .env("PHP_INI_SCAN_DIR", "parent-scan-dir");

    Ok(command.output()?)
}

fn current_test_binary() -> Result<OsString> {
    std::env::args_os()
        .next()
        .ok_or_else(|| anyhow::anyhow!("test binary path was missing"))
}

fn assert_nested_test_succeeded(output: Output) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }

    anyhow::bail!(
        "nested test failed: status={}; stdout={}; stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn seed_stable_runtime_plan_ports(database: &mut Database, php_tracks: &[&str]) -> Result<()> {
    database.assign_gateway_ports(|_port| true)?;

    for (index, php_track) in php_tracks.iter().enumerate() {
        let preferred_port = RUNTIME_PORT_FALLBACK_START + u16::try_from(index)?;
        database.assign_port(
            PortRequest::php_worker(
                *php_track,
                preferred_port,
                RUNTIME_PORT_FALLBACK_START,
                RUNTIME_PORT_FALLBACK_END,
            ),
            |_port| true,
        )?;
    }

    Ok(())
}

fn link_project_record(
    paths: &PvPaths,
    project_root: &Utf8Path,
    primary_hostname: &str,
    desired_php_track: Option<&str>,
) -> Result<()> {
    let mut database = Database::open(paths)?;

    database.link_project(LinkProjectInput {
        path: project_root.to_path_buf(),
        original_path: project_root.to_path_buf(),
        primary_hostname: primary_hostname.to_owned(),
        config_path: project_root.join("pv.yml"),
        desired_php_track: desired_php_track.map(str::to_owned),
        additional_hostnames: Vec::new(),
    })?;

    Ok(())
}

fn seed_installed_php_with_extensions(
    paths: &PvPaths,
    track: &str,
    extensions: &[&str],
) -> Result<camino::Utf8PathBuf> {
    let release = paths
        .home()
        .join(format!("{track}-php-release"))
        .to_path_buf();
    let metadata = extension_metadata(extensions)?;
    let mut database = Database::open(paths)?;

    fs::write_sensitive_file(&release.join("bin/php"), "#!/bin/sh\n")?;
    fs::write_sensitive_file(&release.join("share/pv/php-extensions.json"), &metadata)?;
    for extension in extensions {
        fs::write_sensitive_file(
            &release.join(format!("lib/php/extensions/{extension}.so")),
            "",
        )?;
    }
    database.record_managed_resource_track_installed("php", track, "8.4.8-pv1", &release)?;

    Ok(release)
}

fn seed_installed_frankenphp_with_extensions(
    paths: &PvPaths,
    track: &str,
    release: &Utf8Path,
    extensions: &[&str],
) -> Result<()> {
    let metadata = extension_metadata(extensions)?;
    let mut database = Database::open(paths)?;

    fs::write_sensitive_file(&release.join("bin/frankenphp"), "#!/bin/sh\n")?;
    fs::write_sensitive_file(&release.join("share/pv/php-extensions.json"), &metadata)?;
    for extension in extensions {
        fs::write_sensitive_file(
            &release.join(format!("lib/php/extensions/{extension}.so")),
            "",
        )?;
    }
    database.record_managed_resource_track_installed("frankenphp", track, "8.4.8-pv1", release)?;

    Ok(())
}

fn extension_metadata(extensions: &[&str]) -> Result<String> {
    let modules = extensions
        .iter()
        .map(|extension| {
            json!({
                "name": extension,
                "load_kind": if *extension == "xdebug" { "zend_extension" } else { "extension" },
                "path": format!("lib/php/extensions/{extension}.so"),
            })
        })
        .collect::<Vec<_>>();

    Ok(serde_json::to_string(&modules)?)
}

fn seed_runtime_ports(
    paths: &PvPaths,
    database: &mut Database,
    gateway_http_port: u16,
    gateway_https_port: u16,
    php_workers: &[(&str, u16)],
) -> Result<()> {
    seed_gateway_test_tls(paths)?;
    database.assign_port(
        PortRequest::gateway(
            GatewayPort::Http,
            gateway_http_port,
            gateway_http_port,
            gateway_http_port,
        ),
        |_port| true,
    )?;
    database.assign_port(
        PortRequest::gateway(
            GatewayPort::Https,
            gateway_https_port,
            gateway_https_port,
            gateway_https_port,
        ),
        |_port| true,
    )?;
    for (php_track, port) in php_workers {
        database.assign_port(
            PortRequest::php_worker(*php_track, *port, *port, *port),
            |_port| true,
        )?;
    }

    Ok(())
}

fn seed_gateway_test_tls(paths: &PvPaths) -> Result<()> {
    // Keep these hostnames in sync with gateway reconciliation fixtures that
    // perform HTTPS readiness checks against the seeded CA.
    let certified_key = generate_simple_self_signed(vec![
        "acme.test".to_owned(),
        "api.acme.test".to_owned(),
        "broken.test".to_owned(),
        "changed.acme.test".to_owned(),
        "other.test".to_owned(),
        "pv-gateway.localhost".to_owned(),
    ])?;
    fs::write_sensitive_file(&paths.ca_certificate(), &certified_key.cert.pem())?;
    fs::write_sensitive_file(
        &paths.ca_private_key(),
        &certified_key.signing_key.serialize_pem(),
    )?;

    Ok(())
}

fn available_loopback_ports(count: usize) -> Result<Vec<u16>> {
    let listeners = reserve_loopback_ports(count)?;
    loopback_ports(&listeners)
}

async fn wait_for_existing_path_count(paths: &[Utf8PathBuf], expected: usize) -> Result<()> {
    timeout(Duration::from_secs(10), async {
        loop {
            let existing = paths.iter().filter(|path| path.exists()).count();
            if existing == expected {
                return;
            }

            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .with_context(|| format!("expected {expected} readiness probes"))?;

    Ok(())
}

fn reserve_loopback_ports(count: usize) -> Result<Vec<TcpListener>> {
    let mut listeners = Vec::with_capacity(count);
    let mut ports = Vec::with_capacity(count);

    while ports.len() < count {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        if ports.contains(&port) {
            continue;
        }

        ports.push(port);
        listeners.push(listener);
    }

    Ok(listeners)
}

fn reserve_loopback_ports_in_range(count: usize, start: u16, end: u16) -> Result<Vec<TcpListener>> {
    let mut listeners = Vec::with_capacity(count);

    for port in start..=end {
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => listeners.push(listener),
            Err(error) if error.kind() == ErrorKind::AddrInUse => continue,
            Err(error) => return Err(error.into()),
        }

        if listeners.len() == count {
            return Ok(listeners);
        }
    }

    bail!("expected {count} available loopback ports in {start}..={end}")
}

fn loopback_ports(listeners: &[TcpListener]) -> Result<Vec<u16>> {
    listeners
        .iter()
        .map(|listener| Ok(listener.local_addr()?.port()))
        .collect()
}

#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "integration test deliberately drifts generated config permissions"
)]
fn set_test_mode(path: &Utf8Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(mode);
    std::fs::set_permissions(path, permissions)?;

    Ok(())
}

#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "integration test directly inspects generated config permissions"
)]
fn test_mode(path: &Utf8Path) -> Result<u32> {
    use std::os::unix::fs::PermissionsExt;

    Ok(std::fs::metadata(path)?.permissions().mode() & 0o777)
}

#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "test fixture marks fake FrankenPHP validator executable"
)]
fn set_executable(path: &Utf8Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(path, permissions)?;

    Ok(())
}

#[derive(Clone)]
struct RecordedRuntime {
    pid_path: Utf8PathBuf,
    metadata_path: Utf8PathBuf,
}

#[derive(Clone, Copy)]
struct CapturedProcessGroupMember {
    pid: Pid,
    start_identity: ProcessStartIdentity,
}

struct CapturedProcessGroupGuard {
    process_group: Pid,
    member: CapturedProcessGroupMember,
    armed: bool,
}

impl CapturedProcessGroupGuard {
    fn new(leader_pid: u32) -> Result<Self> {
        let leader = Pid::from_raw(i32::try_from(leader_pid)?)
            .ok_or_else(|| anyhow::anyhow!("invalid process id {leader_pid}"))?;
        let process_group = getpgid(Some(leader))?;
        if process_group != leader {
            bail!("fixture leader {leader} did not own process group {process_group}");
        }
        let leader = capture_process_group_member(leader, process_group)?;

        Ok(Self {
            process_group,
            member: leader,
            armed: true,
        })
    }

    fn capture(&mut self, pid: u32) -> Result<()> {
        let pid = Pid::from_raw(i32::try_from(pid)?)
            .ok_or_else(|| anyhow::anyhow!("invalid process id {pid}"))?;
        self.member = capture_process_group_member(pid, self.process_group)?;

        Ok(())
    }

    fn member_is_live(&self) -> Result<bool> {
        captured_process_group_member_is_live(self.member, self.process_group)
    }

    fn cleanup(&mut self) -> Result<()> {
        if !self.armed {
            return Ok(());
        }

        if self.member_is_live()? {
            match kill_process_group(self.process_group, Signal::KILL) {
                Ok(()) | Err(rustix::io::Errno::SRCH) => {}
                Err(error) => return Err(error.into()),
            }
        }

        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if !self.member_is_live()? {
                self.armed = false;
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!(
                    "fixture process group {} retained captured member {}",
                    self.process_group,
                    self.member.pid
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for CapturedProcessGroupGuard {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            let _write_result = writeln!(
                std::io::stderr().lock(),
                "Gateway fixture process-group cleanup failed: {error:#}"
            );
        }
    }
}

fn capture_process_group_member(
    pid: Pid,
    expected_process_group: Pid,
) -> Result<CapturedProcessGroupMember> {
    let process_group = getpgid(Some(pid))?;
    if process_group != expected_process_group {
        bail!("fixture member {pid} joined process group {process_group}");
    }
    let raw_pid = u32::try_from(pid.as_raw_pid())?;
    let start_identity = platform::inspect_process_start_identity(raw_pid)?
        .ok_or_else(|| anyhow::anyhow!("fixture member {pid} exited before identity capture"))?;

    Ok(CapturedProcessGroupMember {
        pid,
        start_identity,
    })
}

fn captured_process_group_member_is_live(
    member: CapturedProcessGroupMember,
    expected_process_group: Pid,
) -> Result<bool> {
    match getpgid(Some(member.pid)) {
        Ok(process_group) if process_group == expected_process_group => {}
        Ok(_) | Err(rustix::io::Errno::SRCH) => return Ok(false),
        Err(error) => return Err(error.into()),
    }
    let raw_pid = u32::try_from(member.pid.as_raw_pid())?;

    Ok(platform::inspect_process_start_identity(raw_pid)? == Some(member.start_identity))
}

struct GatewayRuntimeGuard {
    paths: PvPaths,
    runtimes: Vec<RecordedRuntime>,
    adopted: Vec<(RecordedRuntime, AdoptedProcess)>,
}

impl GatewayRuntimeGuard {
    fn standard(paths: PvPaths) -> Self {
        Self::new(
            paths,
            &[
                "8.3",
                "8.4",
                "8.5",
                "8.4+apcu",
                "8.4+redis",
                "8.4+xdebug",
                "8.4+redis+xdebug",
            ],
        )
    }

    fn new(paths: PvPaths, worker_runtime_keys: &[&str]) -> Self {
        let mut runtimes = worker_runtime_keys
            .iter()
            .map(|runtime_key| RecordedRuntime {
                pid_path: paths.worker_pid(runtime_key),
                metadata_path: paths.worker_runtime_metadata(runtime_key),
            })
            .collect::<Vec<_>>();
        runtimes.push(RecordedRuntime {
            pid_path: paths.gateway_pid(),
            metadata_path: paths.gateway_runtime_metadata(),
        });

        Self {
            paths,
            runtimes,
            adopted: Vec::new(),
        }
    }

    fn capture(&mut self, pid_path: Utf8PathBuf, metadata_path: Utf8PathBuf) -> Result<u32> {
        let process = ProcessSupervisor::new(self.paths.clone())
            .adopt_recorded(&pid_path, &metadata_path)?
            .ok_or_else(|| anyhow::anyhow!("runtime at {pid_path} was not adoptable"))?;
        let pid = process.pid();
        self.adopted.push((
            RecordedRuntime {
                pid_path,
                metadata_path,
            },
            process,
        ));
        Ok(pid)
    }

    fn capture_adopted(
        &mut self,
        pid_path: Utf8PathBuf,
        metadata_path: Utf8PathBuf,
        process: AdoptedProcess,
    ) {
        self.adopted.push((
            RecordedRuntime {
                pid_path,
                metadata_path,
            },
            process,
        ));
    }

    async fn cleanup(&mut self) -> Result<()> {
        let result = cleanup_gateway_runtimes(&self.paths, &self.adopted, &self.runtimes).await;
        if result.is_ok() {
            self.adopted.clear();
        }
        result
    }
}

impl Drop for GatewayRuntimeGuard {
    fn drop(&mut self) {
        let runtime_files_absent = self.runtimes.iter().all(|runtime| {
            matches!(fs::path_entry_exists(&runtime.pid_path), Ok(false))
                && matches!(fs::path_entry_exists(&runtime.metadata_path), Ok(false))
        });
        if runtime_files_absent && self.adopted.is_empty() {
            return;
        }

        let paths = &self.paths;
        let runtimes = &self.runtimes;
        let adopted = &self.adopted;
        let cleanup_result = std::thread::scope(|scope| {
            let cleanup_thread = std::thread::Builder::new().spawn_scoped(scope, || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| {
                        anyhow::anyhow!("cleanup runtime construction failed: {error}")
                    })?;

                runtime.block_on(cleanup_gateway_runtimes(paths, adopted, runtimes))
            });
            match cleanup_thread {
                Ok(cleanup_thread) => match cleanup_thread.join() {
                    Ok(result) => result.map_err(|error| format!("cleanup failed: {error:#}")),
                    Err(_panic) => Err("cleanup thread panicked".to_owned()),
                },
                Err(error) => Err(format!("cleanup thread construction failed: {error}")),
            }
        });
        if let Err(failure) = cleanup_result {
            let failure = gateway_cleanup_failure_with_emergency(
                failure,
                emergency_cleanup_gateway_runtimes(paths, adopted, runtimes),
            );
            report_gateway_cleanup_failure(paths, &failure);
        }
    }
}

fn gateway_cleanup_failure_with_emergency(primary: String, emergency: Result<()>) -> String {
    match emergency {
        Ok(()) => primary,
        Err(error) => format!("{primary}; emergency cleanup failed: {error:#}"),
    }
}

fn emergency_cleanup_gateway_runtimes(
    paths: &PvPaths,
    adopted: &[(RecordedRuntime, AdoptedProcess)],
    runtimes: &[RecordedRuntime],
) -> Result<()> {
    let mut failures = Vec::new();
    let mut cleaned_captured = Vec::new();
    for (record, process) in adopted {
        if let Err(error) = process.kill_and_wait_for_test(Duration::from_secs(1)) {
            failures.push(format!("{}: {error}", record.pid_path));
            continue;
        }
        cleaned_captured.push((record.clone(), process.pid()));
    }

    let supervisor = ProcessSupervisor::new(paths.clone());
    for runtime in runtimes {
        let publication_deadline = Instant::now() + Duration::from_millis(500);
        let captured_pid = cleaned_captured
            .iter()
            .find(|(record, _pid)| record.pid_path == runtime.pid_path)
            .map(|(_record, pid)| *pid);
        if let Err(error) = emergency_cleanup_recorded_gateway_runtime(
            paths,
            &supervisor,
            runtime,
            captured_pid,
            publication_deadline,
        ) {
            failures.push(format!("{}: {error}", runtime.pid_path));
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        bail!(failures.join("; "))
    }
}

fn emergency_cleanup_recorded_gateway_runtime(
    paths: &PvPaths,
    supervisor: &ProcessSupervisor,
    runtime: &RecordedRuntime,
    captured_pid: Option<u32>,
    publication_deadline: Instant,
) -> Result<()> {
    loop {
        match (
            fs::path_entry_exists(&runtime.pid_path)?,
            fs::path_entry_exists(&runtime.metadata_path)?,
        ) {
            (false, false) if Instant::now() >= publication_deadline => return Ok(()),
            (false, false) => {}
            (true, true) => {
                let pid_snapshot = fs::read_to_string(&runtime.pid_path)?;
                let metadata_snapshot = fs::read_to_string(&runtime.metadata_path)?;
                let captured_record = captured_pid.is_some_and(|pid| {
                    pid_snapshot
                        .trim()
                        .parse::<u32>()
                        .is_ok_and(|recorded_pid| recorded_pid == pid)
                });
                if !captured_record {
                    if !runtime_record_matches_expected_spec(
                        paths,
                        &runtime.pid_path,
                        &runtime.metadata_path,
                    )? {
                        bail!("runtime metadata does not match its registered runtime");
                    }
                    if let Some(process) =
                        supervisor.adopt_recorded(&runtime.pid_path, &runtime.metadata_path)?
                    {
                        process.kill_and_wait_for_test(Duration::from_secs(1))?;
                    } else if Instant::now() >= publication_deadline {
                        bail!("runtime was not adoptable through its recorded identity");
                    } else {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                }

                let records_unchanged = fs::read_to_string(&runtime.pid_path)
                    .is_ok_and(|contents| contents == pid_snapshot)
                    && fs::read_to_string(&runtime.metadata_path)
                        .is_ok_and(|contents| contents == metadata_snapshot);
                if records_unchanged {
                    fs::remove_file_if_exists(&runtime.pid_path)?;
                    fs::remove_file_if_exists(&runtime.metadata_path)?;
                } else if Instant::now() >= publication_deadline {
                    bail!("runtime records changed during emergency cleanup");
                }
            }
            _ if Instant::now() >= publication_deadline => {
                bail!("runtime has incomplete ownership records");
            }
            _ => {}
        }

        if Instant::now() < publication_deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn report_gateway_cleanup_failure(paths: &PvPaths, message: &str) {
    let record = json!({
        "level": "error",
        "target": "gateway_reconciliation",
        "event": "fixture_runtime_cleanup_failed",
        "message": message,
    });
    if let Ok(mut log) = fs::open_append_file(&paths.daemon_log()) {
        let _write_result = log.write_all(format!("{record}\n").as_bytes());
    }
    let _write_result = writeln!(std::io::stderr().lock(), "{record}");
}

async fn cleanup_recorded_runtimes(paths: &PvPaths, runtimes: &[RecordedRuntime]) -> Result<()> {
    let mut failures = Vec::new();
    for runtime in runtimes {
        if let Err(error) =
            stop_recorded_runtime(paths, &runtime.pid_path, &runtime.metadata_path).await
        {
            failures.push(format!("{}: {error}", runtime.pid_path));
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        bail!("{}", failures.join("; "))
    }
}

async fn cleanup_gateway_runtimes(
    paths: &PvPaths,
    adopted: &[(RecordedRuntime, AdoptedProcess)],
    runtimes: &[RecordedRuntime],
) -> Result<()> {
    let mut failures = Vec::new();
    for (record, process) in adopted {
        let pid = process.pid();
        if let Err(error) = process.clone().stop(Duration::from_secs(1)).await {
            failures.push(format!("{}: {error}", record.pid_path));
            continue;
        }

        let supervisor = ProcessSupervisor::new(paths.clone());
        match supervisor.adopt_recorded(&record.pid_path, &record.metadata_path) {
            Ok(Some(_replacement)) => {}
            Ok(None) => match pid_path_is_absent_or_matches(&record.pid_path, pid) {
                Ok(true) => {
                    if let Err(error) = fs::remove_file_if_exists(&record.pid_path) {
                        failures.push(format!("{}: {error}", record.pid_path));
                    }
                    if let Err(error) = fs::remove_file_if_exists(&record.metadata_path) {
                        failures.push(format!("{}: {error}", record.metadata_path));
                    }
                }
                Ok(false) => {}
                Err(error) => failures.push(format!("{}: {error}", record.pid_path)),
            },
            Err(error) => match pid_path_is_absent_or_matches(&record.pid_path, pid) {
                Ok(true) => {
                    if let Err(remove_error) = fs::remove_file_if_exists(&record.pid_path) {
                        failures.push(format!("{}: {remove_error}", record.pid_path));
                    }
                    if let Err(remove_error) = fs::remove_file_if_exists(&record.metadata_path) {
                        failures.push(format!("{}: {remove_error}", record.metadata_path));
                    }
                }
                Ok(false) => failures.push(format!("{}: {error}", record.pid_path)),
                Err(inspect_error) => failures.push(format!(
                    "{}: {error}; additionally failed to inspect captured pid: {inspect_error}",
                    record.pid_path
                )),
            },
        }
    }

    if let Err(error) = cleanup_recorded_runtimes(paths, runtimes).await {
        failures.push(error.to_string());
    }

    if failures.is_empty() {
        Ok(())
    } else {
        bail!("{}", failures.join("; "))
    }
}

fn pid_path_is_absent_or_matches(pid_path: &Utf8Path, expected_pid: u32) -> Result<bool> {
    match fs::read_to_string(pid_path) {
        Ok(contents) => Ok(contents.trim().parse::<u32>()? == expected_pid),
        Err(StateError::Filesystem { source, .. }) if source.kind() == ErrorKind::NotFound => {
            Ok(true)
        }
        Err(error) => Err(error.into()),
    }
}

async fn stop_runtime_from_pid_file(path: &Utf8Path) -> Result<()> {
    let metadata_path = path.with_extension("json");
    let pv_root = path
        .ancestors()
        .find(|ancestor| ancestor.file_name() == Some(".pv"))
        .ok_or_else(|| anyhow::anyhow!("runtime path is outside a PV home: {path}"))?;
    let home = pv_root
        .parent()
        .ok_or_else(|| anyhow::anyhow!("PV root has no home directory: {pv_root}"))?;
    stop_recorded_runtime(&PvPaths::for_home(home), path, &metadata_path).await
}

async fn stop_recorded_runtime(
    paths: &PvPaths,
    pid_path: &Utf8Path,
    metadata_path: &Utf8Path,
) -> Result<()> {
    let publication_deadline = Instant::now() + Duration::from_millis(500);
    loop {
        let pid_exists = fs::path_entry_exists(pid_path)?;
        let metadata_exists = fs::path_entry_exists(metadata_path)?;
        if !pid_exists && !metadata_exists {
            return Ok(());
        }
        if pid_exists && !metadata_exists && Instant::now() < publication_deadline {
            sleep(Duration::from_millis(10)).await;
            continue;
        }
        if pid_exists && !metadata_exists {
            let pid = fs::read_to_string(pid_path)?.trim().parse::<u32>()?;
            if !process_and_group_are_absent(pid, Duration::from_secs(1)).await? {
                bail!(
                    "runtime pid at {pid_path} was published without metadata while its process group is still alive"
                );
            }
            fs::remove_file_if_exists(pid_path)?;
            return Ok(());
        }
        if !runtime_record_matches_expected_spec(paths, pid_path, metadata_path)? {
            bail!("runtime metadata at {metadata_path} does not match its registered runtime");
        }
        if !pid_exists {
            let Some(pid) = runtime_metadata_pid(metadata_path)? else {
                bail!("runtime metadata at {metadata_path} has no recorded pid");
            };
            if !process_and_group_are_absent(pid, Duration::from_secs(1)).await? {
                bail!("runtime metadata at {metadata_path} still names a live process group");
            }
            fs::remove_file_if_exists(metadata_path)?;
            return Ok(());
        }

        let supervisor = ProcessSupervisor::new(paths.clone());
        let Some(runtime) = supervisor.adopt_recorded(pid_path, metadata_path)? else {
            let pid = fs::read_to_string(pid_path)?.trim().parse::<u32>()?;
            if runtime_metadata_pid(metadata_path)? != Some(pid) {
                bail!("runtime at {pid_path} had inconsistent ownership records");
            }
            if process_and_group_are_absent(pid, Duration::from_secs(1)).await? {
                fs::remove_file_if_exists(pid_path)?;
                fs::remove_file_if_exists(metadata_path)?;
                return Ok(());
            }

            return Err(DaemonError::RuntimeProcessIdentityChanged { pid }.into());
        };

        runtime.stop(Duration::from_secs(1)).await?;
        fs::remove_file_if_exists(pid_path)?;
        fs::remove_file_if_exists(metadata_path)?;

        if fs::path_entry_exists(pid_path)? || fs::path_entry_exists(metadata_path)? {
            bail!("runtime files remained after cleanup: {pid_path}, {metadata_path}");
        }

        return Ok(());
    }
}

fn runtime_record_matches_expected_spec(
    paths: &PvPaths,
    pid_path: &Utf8Path,
    metadata_path: &Utf8Path,
) -> Result<bool> {
    let metadata: Value = serde_json::from_str(&fs::read_to_string(metadata_path)?)?;
    let (resource_name, track, config_path, log_path) =
        if pid_path == paths.gateway_pid() && metadata_path == paths.gateway_runtime_metadata() {
            (
                "caddy",
                "2",
                paths.gateway_root_config(),
                paths.gateway_supervisor_log(),
            )
        } else {
            let workers_path = paths.run().join("workers");
            if pid_path.parent() != Some(workers_path.as_path()) {
                return Ok(false);
            }
            let Some(runtime_key) = pid_path
                .file_name()
                .and_then(|file_name| file_name.strip_prefix("php-"))
                .and_then(|file_name| file_name.strip_suffix(".pid"))
            else {
                return Ok(false);
            };
            if metadata_path != paths.worker_runtime_metadata(runtime_key) {
                return Ok(false);
            }
            (
                "frankenphp",
                runtime_key,
                paths.worker_root_config(runtime_key),
                paths.worker_log(runtime_key),
            )
        };
    let expected_arguments = json!([
        "run",
        "--config",
        config_path.as_str(),
        "--adapter",
        "caddyfile"
    ]);
    Ok(metadata["resource_name"] == resource_name
        && metadata["track"] == track
        && metadata["config_path"] == config_path.as_str()
        && metadata["log_path"] == log_path.as_str()
        && metadata["command"]
            .as_str()
            .is_some_and(|command| !command.is_empty())
        && metadata["arguments"] == expected_arguments)
}

async fn stop_recorded_runtime_preserving_files(
    paths: &PvPaths,
    pid_path: &Utf8Path,
    metadata_path: &Utf8Path,
) -> Result<()> {
    let supervisor = ProcessSupervisor::new(paths.clone());
    let runtime = supervisor
        .adopt_recorded(pid_path, metadata_path)?
        .ok_or_else(|| anyhow::anyhow!("runtime at {pid_path} was not adoptable"))?;
    runtime.stop(Duration::from_secs(1)).await?;

    Ok(())
}

async fn wait_for_process_exit(pid: u32) -> Result<()> {
    let raw_pid = i32::try_from(pid)?;
    let process =
        Pid::from_raw(raw_pid).ok_or_else(|| anyhow::anyhow!("invalid process id {pid}"))?;

    for _attempt in 0..50 {
        match test_kill_process(process) {
            Err(rustix::io::Errno::SRCH) => return Ok(()),
            Ok(()) => {}
            Err(error) => bail!("failed to inspect process {pid}: {error}"),
        }

        sleep(Duration::from_millis(20)).await;
    }

    Err(anyhow::anyhow!("process {process:?} was still running"))
}

fn metadata_pid(metadata: &serde_json::Value) -> Result<u32> {
    let pid = metadata["pid"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("runtime metadata is missing a numeric pid"))?;

    Ok(u32::try_from(pid)?)
}

fn runtime_metadata_pid(path: &Utf8Path) -> Result<Option<u32>> {
    let Ok(metadata) = fs::read_to_string(path) else {
        return Ok(None);
    };
    let metadata: serde_json::Value = serde_json::from_str(&metadata)?;

    metadata_pid(&metadata).map(Some)
}

fn required_runtime_metadata_pid(path: &Utf8Path) -> Result<u32> {
    runtime_metadata_pid(path)?
        .ok_or_else(|| anyhow::anyhow!("expected runtime metadata at {path}"))
}

fn process_is_alive(pid: u32) -> Result<bool> {
    let raw_pid = i32::try_from(pid)?;
    let process =
        Pid::from_raw(raw_pid).ok_or_else(|| anyhow::anyhow!("invalid process id {pid}"))?;

    match test_kill_process(process) {
        Ok(()) => Ok(true),
        Err(rustix::io::Errno::SRCH) => Ok(false),
        Err(error) => bail!("failed to inspect process {pid}: {error}"),
    }
}

fn process_group_is_alive(pid: u32) -> Result<bool> {
    let raw_pid = i32::try_from(pid)?;
    let process_group =
        Pid::from_raw(raw_pid).ok_or_else(|| anyhow::anyhow!("invalid process id {pid}"))?;

    match test_kill_process_group(process_group) {
        Ok(()) => Ok(true),
        Err(rustix::io::Errno::SRCH) => Ok(false),
        Err(error) => bail!("failed to inspect process group {pid}: {error}"),
    }
}

async fn process_and_group_are_absent(pid: u32, wait: Duration) -> Result<bool> {
    let deadline = Instant::now() + wait;
    loop {
        if !process_is_alive(pid)? && !process_group_is_alive(pid)? {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        sleep(Duration::from_millis(10)).await;
    }
}

fn assert_process_group_absent(pid: u32) -> Result<()> {
    let raw_pid = i32::try_from(pid)?;
    let process_group =
        Pid::from_raw(raw_pid).ok_or_else(|| anyhow::anyhow!("invalid process id {pid}"))?;
    match test_kill_process_group(process_group) {
        Err(rustix::io::Errno::SRCH) => Ok(()),
        Ok(()) => bail!("process group {pid} remained after cleanup"),
        Err(error) => bail!("failed to inspect process group {pid}: {error}"),
    }
}

fn replace_runtime_metadata_identity(
    path: &Utf8Path,
    resource_name: &str,
    track: &str,
) -> Result<()> {
    let metadata = fs::read_to_string(path)?;
    let mut metadata: serde_json::Value = serde_json::from_str(&metadata)?;
    let Some(object) = metadata.as_object_mut() else {
        anyhow::bail!("runtime metadata must be a JSON object");
    };
    object.insert(
        "resource_name".to_owned(),
        serde_json::Value::String(resource_name.to_owned()),
    );
    object.insert(
        "track".to_owned(),
        serde_json::Value::String(track.to_owned()),
    );
    let metadata = serde_json::to_string(&metadata)?;
    fs::write_sensitive_file(path, &metadata)?;

    Ok(())
}

fn seed_php_manifest(paths: &PvPaths, default_track: &str) -> Result<()> {
    fs::write_sensitive_file(
        &paths.downloads().join("manifest.json"),
        &json!({
            "schema_version": 1,
            "minimum_pv_version": "0.1.0",
            "resources": [
                {
                    "name": "php",
                    "default_track": default_track,
                    "tracks": [
                        {
                            "name": "8.3",
                            "artifacts": [
                                {
                                    "artifact_version": "8.3.21-pv1",
                                    "upstream_version": "8.3.21",
                                    "pv_build_revision": "pv1",
                                    "platform": "darwin-arm64",
                                    "url": "https://artifacts.example.test/php-8.3.21-pv1-darwin-arm64.tar.gz",
                                    "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                                    "size": 12345,
                                    "published_at": "2026-05-26T14:30:00Z"
                                }
                            ]
                        },
                        {
                            "name": "8.4",
                            "artifacts": [
                                {
                                    "artifact_version": "8.4.8-pv1",
                                    "upstream_version": "8.4.8",
                                    "pv_build_revision": "pv1",
                                    "platform": "darwin-arm64",
                                    "url": "https://artifacts.example.test/php-8.4.8-pv1-darwin-arm64.tar.gz",
                                    "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                                    "size": 12345,
                                    "published_at": "2026-05-27T14:30:00Z"
                                }
                            ]
                        }
                    ]
                }
            ]
        })
        .to_string(),
    )?;

    Ok(())
}

fn assert_runtime_plan_snapshot(name: &str, plan: daemon::gateway::RuntimePlan) {
    let mut settings = Settings::clone_current();
    settings.add_filter(r#"/[^"]*/\.tmp[A-Za-z0-9._-]+"#, "<tempdir>");
    settings.add_filter(r#"id: "[a-z0-9]{10}""#, r#"id: "<project_id>""#);
    settings.add_filter(r"port: \d+", "port: <port>");
    settings.bind(|| {
        assert_debug_snapshot!(name, plan);
    });
}

fn assert_runtime_states_snapshot(
    name: &str,
    snapshot: Vec<state::RuntimeObservedStateRecord>,
) -> Result<()> {
    let mut settings = Settings::clone_current();
    settings.add_filter(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", "<timestamp>");
    settings.bind(|| {
        assert_debug_snapshot!(name, snapshot);
        Ok::<(), anyhow::Error>(())
    })
}

fn assert_process_spec_snapshot(
    tempdir: &Utf8Path,
    snapshot: (
        Vec<String>,
        Vec<String>,
        daemon::ProcessSpec,
        daemon::ProcessSpec,
    ),
) {
    let mut settings = Settings::clone_current();
    settings.add_filter(tempdir.as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!("caddy_cli_command_and_process_specs_are_stable", snapshot);
    });
}

#[tokio::test]
async fn resource_only_target_recovers_alive_unready_gateway_with_invalid_config() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let project_root = create_project_with_config(tempdir.path(), "acme", "serve: false\n")?;
    let caddy_release = tempdir.path().join("fake-caddy-release");
    write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;
    let mut database = Database::open(&paths)?;
    let project = database
        .link_project_with_mode(
            LinkProjectInput {
                path: project_root.clone(),
                original_path: project_root.clone(),
                primary_hostname: "ignored.test".to_owned(),
                config_path: project_root.join("pv.yml"),
                desired_php_track: None,
                additional_hostnames: Vec::new(),
            },
            ProjectMode::ResourceOnly,
        )?
        .project;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    let ports = available_loopback_ports(2)?;
    seed_runtime_ports(&paths, &mut database, ports[0], ports[1], &[])?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let initial_gateway_pid = required_runtime_metadata_pid(&paths.gateway_runtime_metadata())?;
    write_fake_admin_control(&paths.gateway_root_config(), json!({"stop_service": true}))?;
    timeout(Duration::from_secs(5), async {
        loop {
            if TcpStream::connect(("127.0.0.1", ports[0])).await.is_err() {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .context("Gateway listener stayed up")?;
    write_fake_admin_control(&paths.gateway_root_config(), json!({}))?;
    write_test_bytes(&paths.gateway_root_config(), &[0xff])?;
    let invalid_fragment = paths
        .gateway_projects_config_dir()
        .join("invalid.Caddyfile");
    write_test_bytes(&invalid_fragment, &[0xff])?;

    reconcile_project_gateway_runtimes_for_test(
        &paths,
        &project.id,
        Duration::from_secs(5),
        GatewayPfRoutingState::Inactive,
    )
    .await?;
    let recovered_gateway_pid = required_runtime_metadata_pid(&paths.gateway_runtime_metadata())?;
    let gateway_status = Database::open(&paths)?
        .runtime_observed_states()?
        .into_iter()
        .find(|state| state.subject == RuntimeSubject::Gateway)
        .map(|state| state.status);

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;

    assert_ne!(recovered_gateway_pid, initial_gateway_pid);
    assert!(fs::read_to_string(&paths.gateway_root_config())?.contains("PV Gateway is running"));
    assert!(!invalid_fragment.exists());
    assert_eq!(gateway_status, Some(RuntimeObservedStatus::Degraded));

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn targeted_inspection_promotes_only_recoverable_uncertainty() -> Result<()> {
    // Allowed: an unprovable active Gateway state promotes once, and System
    // reconciliation repairs it by starting the Gateway.
    {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
        let project_root = create_project_with_config(tempdir.path(), "acme", "php: \"8.4\"\n")?;
        let caddy_release = tempdir.path().join("fake-caddy-release");
        let frankenphp_release = tempdir.path().join("fake-frankenphp-release");
        write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;
        write_stateful_fake_frankenphp(&frankenphp_release.join("bin/frankenphp"))?;
        let mut database = Database::open(&paths)?;
        let project = database
            .link_project(LinkProjectInput {
                path: project_root.clone(),
                original_path: project_root.clone(),
                primary_hostname: "acme.test".to_owned(),
                config_path: project_root.join("pv.yml"),
                desired_php_track: Some("8.4".to_owned()),
                additional_hostnames: Vec::new(),
            })?
            .project;
        database.record_managed_resource_track_installed(
            "caddy",
            "2",
            "fake-caddy-pv1",
            &caddy_release,
        )?;
        database.record_managed_resource_track_installed(
            "frankenphp",
            "8.4",
            "fake-frankenphp-pv1",
            &frankenphp_release,
        )?;
        let ports = available_loopback_ports(3)?;
        seed_runtime_ports(
            &paths,
            &mut database,
            ports[0],
            ports[1],
            &[("8.4", ports[2])],
        )?;
        drop(database);

        let summary = reconcile_project_gateway_runtimes_for_test(
            &paths,
            &project.id,
            Duration::from_secs(5),
            GatewayPfRoutingState::Inactive,
        )
        .await?;
        assert_eq!(summary, GATEWAY_RECONCILIATION_SUMMARY);
        assert!(paths.gateway_pid().exists());

        stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
        stop_runtime_from_pid_file(&paths.worker_pid("8.4")).await?;
        runtime_guard.cleanup().await?;
    }

    // Denied: invalid target config and a deleted target keep their original
    // typed errors and trigger no System work.
    {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
        let project_root = create_project_with_config(tempdir.path(), "acme", "php: \"8.4\"\n")?;
        let caddy_release = tempdir.path().join("fake-caddy-release");
        let frankenphp_release = tempdir.path().join("fake-frankenphp-release");
        write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;
        write_stateful_fake_frankenphp(&frankenphp_release.join("bin/frankenphp"))?;
        let mut database = Database::open(&paths)?;
        let project = database
            .link_project(LinkProjectInput {
                path: project_root.clone(),
                original_path: project_root.clone(),
                primary_hostname: "acme.test".to_owned(),
                config_path: project_root.join("pv.yml"),
                desired_php_track: Some("8.4".to_owned()),
                additional_hostnames: Vec::new(),
            })?
            .project;
        database.record_managed_resource_track_installed(
            "caddy",
            "2",
            "fake-caddy-pv1",
            &caddy_release,
        )?;
        database.record_managed_resource_track_installed(
            "frankenphp",
            "8.4",
            "fake-frankenphp-pv1",
            &frankenphp_release,
        )?;
        let ports = available_loopback_ports(3)?;
        seed_runtime_ports(
            &paths,
            &mut database,
            ports[0],
            ports[1],
            &[("8.4", ports[2])],
        )?;
        drop(database);
        reconcile_gateway_runtimes(&paths).await?;
        let gateway_pid = required_runtime_metadata_pid(&paths.gateway_runtime_metadata())?;
        let gateway_loads = fake_admin_load_bodies(&paths.gateway_root_config())?.len();
        let worker_loads = fake_admin_load_bodies(&paths.worker_root_config("8.4"))?.len();

        fs::write_sensitive_file(&project_root.join("pv.yml"), "php: [\n")?;
        let invalid = reconcile_project_gateway_runtimes_for_test(
            &paths,
            &project.id,
            Duration::from_secs(5),
            GatewayPfRoutingState::Inactive,
        )
        .await;
        assert!(
            matches!(invalid, Err(DaemonError::Config(_))),
            "expected the original config error, got {invalid:?}"
        );
        assert_eq!(
            fake_admin_load_bodies(&paths.gateway_root_config())?.len(),
            gateway_loads
        );
        assert_eq!(
            fake_admin_load_bodies(&paths.worker_root_config("8.4"))?.len(),
            worker_loads
        );

        let mut database = Database::open(&paths)?;
        database.unlink_project(&project.id)?;
        drop(database);
        let deleted = reconcile_project_gateway_runtimes_for_test(
            &paths,
            &project.id,
            Duration::from_secs(5),
            GatewayPfRoutingState::Inactive,
        )
        .await;
        assert!(
            matches!(
                deleted,
                Err(DaemonError::State(StateError::ProjectNotFound { .. }))
            ),
            "expected the original not-found error, got {deleted:?}"
        );
        assert_eq!(
            fake_admin_load_bodies(&paths.gateway_root_config())?.len(),
            gateway_loads
        );
        assert_eq!(
            fake_admin_load_bodies(&paths.worker_root_config("8.4"))?.len(),
            worker_loads
        );
        assert_eq!(
            required_runtime_metadata_pid(&paths.gateway_runtime_metadata())?,
            gateway_pid
        );

        stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
        stop_runtime_from_pid_file(&paths.worker_pid("8.4")).await?;
        runtime_guard.cleanup().await?;
    }

    Ok(())
}

#[tokio::test]
async fn gateway_failure_preserves_primary_error_when_observation_write_fails() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let project_root = create_project_with_config(tempdir.path(), "acme", "php: \"8.4\"\n")?;
    let caddy_release = tempdir.path().join("fake-caddy-release");
    let frankenphp_release = tempdir.path().join("fake-frankenphp-release");
    write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;
    write_stateful_fake_frankenphp(&frankenphp_release.join("bin/frankenphp"))?;
    let mut database = Database::open(&paths)?;
    let project = database
        .link_project(LinkProjectInput {
            path: project_root.clone(),
            original_path: project_root.clone(),
            primary_hostname: "acme.test".to_owned(),
            config_path: project_root.join("pv.yml"),
            desired_php_track: Some("8.4".to_owned()),
            additional_hostnames: Vec::new(),
        })?
        .project;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-frankenphp-pv1",
        &frankenphp_release,
    )?;
    let ports = available_loopback_ports(3)?;
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[("8.4", ports[2])],
    )?;
    drop(database);
    reconcile_gateway_runtimes(&paths).await?;

    Connection::open(paths.db().as_std_path())?.execute_batch(
        "CREATE TRIGGER reject_gateway_observation BEFORE INSERT ON observed_states
         WHEN NEW.subject_kind = 'runtime' AND NEW.subject_id = 'gateway'
         BEGIN SELECT RAISE(FAIL, 'fixture rejected Gateway observation'); END;",
    )?;
    fs::write_sensitive_file(&project_root.join("pv.yml"), "php: [\n")?;
    let result = reconcile_project_gateway_runtimes_for_test(
        &paths,
        &project.id,
        Duration::from_secs(5),
        GatewayPfRoutingState::Inactive,
    )
    .await;

    let Err(DaemonError::RuntimeCleanupFailed {
        runtime,
        source,
        cleanup,
    }) = result
    else {
        bail!("expected the primary and recording failures aggregated, got {result:?}");
    };
    assert_eq!(runtime, "gateway");
    assert!(
        matches!(source.as_ref(), DaemonError::Config(_)),
        "expected the primary config error, got {source:?}"
    );
    assert!(
        matches!(cleanup.as_ref(), DaemonError::State(StateError::Sqlite(_))),
        "expected the recording SQLite failure, got {cleanup:?}"
    );

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    stop_runtime_from_pid_file(&paths.worker_pid("8.4")).await?;

    runtime_guard.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn targeted_reconciliation_reports_alive_unready_worker() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut runtime_guard = GatewayRuntimeGuard::standard(paths.clone());
    let project_root = create_project_with_config(tempdir.path(), "acme", "php: \"8.4\"\n")?;
    let caddy_release = tempdir.path().join("fake-caddy-release");
    let frankenphp_release = tempdir.path().join("fake-frankenphp-release");
    write_stateful_fake_caddy(&caddy_release.join("bin/caddy"))?;
    write_stateful_fake_frankenphp(&frankenphp_release.join("bin/frankenphp"))?;
    let mut database = Database::open(&paths)?;
    let project = database
        .link_project(LinkProjectInput {
            path: project_root.clone(),
            original_path: project_root.clone(),
            primary_hostname: "acme.test".to_owned(),
            config_path: project_root.join("pv.yml"),
            desired_php_track: Some("8.4".to_owned()),
            additional_hostnames: Vec::new(),
        })?
        .project;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &caddy_release,
    )?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-frankenphp-pv1",
        &frankenphp_release,
    )?;
    let ports = available_loopback_ports(3)?;
    seed_runtime_ports(
        &paths,
        &mut database,
        ports[0],
        ports[1],
        &[("8.4", ports[2])],
    )?;
    drop(database);
    reconcile_gateway_runtimes(&paths).await?;
    let plan = build_runtime_plan(&paths)?;
    let worker = plan.workers.first().context("expected worker")?;
    let command = CaddyCliCommand::frankenphp(frankenphp_release.join("bin/frankenphp"));
    let spec = worker_process_spec(&paths, worker, &command, &frankenphp_release)?;
    let supervisor = ProcessSupervisor::new(paths.clone());
    let original_pid = required_runtime_metadata_pid(&paths.worker_runtime_metadata("8.4"))?;
    write_fake_admin_control(
        &paths.worker_root_config("8.4"),
        json!({"stop_service": true}),
    )?;
    timeout(Duration::from_secs(5), async {
        loop {
            if TcpStream::connect(("127.0.0.1", ports[2])).await.is_err() {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .context("worker listener stayed up")?;
    assert!(supervisor.verify_ownership(&spec)?.is_some());
    let result = reconcile_project_gateway_runtimes_for_test(
        &paths,
        &project.id,
        Duration::from_millis(250),
        GatewayPfRoutingState::Inactive,
    )
    .await;
    let still_owned = supervisor.verify_ownership(&spec)?.is_some();
    let still_unready = TcpStream::connect(("127.0.0.1", ports[2])).await.is_err();
    let final_pid = required_runtime_metadata_pid(&paths.worker_runtime_metadata("8.4"))?;
    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    stop_runtime_from_pid_file(&paths.worker_pid("8.4")).await?;
    let worker_status = Database::open(&paths)?
        .runtime_observed_states()?
        .into_iter()
        .find(|record| matches!(&record.subject, RuntimeSubject::PhpWorker { php_track } if php_track == "8.4"))
        .map(|record| record.status);
    let Err(DaemonError::CaddyAdmin(CaddyAdminError::RestoredConfigReloadFailed {
        original_error,
        restored_error,
    })) = result
    else {
        bail!("expected readiness and rollback failure, got {result:?}");
    };
    assert!(matches!(
        original_error.as_ref(),
        CaddyAdminError::TaskFailed {
            operation: CaddyAdminOperation::Readiness,
            ..
        }
    ));
    assert!(matches!(
        restored_error.as_ref(),
        CaddyAdminError::TaskFailed {
            operation: CaddyAdminOperation::Rollback,
            ..
        }
    ));
    assert_debug_snapshot!((still_owned, still_unready, original_pid == final_pid, worker_status), @r"
    (
        true,
        true,
        true,
        Some(
            Failed,
        ),
    )
    ");
    runtime_guard.cleanup().await?;

    Ok(())
}
