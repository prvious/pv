use anyhow::Result;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use daemon::DaemonError;
use daemon::gateway::{
    FrankenphpCommand, build_runtime_plan, gateway_process_spec, promote_validated_config_for_test,
    reconcile_gateway_runtimes, reconcile_gateway_runtimes_with_readiness_timeout, validate_config,
    worker_process_spec,
};
use insta::{Settings, assert_debug_snapshot};
use rcgen::generate_simple_self_signed;
use rustix::process::{Pid, Signal, kill_process_group, test_kill_process};
use serde_json::json;
use state::{
    Database, GatewayPort, LinkProjectInput, PortOwner, PortRequest, PvPaths,
    RUNTIME_PORT_FALLBACK_END, RUNTIME_PORT_FALLBACK_START, fs,
};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::net::TcpListener;
use std::process::Output;
use std::time::Duration;
use tokio::time::{sleep, timeout};

const GATEWAY_RECONCILIATION_SUMMARY: &str = "Gateway runtime reconciled";

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
async fn gateway_reconciliation_preserves_running_runtimes_on_second_reconcile() -> Result<()> {
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

    reconcile_gateway_runtimes(&paths).await?;
    let second_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected gateway runtime metadata"))?;
    let second_worker_pid = runtime_metadata_pid(&paths.worker_runtime_metadata("8.4"))?
        .ok_or_else(|| anyhow::anyhow!("expected worker runtime metadata"))?;

    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    stop_runtime_from_pid_file(&paths.worker_pid("8.4")).await?;

    assert_eq!(second_gateway_pid, first_gateway_pid);
    assert_eq!(second_worker_pid, first_worker_pid);

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_restarts_recorded_runtime_with_legacy_metadata() -> Result<()> {
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
    remove_private_environment_fingerprint(&paths.gateway_runtime_metadata())?;

    reconcile_gateway_runtimes(&paths).await?;
    let second_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected gateway runtime metadata"))?;

    wait_for_process_exit(first_gateway_pid).await?;
    stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    stop_runtime_from_pid_file(&paths.worker_pid("8.4")).await?;

    assert_ne!(second_gateway_pid, first_gateway_pid);

    Ok(())
}

#[tokio::test]
async fn gateway_reconciliation_restarts_when_reload_is_unavailable() -> Result<()> {
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
        sleep(Duration::from_secs(5)).await;

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
        Duration::from_secs(2),
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
        &FrankenphpCommand::new(&validator),
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
    assert!(!database.assigned_ports()?.iter().any(|port| matches!(
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
    let gateway_fragment = fs::read_to_string(
        &paths
            .gateway_projects_config_dir()
            .join(format!("{}.Caddyfile", project.project.id)),
    )?;
    let worker_fragment = fs::read_to_string(
        &paths
            .worker_projects_config_dir("8.4")
            .join(format!("{}.Caddyfile", project.project.id)),
    )?;

    fs::write_sensitive_file(&project_root.join("pv.yml"), "php: [\n")?;

    reconcile_gateway_runtimes(&paths).await?;
    let database = Database::open(&paths)?;
    let observed = database
        .project_env_observed_state(&project.project.id)?
        .ok_or_else(|| anyhow::anyhow!("expected Project env observed failure"))?;
    let gateway_root_config = fs::read_to_string(&paths.gateway_root_config())?;

    assert_eq!(
        fs::read_to_string(
            &paths
                .gateway_projects_config_dir()
                .join(format!("{}.Caddyfile", project.project.id)),
        )?,
        gateway_fragment
    );
    assert_eq!(
        fs::read_to_string(
            &paths
                .worker_projects_config_dir("8.4")
                .join(format!("{}.Caddyfile", project.project.id)),
        )?,
        worker_fragment
    );
    assert!(matches!(
        observed.status,
        state::ProjectEnvObservedStatus::Failed
    ));
    assert!(gateway_root_config.contains("import "));
    assert!(!gateway_root_config.contains("PV Gateway is running"));

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
async fn gateway_reconciliation_rolls_back_config_when_runtime_readiness_fails() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let release_path = tempdir.path().join("fake-frankenphp-release");
    let fake_frankenphp = release_path.join("bin/frankenphp");
    let ports = available_loopback_ports(4)?;
    let old_http_port = ports[0];
    let old_https_port = ports[1];
    let new_http_port = ports[2];
    let new_https_port = ports[3];

    write_fake_frankenphp_that_hangs_on_port(&fake_frankenphp, new_http_port)?;

    let mut database = Database::open(&paths)?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        "8.4",
        "fake-frankenphp-pv1",
        &release_path,
    )?;
    seed_runtime_ports(&paths, &mut database, old_http_port, old_https_port, &[])?;
    drop(database);

    reconcile_gateway_runtimes(&paths).await?;
    let first_gateway_pid = runtime_metadata_pid(&paths.gateway_runtime_metadata())?
        .ok_or_else(|| anyhow::anyhow!("expected gateway runtime metadata"))?;
    let previous_root_config = fs::read_to_string(&paths.gateway_root_config())?;

    let mut database = Database::open(&paths)?;
    database.release_port(PortOwner::Gateway(GatewayPort::Http))?;
    database.release_port(PortOwner::Gateway(GatewayPort::Https))?;
    seed_runtime_ports(&paths, &mut database, new_http_port, new_https_port, &[])?;
    drop(database);

    let result =
        reconcile_gateway_runtimes_with_readiness_timeout(&paths, Duration::from_millis(100)).await;
    let root_config = fs::read_to_string(&paths.gateway_root_config())?;
    let first_gateway_is_alive = process_is_alive(first_gateway_pid)?;
    if first_gateway_is_alive {
        stop_runtime_pid(first_gateway_pid).await?;
    } else if paths.gateway_pid().exists() {
        stop_runtime_from_pid_file(&paths.gateway_pid()).await?;
    }

    assert!(matches!(result, Err(DaemonError::ReadinessTimedOut { .. })));
    assert_eq!(root_config, previous_root_config);
    assert!(first_gateway_is_alive);

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
        &FrankenphpCommand::new(validator),
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

#[test]
fn frankenphp_command_and_process_specs_are_stable() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let command = FrankenphpCommand::new(tempdir.path().join("frankenphp"));
    let gateway = gateway_process_spec(&paths, &command);
    let worker_plan = php_worker_plan("8.4");
    let worker = worker_process_spec(&paths, &worker_plan, &command, tempdir.path())?;

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

    assert_process_spec_snapshot(
        tempdir.path(),
        (
            command.validate_arguments(&paths.gateway_root_config()),
            command.run_arguments(&paths.gateway_root_config()),
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
        &FrankenphpCommand::new(&validator),
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
    let command = FrankenphpCommand::new(&validator);
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
    let command = FrankenphpCommand::new(&validator);
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
    fs::write_sensitive_file(
        path,
        r#"#!/bin/sh
set -eu

if [ "$1" = "validate" ]; then
  test -f "$3"
  exit 0
fi

if [ "$1" = "run" ]; then
  python3 - "$3" <<'PY' &
import http.server
import re
import signal
import ssl
import sys
import threading

signal.signal(signal.SIGUSR1, signal.SIG_IGN)

config = open(sys.argv[1], encoding="utf-8").read()

def required(pattern):
    match = re.search(pattern, config, re.MULTILINE)
    if not match:
        raise SystemExit(f"missing fake runtime setting: {pattern}")
    return match.group(1)

def optional(pattern):
    match = re.search(pattern, config, re.MULTILINE)
    if not match:
        return None
    return match.group(1)

class Handler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, format, *args):
        pass

http_port = int(required(r"^# PV_FAKE_PORT (\d+)$"))
https_port = optional(r"^\s*https_port (\d+)$")
cert_path = optional(r'^\s*cert "([^"]+)"$')
key_path = optional(r'^\s*key "([^"]+)"$')
servers = [http.server.ThreadingHTTPServer(("127.0.0.1", http_port), Handler)]

if https_port is not None and cert_path is not None and key_path is not None:
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(certfile=cert_path, keyfile=key_path)
    https_server = http.server.ThreadingHTTPServer(("127.0.0.1", int(https_port)), Handler)
    https_server.socket = context.wrap_socket(https_server.socket, server_side=True)
    servers.append(https_server)

for server in servers[1:]:
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()

with servers[0] as server:
    server.serve_forever()
PY
  child="$!"
  trap ':' USR1
  trap 'kill "$child"; wait "$child"; exit 0' TERM INT
  while true; do
    wait "$child" && exit 0
    status="$?"
    if kill -0 "$child" 2>/dev/null; then
      continue
    fi
    exit "$status"
  done
fi

exit 2
"#,
    )?;
    set_executable(path)?;

    Ok(())
}

fn write_fake_frankenphp_that_hangs_on_port(path: &Utf8Path, blocked_port: u16) -> Result<()> {
    fs::write_sensitive_file(
        path,
        &format!(
            r#"#!/bin/sh
set -eu

if [ "$1" = "validate" ]; then
  test -f "$3"
  exit 0
fi

if [ "$1" = "run" ]; then
  port="$(awk '/^# PV_FAKE_PORT / {{ print $3; exit }}' "$3")"
  if [ "$port" = "{}" ]; then
    sleep 30
    exit 0
  fi
  python3 -c 'import http.server, signal, sys
signal.signal(signal.SIGUSR1, signal.SIG_IGN)
port = int(sys.argv[1])
with http.server.ThreadingHTTPServer(("127.0.0.1", port), http.server.SimpleHTTPRequestHandler) as server:
    server.serve_forever()
' "$port" &
  child="$!"
  trap ':' USR1
  trap 'kill "$child"; wait "$child"; exit 0' TERM INT
  while true; do
    wait "$child" && exit 0
    status="$?"
    if kill -0 "$child" 2>/dev/null; then
      continue
    fi
    exit "$status"
  done
fi

exit 2
"#,
            blocked_port
        ),
    )?;
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
    ])?;
    fs::write_sensitive_file(&paths.ca_certificate(), &certified_key.cert.pem())?;
    fs::write_sensitive_file(
        &paths.ca_private_key(),
        &certified_key.signing_key.serialize_pem(),
    )?;

    Ok(())
}

fn available_loopback_ports(count: usize) -> Result<Vec<u16>> {
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

    drop(listeners);

    Ok(ports)
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

fn remove_private_environment_fingerprint(path: &Utf8Path) -> Result<()> {
    let metadata = fs::read_to_string(path)?;
    let mut metadata: serde_json::Value = serde_json::from_str(&metadata)?;
    let Some(object) = metadata.as_object_mut() else {
        anyhow::bail!("runtime metadata must be a JSON object");
    };
    if object.remove("private_environment_fingerprint").is_none() {
        anyhow::bail!("runtime metadata is missing private_environment_fingerprint");
    }
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
        assert_debug_snapshot!("frankenphp_command_and_process_specs_are_stable", snapshot);
    });
}
