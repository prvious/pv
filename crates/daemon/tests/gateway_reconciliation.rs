use anyhow::{Error, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use daemon::gateway::{
    CaddyCliCommand, GatewayPfRoutingState, build_runtime_plan, gateway_process_spec,
    promote_validated_config_for_test, reconcile_gateway_runtimes_with_pf_state_for_test,
    validate_config, worker_process_spec,
};
use daemon::{CaddyAdminError, CaddyAdminOperation, DaemonError, ProcessSupervisor};
use insta::{Settings, assert_debug_snapshot};
use rcgen::generate_simple_self_signed;
use rustix::process::{Pid, Signal, kill_process_group, test_kill_process};
use serde_json::{Value, json};
use state::{
    Database, GatewayPort, LinkProjectInput, PortOwner, PortRequest, ProjectMode, PvPaths,
    RUNTIME_PORT_FALLBACK_END, RUNTIME_PORT_FALLBACK_START, fs,
};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::ErrorKind;
use std::net::TcpListener;
use std::process::Output;
use std::time::{Duration, Instant};
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

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_does_not_fallback_to_another_caddy_track() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_rejects_invalid_caddy_two_without_fallback() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_rolls_back_after_fresh_admin_startup_failure() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_rolls_back_after_fresh_service_readiness_failure() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_starts_gateway_without_linked_projects() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

    Ok(())
}

#[tokio::test]
async fn fresh_gateway_ownership_probe_failure_cleans_runtime_before_rollback() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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
                    fs::write_sensitive_file(&paths.gateway_runtime_metadata(), "{")?;
                    fs::write_sensitive_file(&admin_response_gate, "")?;
                    return Ok::<u32, Error>(gateway_pid);
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_error| anyhow::anyhow!("fresh Gateway did not reach gated admin readiness"))?
    };
    let (result, gateway_pid) = tokio::join!(
        reconcile_gateway_runtimes_with_pf_state_for_test(
            &paths,
            Duration::from_secs(5),
            GatewayPfRoutingState::Unknown,
        ),
        corrupt_runtime_metadata,
    );
    let gateway_pid = gateway_pid?;
    let gateway_was_alive = process_is_alive(gateway_pid)?;
    if gateway_was_alive {
        stop_runtime_pid(gateway_pid).await?;
    }

    assert!(matches!(result, Err(DaemonError::Json(_))));
    assert!(!gateway_was_alive);
    assert!(!paths.gateway_pid().exists());
    assert!(!paths.gateway_runtime_metadata().exists());
    assert_eq!(
        fs::read_to_string(&paths.gateway_root_config())?,
        previous_root
    );

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_preserves_running_runtimes_after_env_only_edit() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

    reconcile_gateway_runtimes(&paths).await?;
    let third_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected gateway runtime metadata"))?;
    let third_worker_pid = runtime_metadata_pid(&paths.worker_runtime_metadata("8.4"))?
        .ok_or_else(|| anyhow::anyhow!("expected worker runtime metadata"))?;

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
    assert_eq!(
        fs::read_to_string(&paths.gateway_root_config())?,
        edited_gateway_root
    );
    assert_eq!(
        fs::read_to_string(&paths.worker_root_config("8.4"))?,
        edited_worker_root
    );
    assert_eq!(
        fs::read_to_string(&gateway_fragment_path)?,
        edited_gateway_fragment
    );
    assert_eq!(
        fs::read_to_string(&worker_fragment_path)?,
        edited_worker_fragment
    );
    assert!(gateway_metadata["applied_config_fingerprint"].is_string());
    assert!(worker_metadata["applied_config_fingerprint"].is_string());
    assert_ne!(gateway_metadata["replacement_required"], true);
    assert_ne!(worker_metadata["replacement_required"], true);

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_takes_full_path_when_applied_fingerprint_is_missing() -> Result<()>
{
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_keeps_pending_state_when_applied_fingerprint_commit_fails()
-> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_replaces_legacy_runtime_identities_once() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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
    let first_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected gateway runtime metadata"))?;
    let first_worker_pid = runtime_metadata_pid(&paths.worker_runtime_metadata("8.4"))?
        .ok_or_else(|| anyhow::anyhow!("expected worker runtime metadata"))?;
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
    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    stop_runtime_from_pid_file(&paths.worker_pid("8.4")).await?;

    assert_ne!(second_gateway_pid, first_gateway_pid);
    assert_ne!(second_worker_pid, first_worker_pid);
    assert_eq!(stable_gateway_pid, second_gateway_pid);
    assert_eq!(stable_worker_pid, second_worker_pid);

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_replaces_legacy_admin_off_process_before_admin_contact()
-> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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
    let legacy_process = supervisor.start(legacy_spec).await?;
    let legacy_pid = legacy_process.pid();
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
    legacy_process.stop(Duration::from_secs(1)).await?;
    wait_for_process_exit(legacy_pid).await?;

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

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_replaces_dead_gateway_process() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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
    stop_runtime_pid(first_gateway_pid).await?;

    reconcile_gateway_runtimes(&paths).await?;
    let second_metadata = state::testing::read_to_string(&paths.gateway_runtime_metadata())?;

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    stop_runtime_from_pid_file(&paths.worker_pid("8.4")).await?;

    assert_ne!(first_metadata, second_metadata);

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_rejects_unverified_live_gateway_listener() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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
    fs::delete_file(&paths.gateway_runtime_metadata())?;

    let result = reconcile_gateway_runtimes(&paths).await;

    stop_runtime_pid(first_gateway_pid).await?;
    stop_runtime_from_pid_file(&paths.worker_pid("8.4")).await?;

    assert!(matches!(
        result,
        Err(DaemonError::UnexpectedProtocolResponse { reason })
            if reason.contains("is listening but no PV-owned process could be verified")
    ));

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_bounds_foreign_https_listener_probe() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

#[tokio::test]
async fn gateway_reconciliation_stops_worker_when_no_projects_remain_on_track() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project_root = tempdir.path().join("acme");
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
        "fake-frankenphp-pv1",
        &release_path,
    )?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.3",
        "fake-frankenphp-83-pv1",
        &gateway_release_path,
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
    let worker_metadata = state::testing::read_to_string(&paths.worker_runtime_metadata("8.4"))?;
    let worker_metadata_json: serde_json::Value = serde_json::from_str(&worker_metadata)?;
    let worker_pid = metadata_pid(&worker_metadata_json)?;
    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;

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

    wait_for_process_exit(worker_pid).await?;
    assert!(!paths.worker_pid("8.4").exists());
    assert!(!paths.worker_runtime_metadata("8.4").exists());
    assert!(!paths.worker_root_config("8.4").exists());

    let database = Database::open(&paths)?;
    let assigned_ports = database.assigned_ports()?;
    assert!(!assigned_ports.iter().any(|port| matches!(
        &port.owner,
        PortOwner::PhpWorker { php_runtime_key } if php_runtime_key == "8.4"
    )));

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_preserves_project_fragments_for_invalid_project_config()
-> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_skips_invalid_project_without_preserved_fragments() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_uses_persisted_track_after_config_becomes_invalid() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

    Ok(())
}

#[test]
fn gateway_runtime_plan_skips_invalid_config_fallback_when_persisted_loaded_extension_metadata_is_missing()
-> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

    let plan = build_runtime_plan(&paths)?;
    let database = Database::open(&paths)?;
    let observed = database
        .project_env_observed_state(&project.project.id)?
        .ok_or_else(|| anyhow::anyhow!("expected Project env observed failure"))?;

    assert!(plan.workers.is_empty());
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

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_preserves_fragments_for_parseable_invalid_project_config()
-> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_preserves_active_fragments_when_validation_fails() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

    write_failing_frankenphp_validator(&fake_frankenphp)?;
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

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_loads_exact_gateway_and_worker_roots_without_restarting()
-> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

    Ok(())
}

#[tokio::test]
async fn worker_reconciliation_restores_previous_service_port_after_readiness_failure() -> Result<()>
{
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    stop_runtime_from_pid_file(&paths.worker_pid("8.4")).await?;

    assert!(matches!(result, Err(DaemonError::ReadinessTimedOut { .. })));
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

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_rejection_keeps_old_runtime_and_disk_state() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;

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
    assert!(metadata["applied_config_fingerprint"].is_null());
    assert_eq!(load_bodies.len(), 1);
    assert_eq!(
        requests
            .iter()
            .filter(|request| request["method"] == "POST" && request["path"] == "/load")
            .map(|request| request["status"].as_u64())
            .collect::<Vec<_>>(),
        vec![Some(422)]
    );

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_rejects_load_error_reported_with_success_status() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_replaces_runtime_before_newer_desired_state() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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
        Duration::from_millis(250),
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
    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_does_not_reload_after_runtime_exits_after_load() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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
    if let Some(pid) = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        && process_is_alive(pid)?
    {
        stop_runtime_pid(pid).await?;
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

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_refuses_to_load_tampered_rollback_fragment_backup() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_reports_compound_restored_readiness_failure() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_rolls_back_config_when_runtime_readiness_fails() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

    let mut database = Database::open(&paths)?;
    database.release_port(PortOwner::Gateway(GatewayPort::Http))?;
    database.release_port(PortOwner::Gateway(GatewayPort::Https))?;
    seed_runtime_ports(&paths, &mut database, new_http_port, new_https_port, &[])?;
    drop(database);

    let result =
        reconcile_gateway_runtimes_with_readiness_timeout(&paths, Duration::from_millis(100)).await;
    let root_config = fs::read_to_string(&paths.gateway_root_config())?;
    let load_bodies = fake_admin_load_bodies(&paths.gateway_root_config())?;
    let requests = fake_admin_requests(&paths.gateway_root_config())?;
    let restored_metadata: Value =
        serde_json::from_str(&fs::read_to_string(&paths.gateway_runtime_metadata())?)?;
    let first_gateway_is_alive = process_is_alive(first_gateway_pid)?;
    if first_gateway_is_alive {
        stop_runtime_pid(first_gateway_pid).await?;
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

    Ok(())
}

fn assert_worker_command(paths: &PvPaths, php_track: &str, expected: &Utf8Path) -> Result<()> {
    let metadata = fs::read_to_string(&paths.worker_runtime_metadata(php_track))?;
    let metadata: serde_json::Value = serde_json::from_str(&metadata)?;

    assert_eq!(metadata["command"], expected.as_str());

    Ok(())
}

#[test]
fn runtime_plan_groups_linked_projects_by_php_track() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

    Ok(())
}

#[test]
fn runtime_plan_excludes_resource_only_project_with_explicit_php() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

    Ok(())
}

#[test]
fn runtime_plan_preserves_served_project_while_resource_only_transition_is_pending() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

    Ok(())
}

#[test]
fn gateway_runtime_plan_groups_projects_by_php_track_and_extensions() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

    Ok(())
}

#[test]
fn runtime_plan_resolves_latest_php_track_from_cached_manifest() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

    Ok(())
}

#[test]
fn runtime_plan_defaults_document_root_to_public_directory_without_config() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

    Ok(())
}

#[test]
fn runtime_plan_defaults_document_root_to_project_root_without_public_directory() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

    Ok(())
}

#[test]
fn runtime_plan_uses_project_root_not_original_or_config_path() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

    Ok(())
}

#[test]
fn gateway_config_validation_failure_preserves_active_config_and_cleans_candidate() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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

#[test]
fn caddy_cli_command_and_process_specs_are_stable() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
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
    let private_environment = gateway_process_spec(&paths, &command).private_environment;

    validate_config(&command, &config_path, &private_environment).await?;

    assert_eq!(state::testing::read_to_string(&observed_phprc)?, "");
    assert_eq!(state::testing::read_to_string(&observed_scan_dir)?, "");

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

fn write_failing_frankenphp_validator(path: &Utf8Path) -> Result<()> {
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
    reason = "test fixture marks fake FrankenPHP validator executable"
)]
fn set_executable(path: &Utf8Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(path, permissions)?;

    Ok(())
}

async fn stop_runtime_from_pid_file(path: &Utf8Path) -> Result<()> {
    let pid = state::testing::read_to_string(path)?
        .trim()
        .parse::<u32>()?;

    stop_runtime_pid(pid).await
}

async fn stop_runtime_pid(pid: u32) -> Result<()> {
    let raw_pid = i32::try_from(pid)?;
    let process_group =
        Pid::from_raw(raw_pid).ok_or_else(|| anyhow::anyhow!("invalid process id {pid}"))?;

    let _term_result = kill_process_group(process_group, Signal::TERM);
    for _attempt in 0..50 {
        if test_kill_process(process_group).is_err() {
            return Ok(());
        }

        sleep(Duration::from_millis(20)).await;
    }

    kill_process_group(process_group, Signal::KILL)?;

    for _attempt in 0..50 {
        if test_kill_process(process_group).is_err() {
            return Ok(());
        }

        sleep(Duration::from_millis(20)).await;
    }

    Err(anyhow::anyhow!(
        "process {process_group:?} was still running"
    ))
}

async fn wait_for_process_exit(pid: u32) -> Result<()> {
    let raw_pid = i32::try_from(pid)?;
    let process =
        Pid::from_raw(raw_pid).ok_or_else(|| anyhow::anyhow!("invalid process id {pid}"))?;

    for _attempt in 0..50 {
        if test_kill_process(process).is_err() {
            return Ok(());
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

fn process_is_alive(pid: u32) -> Result<bool> {
    let raw_pid = i32::try_from(pid)?;
    let process =
        Pid::from_raw(raw_pid).ok_or_else(|| anyhow::anyhow!("invalid process id {pid}"))?;

    Ok(test_kill_process(process).is_ok())
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
