use anyhow::Result;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use daemon::DaemonError;
use daemon::gateway::{
    FrankenphpCommand, build_runtime_plan, gateway_process_spec, promote_validated_config_for_test,
    reconcile_gateway_runtimes, validate_config, worker_process_spec,
};
use insta::{Settings, assert_debug_snapshot};
use rustix::process::{Pid, Signal, kill_process_group, test_kill_process};
use serde_json::json;
use state::{
    Database, GatewayPort, LinkProjectInput, PortOwner, PortRequest, PvPaths,
    RUNTIME_PORT_FALLBACK_END, RUNTIME_PORT_FALLBACK_START, fs,
};
use std::net::TcpListener;
use std::time::Duration;
use tokio::time::sleep;

const GATEWAY_RECONCILIATION_SUMMARY: &str = "Gateway runtime reconciled";

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
    seed_runtime_ports(&mut database, ports[0], ports[1], &[("8.4", ports[2])])?;
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
    seed_runtime_ports(&mut database, ports[0], ports[1], &[])?;
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
    seed_runtime_ports(&mut database, ports[0], ports[1], &[("8.4", ports[2])])?;
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
    seed_runtime_ports(&mut database, ports[0], ports[1], &[("8.4", ports[2])])?;
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
    seed_runtime_ports(&mut database, ports[0], ports[1], &[("8.4", ports[2])])?;
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
async fn frankenphp_config_validation_timeout_stops_validator_process_group() -> Result<()> {
    let tempdir = tempdir()?;
    let validator = tempdir.path().join("hanging-validator");
    let validator_child_pid = tempdir.path().join("validator-child.pid");
    let config_path = tempdir.path().join("Caddyfile");

    write_hanging_frankenphp_validator(&validator, &validator_child_pid)?;
    fs::write_sensitive_file(&config_path, "{}\n")?;

    let result = validate_config(&FrankenphpCommand::new(&validator), &config_path).await;

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
    seed_runtime_ports(&mut database, ports[0], ports[1], &[("8.4", ports[2])])?;
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
        PortOwner::PhpWorker { php_track } if php_track == "8.4"
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
    seed_runtime_ports(&mut database, ports[0], ports[1], &[("8.4", ports[2])])?;
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
    seed_runtime_ports(&mut database, ports[0], ports[1], &[("8.4", ports[2])])?;
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
    seed_runtime_ports(&mut database, ports[0], ports[1], &[("8.4", ports[2])])?;
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

    let result = validate_config(&FrankenphpCommand::new(validator), &config_path).await;

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
    let worker = worker_process_spec(&paths, "8.4", &command);

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

fn create_project(project_root: &Utf8Path, config_source: &str) -> Result<()> {
    fs::write_sensitive_file(&project_root.join("public/index.php"), "<?php\n")?;
    fs::write_sensitive_file(&project_root.join("pv.yml"), config_source)?;

    Ok(())
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
  port="$(awk '/^# PV_FAKE_PORT / { print $3; exit }' "$3")"
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
    )?;
    set_executable(path)?;

    Ok(())
}

fn shell_single_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
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

fn seed_runtime_ports(
    database: &mut Database,
    gateway_http_port: u16,
    gateway_https_port: u16,
    php_workers: &[(&str, u16)],
) -> Result<()> {
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
