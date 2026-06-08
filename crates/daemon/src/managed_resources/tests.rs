use std::collections::BTreeMap;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::{Settings, assert_debug_snapshot};
use state::{
    Database, EnvContextValues, LinkProjectInput, PortOwner, PortRequest,
    ProjectManagedResourceInput, ProjectRecord, PvPaths, ResourceAllocationInput,
    ResourceAllocationRecord, ResourceAllocationStatus, RuntimeObservedStatus, RuntimeSubject,
    StateError,
};

use crate::{
    DaemonError, ProcessSpec, ReadinessCheck,
    managed_resources::{ManagedResourceRuntimeAdapter, ManagedResourceRuntimeContext},
};

const FAKE_MAILPIT_TRACK: &str = "1.0";
const FAKE_MAILPIT_NEXT_TRACK: &str = "1.1";
const FAKE_MAILPIT_ARTIFACT_VERSION: &str = "1.0.0-pv1";
const FAKE_MAILPIT_ARCHIVE_FILE_NAME: &str = "mailpit-1.0.0-pv1-any.tar.gz";
const FAKE_SQL_TRACK: &str = "8.0";
const FAKE_SQL_ARTIFACT_VERSION: &str = "8.0.0-pv1";
const MAILPIT_ARCHIVE_FILE_NAME: &str = "mailpit-1.0.0-pv1-any.tar.gz";
const REDIS_TRACK: &str = "7.2";
const REDIS_ARTIFACT_VERSION: &str = "7.2.0-pv1";
const REDIS_ARCHIVE_FILE_NAME: &str = "redis-7.2.0-pv1-any.tar.gz";
const OFFLINE_TEST_MANIFEST_URL: &str = "https://127.0.0.1:9/manifest.json";
const INVALID_DEFAULT_PORT_SPECS: &[super::ManagedResourcePortSpec] = &[
    super::ManagedResourcePortSpec {
        name: "smtp",
        preferred_port: 1025,
    },
    super::ManagedResourcePortSpec {
        name: "default",
        preferred_port: 8025,
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
struct RuntimeFilePresence {
    pid: bool,
    metadata: bool,
    config: bool,
}

#[tokio::test]
async fn mailpit_reconciliation_records_smtp_and_dashboard_env() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"mailpit:
  version: "1.0"
  env:
    MAIL_HOST: "${smtp_host}"
    MAIL_PORT: "${smtp_port}"
    MAILPIT_DASHBOARD: "${dashboard_url}"
"#,
    )?;
    seed_mailpit_fixture_artifact(&paths, FAKE_MAILPIT_TRACK)?;
    let mailpit_port_guards = seed_mailpit_runtime_ports(&paths, FAKE_MAILPIT_TRACK)?;

    drop(mailpit_port_guards);
    crate::project_env::reconcile_project_env(&paths, &project.id).await?;
    let snapshot = {
        let database = Database::open(&paths)?;
        let runtime_states = database.runtime_observed_states()?;

        assert_runtime_status(
            &runtime_states,
            FAKE_MAILPIT_TRACK,
            RuntimeObservedStatus::Running,
        );

        (
            read_dotenv(&project)?,
            database.managed_resource_track("mailpit", FAKE_MAILPIT_TRACK)?,
            database.assigned_ports()?,
            runtime_states,
        )
    };

    assert_with_normalized_runtime(
        tempdir.path(),
        "mailpit_reconciliation_records_smtp_and_dashboard_env",
        snapshot,
    )?;

    write_project_config(
        &project,
        r#"env:
  APP_URL: "${project_url}"
"#,
    )?;
    crate::project_env::reconcile_project_env(&paths, &project.id).await?;

    Ok(())
}

#[tokio::test]
async fn mailpit_project_demand_installs_missing_fixture_track_before_start() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"mailpit:
  version: "1.0"
  env:
    MAIL_HOST: "${smtp_host}"
    MAIL_PORT: "${smtp_port}"
    MAILPIT_DASHBOARD: "${dashboard_url}"
"#,
    )?;
    seed_mailpit_cached_fixture(&paths, tempdir.path())?;
    let mailpit_port_guards = seed_mailpit_runtime_ports(&paths, FAKE_MAILPIT_TRACK)?;

    drop(mailpit_port_guards);
    reconcile_project_env_with_mailpit_runtime_catalog_and_manifest_url(
        &paths,
        &project.id,
        OFFLINE_TEST_MANIFEST_URL,
    )
    .await?;
    let snapshot = {
        let database = Database::open(&paths)?;
        let runtime_states = database.runtime_observed_states()?;

        assert_runtime_status(
            &runtime_states,
            FAKE_MAILPIT_TRACK,
            RuntimeObservedStatus::Running,
        );

        (
            read_dotenv(&project)?,
            database.managed_resource_track("mailpit", FAKE_MAILPIT_TRACK)?,
            database.assigned_ports()?,
            runtime_states,
        )
    };

    assert_with_normalized_runtime(
        tempdir.path(),
        "mailpit_project_demand_installs_missing_fixture_track_before_start",
        snapshot,
    )?;

    write_project_config(
        &project,
        r#"env:
  APP_URL: "${project_url}"
"#,
    )?;
    reconcile_project_env_with_mailpit_runtime_catalog_and_manifest_url(
        &paths,
        &project.id,
        OFFLINE_TEST_MANIFEST_URL,
    )
    .await?;

    Ok(())
}

#[test]
fn mailpit_process_spec_uses_persistent_database_and_disables_version_check() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let data_dir = paths.resource_data_dir("mailpit", FAKE_MAILPIT_TRACK);
    let database_path = data_dir.join("mailpit.db");
    let context = ManagedResourceRuntimeContext {
        resource_name: "mailpit".to_string(),
        track: FAKE_MAILPIT_TRACK.to_string(),
        artifact_path: paths
            .resources()
            .join("mailpit")
            .join(FAKE_MAILPIT_TRACK)
            .join(format!("releases/{FAKE_MAILPIT_ARTIFACT_VERSION}")),
        data_dir,
        ports: BTreeMap::from([("smtp".to_string(), 1025), ("dashboard".to_string(), 8025)]),
    };
    let adapter = super::mailpit::MailpitRuntimeAdapter::new();

    let spec = adapter.build_process_spec(&paths, &context)?;

    assert_eq!(
        spec.arguments,
        vec![
            "--smtp".to_string(),
            "127.0.0.1:1025".to_string(),
            "--listen".to_string(),
            "127.0.0.1:8025".to_string(),
            "--database".to_string(),
            database_path.to_string(),
            "--disable-version-check".to_string(),
        ],
    );
    assert!(
        path_exists(&context.data_dir)?,
        "expected Mailpit data directory to be created before process start"
    );

    assert_with_normalized_runtime(
        tempdir.path(),
        "mailpit_process_spec_uses_persistent_database_and_disables_version_check",
        (spec.arguments, path_exists(&context.data_dir)?),
    )?;

    Ok(())
}

#[tokio::test]
async fn demanded_resource_starts_fake_multi_port_runtime_before_env_rendering() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"mailpit:
  version: "1.0"
  env:
    MAIL_HOST: "${smtp_host}"
    MAIL_PORT: "${smtp_port}"
    MAILPIT_DASHBOARD: "${dashboard_url}"
"#,
    )?;
    seed_fake_mailpit_artifact(&paths, FAKE_MAILPIT_TRACK)?;
    let mailpit_port_guards = seed_mailpit_runtime_ports(&paths, FAKE_MAILPIT_TRACK)?;

    drop(mailpit_port_guards);
    reconcile_project_env_with_fake_runtime_catalog(&paths, &project.id).await?;
    let started_snapshot = {
        let database = Database::open(&paths)?;

        (
            read_dotenv(&project)?,
            database.managed_resource_track("mailpit", FAKE_MAILPIT_TRACK)?,
            database.assigned_ports()?,
            database.runtime_observed_states()?,
        )
    };

    write_project_config(
        &project,
        r#"env:
  APP_URL: "${project_url}"
"#,
    )?;
    reconcile_project_env_with_fake_runtime_catalog(&paths, &project.id).await?;
    let stopped_snapshot = {
        let database = Database::open(&paths)?;

        (
            read_dotenv(&project)?,
            database.managed_resource_track("mailpit", FAKE_MAILPIT_TRACK)?,
            database.assigned_ports()?,
            database.runtime_observed_states()?,
        )
    };

    assert_with_normalized_runtime(
        tempdir.path(),
        "demanded_resource_starts_fake_multi_port_runtime_before_env_rendering",
        started_snapshot,
    )?;

    assert_with_normalized_runtime(
        tempdir.path(),
        "fake_multi_port_runtime_stops_when_project_demand_is_removed",
        stopped_snapshot,
    )?;

    Ok(())
}

#[test]
fn unready_fake_runtime_uses_http_readiness_to_avoid_parallel_tcp_collisions() -> Result<()> {
    let adapter = super::fake::FakeMailpitRuntimeAdapter::unready()?;
    let context = super::ManagedResourceRuntimeContext {
        resource_name: "mailpit".to_string(),
        track: FAKE_MAILPIT_TRACK.to_string(),
        artifact_path: "/pv/fake/mailpit".into(),
        data_dir: "/pv/fake/data".into(),
        ports: BTreeMap::from([
            ("smtp".to_string(), 18025),
            ("dashboard".to_string(), 18026),
        ]),
    };

    let readiness = adapter.readiness(&context)?;
    let super::ManagedResourceReadiness::TcpHttp(readiness) = readiness else {
        bail!("expected tcp/http readiness");
    };

    assert_eq!(
        readiness,
        ReadinessCheck::Http {
            host: super::RESOURCE_HOST.to_string(),
            port: 18026,
            path: "/__pv_unready_fixture__".to_string(),
        }
    );

    Ok(())
}

#[tokio::test]
async fn system_resource_reconciliation_stops_unlinked_project_runtime() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"mailpit:
  version: "1.0"
  env:
    MAIL_HOST: "${smtp_host}"
    MAIL_PORT: "${smtp_port}"
    MAILPIT_DASHBOARD: "${dashboard_url}"
"#,
    )?;
    seed_fake_mailpit_artifact(&paths, FAKE_MAILPIT_TRACK)?;
    let mailpit_port_guards = seed_mailpit_runtime_ports(&paths, FAKE_MAILPIT_TRACK)?;

    drop(mailpit_port_guards);
    reconcile_project_env_with_fake_runtime_catalog(&paths, &project.id).await?;
    let stale_port_guard = seed_mailpit_runtime_port(&paths, FAKE_MAILPIT_TRACK, "obsolete")?;
    drop(stale_port_guard);

    let cleanup_snapshot = {
        let mut database = Database::open(&paths)?;
        database.unlink_project(&project.id)?;
        let catalog = super::fake_runtime_catalog(super::DEFAULT_MANIFEST_URL)?;

        super::reconcile_system_resources_with_catalog(&paths, &mut database, &catalog).await?;

        (
            database.managed_resource_track("mailpit", FAKE_MAILPIT_TRACK)?,
            database.assigned_ports()?,
            database.runtime_observed_states()?,
            runtime_files_exist(&paths, FAKE_MAILPIT_TRACK)?,
        )
    };

    assert_runtime_status(
        &cleanup_snapshot.2,
        FAKE_MAILPIT_TRACK,
        RuntimeObservedStatus::Stopped,
    );
    assert_eq!(
        cleanup_snapshot.3,
        RuntimeFilePresence {
            pid: false,
            metadata: false,
            config: false,
        },
        "expected system cleanup to remove runtime files"
    );
    assert_with_normalized_runtime(
        tempdir.path(),
        "system_resource_reconciliation_stops_unlinked_project_runtime",
        cleanup_snapshot,
    )?;

    Ok(())
}

#[tokio::test]
async fn demanded_resource_reassigns_persisted_port_when_non_pv_listener_occupies_it() -> Result<()>
{
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"mailpit:
  version: "1.0"
  env:
    MAIL_HOST: "${smtp_host}"
    MAIL_PORT: "${smtp_port}"
    MAILPIT_DASHBOARD: "${dashboard_url}"
"#,
    )?;
    seed_fake_mailpit_artifact(&paths, FAKE_MAILPIT_TRACK)?;
    let stale_smtp_guard = seed_mailpit_runtime_port(&paths, FAKE_MAILPIT_TRACK, "smtp")?;
    let stale_smtp_port = stale_smtp_guard.local_addr()?.port();
    let stale_dashboard_guard = seed_mailpit_runtime_port(&paths, FAKE_MAILPIT_TRACK, "dashboard")?;
    let stale_dashboard_port = stale_dashboard_guard.local_addr()?.port();

    let result = reconcile_project_env_with_fake_runtime_catalog(&paths, &project.id).await;
    let reassign_snapshot = {
        let database = Database::open(&paths)?;

        (
            format!("{result:#?}"),
            read_optional_dotenv(&project)?,
            database.assigned_ports()?,
            database.runtime_observed_states()?,
        )
    };
    let stale_port_still_assigned = reassign_snapshot.2.iter().any(|assignment| {
        matches!(
            &assignment.owner,
            state::PortOwner::Resource { name, track, port }
                if name == "mailpit"
                    && track == FAKE_MAILPIT_TRACK
                    && ((port == "smtp" && assignment.port == stale_smtp_port)
                        || (port == "dashboard" && assignment.port == stale_dashboard_port))
        )
    });

    if result.is_ok() {
        write_project_config(
            &project,
            r#"env:
  APP_URL: "${project_url}"
"#,
        )?;
        let _cleanup = reconcile_project_env_with_fake_runtime_catalog(&paths, &project.id).await;
    }

    assert!(
        result.is_ok(),
        "expected occupied stale port to be reassigned, got {result:#?}"
    );
    assert!(
        !stale_port_still_assigned,
        "expected stale non-PV listener ports {stale_smtp_port} and {stale_dashboard_port} to be released"
    );
    assert_runtime_status(
        &reassign_snapshot.3,
        FAKE_MAILPIT_TRACK,
        RuntimeObservedStatus::Running,
    );
    assert_with_normalized_runtime(
        tempdir.path(),
        "demanded_resource_reassigns_persisted_port_when_non_pv_listener_occupies_it",
        reassign_snapshot,
    )?;

    Ok(())
}

#[tokio::test]
async fn demanded_resource_installs_fake_multi_port_runtime_from_cached_fixture_before_env_rendering()
-> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"mailpit:
  version: "1.0"
  env:
    MAIL_HOST: "${smtp_host}"
    MAIL_PORT: "${smtp_port}"
    MAILPIT_DASHBOARD: "${dashboard_url}"
"#,
    )?;
    seed_fake_mailpit_cached_fixture(&paths, tempdir.path())?;
    let mailpit_port_guards = seed_mailpit_runtime_ports(&paths, FAKE_MAILPIT_TRACK)?;

    drop(mailpit_port_guards);
    reconcile_project_env_with_fake_runtime_catalog_and_manifest_url(
        &paths,
        &project.id,
        OFFLINE_TEST_MANIFEST_URL,
    )
    .await?;
    let installed_snapshot = {
        let database = Database::open(&paths)?;

        (
            read_dotenv(&project)?,
            database.managed_resource_track("mailpit", FAKE_MAILPIT_TRACK)?,
            database.assigned_ports()?,
            database.runtime_observed_states()?,
        )
    };

    write_project_config(
        &project,
        r#"env:
  APP_URL: "${project_url}"
"#,
    )?;
    reconcile_project_env_with_fake_runtime_catalog_and_manifest_url(
        &paths,
        &project.id,
        OFFLINE_TEST_MANIFEST_URL,
    )
    .await?;
    let stopped_snapshot = {
        let database = Database::open(&paths)?;

        (
            database.managed_resource_track("mailpit", FAKE_MAILPIT_TRACK)?,
            database.assigned_ports()?,
            database.runtime_observed_states()?,
        )
    };

    assert_with_normalized_runtime(
        tempdir.path(),
        "demanded_resource_installs_fake_multi_port_runtime_from_cached_fixture_before_env_rendering",
        installed_snapshot,
    )?;
    assert_with_normalized_runtime(
        tempdir.path(),
        "cached_fake_multi_port_runtime_stops_when_project_demand_is_removed",
        stopped_snapshot,
    )?;

    Ok(())
}

#[tokio::test]
async fn demanded_resource_records_failed_runtime_when_readiness_fails_before_env_rendering()
-> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"mailpit:
  version: "1.0"
  env:
    MAIL_HOST: "${smtp_host}"
    MAIL_PORT: "${smtp_port}"
    MAILPIT_DASHBOARD: "${dashboard_url}"
"#,
    )?;
    seed_unready_fake_mailpit_artifact(&paths, FAKE_MAILPIT_TRACK)?;

    let result = reconcile_project_env_with_unready_fake_runtime_catalog(&paths, &project.id).await;
    let failure_snapshot = {
        let database = Database::open(&paths)?;
        let runtime_states = database.runtime_observed_states()?;
        let runtime_files = runtime_files_exist(&paths, FAKE_MAILPIT_TRACK)?;

        (
            format!("{result:#?}"),
            read_optional_dotenv(&project)?,
            database.managed_resource_track("mailpit", FAKE_MAILPIT_TRACK)?,
            database.assigned_ports()?,
            runtime_states.clone(),
            runtime_files.clone(),
            database.project_env_observed_state(&project.id)?,
        )
    };

    assert!(
        result.is_err(),
        "expected readiness failure, got {result:#?}"
    );
    assert_failed_mailpit_runtime(&failure_snapshot.4);
    assert_eq!(
        failure_snapshot.5,
        RuntimeFilePresence {
            pid: false,
            metadata: false,
            config: false,
        },
        "expected readiness failure cleanup to remove runtime files"
    );
    assert_with_normalized_runtime(
        tempdir.path(),
        "demanded_resource_records_failed_runtime_when_readiness_fails_before_env_rendering",
        failure_snapshot,
    )?;

    Ok(())
}

#[tokio::test]
async fn demanded_resource_cleans_runtime_files_when_process_exits_after_readiness() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"mailpit:
  version: "1.0"
  env:
    MAIL_HOST: "${smtp_host}"
    MAIL_PORT: "${smtp_port}"
    MAILPIT_DASHBOARD: "${dashboard_url}"
"#,
    )?;
    seed_fast_exit_fake_mailpit_artifact(&paths, FAKE_MAILPIT_TRACK)?;
    let mailpit_port_guards = seed_mailpit_runtime_ports(&paths, FAKE_MAILPIT_TRACK)?;

    drop(mailpit_port_guards);
    let result =
        reconcile_project_env_with_fast_exit_fake_runtime_catalog(&paths, &project.id).await;
    let failure_snapshot = {
        let database = Database::open(&paths)?;
        let runtime_states = database.runtime_observed_states()?;

        (
            format!("{result:#?}"),
            read_optional_dotenv(&project)?,
            database.assigned_ports()?,
            runtime_states.clone(),
            runtime_files_exist(&paths, FAKE_MAILPIT_TRACK)?,
            database.project_env_observed_state(&project.id)?,
        )
    };

    if result.is_ok() {
        write_project_config(
            &project,
            r#"env:
  APP_URL: "${project_url}"
"#,
        )?;
        let _cleanup = reconcile_project_env_with_fake_runtime_catalog(&paths, &project.id).await;
    }

    assert!(
        result.is_err(),
        "expected fast-exit runtime failure, got {result:#?}"
    );
    assert_failed_mailpit_runtime(&failure_snapshot.3);
    assert_eq!(
        failure_snapshot.4,
        RuntimeFilePresence {
            pid: false,
            metadata: false,
            config: false,
        },
        "expected fast-exit failure cleanup to remove runtime files"
    );
    assert_with_normalized_runtime(
        tempdir.path(),
        "demanded_resource_cleans_runtime_files_when_process_exits_after_readiness",
        failure_snapshot,
    )?;

    Ok(())
}

#[tokio::test]
async fn demanded_removed_track_fails_without_starting_runtime() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"mailpit:
  version: "1.0"
  env:
    MAIL_HOST: "${smtp_host}"
    MAIL_PORT: "${smtp_port}"
    MAILPIT_DASHBOARD: "${dashboard_url}"
"#,
    )?;
    seed_fake_mailpit_artifact(&paths, FAKE_MAILPIT_TRACK)?;
    {
        let mut database = Database::open(&paths)?;
        database.record_managed_resource_track_removal_intent(
            "mailpit",
            FAKE_MAILPIT_TRACK,
            false,
            true,
        )?;
    }
    let mailpit_port_guards = seed_mailpit_runtime_ports(&paths, FAKE_MAILPIT_TRACK)?;

    drop(mailpit_port_guards);
    let result = reconcile_project_env_with_fake_runtime_catalog(&paths, &project.id).await;
    let failure_snapshot = {
        let database = Database::open(&paths)?;
        let runtime_states = database.runtime_observed_states()?;

        (
            format!("{result:#?}"),
            read_optional_dotenv(&project)?,
            database.managed_resource_track("mailpit", FAKE_MAILPIT_TRACK)?,
            database.assigned_ports()?,
            runtime_states.clone(),
            database.project_env_observed_state(&project.id)?,
        )
    };

    if result.is_ok() {
        write_project_config(
            &project,
            r#"env:
  APP_URL: "${project_url}"
"#,
        )?;
        let _cleanup = reconcile_project_env_with_fake_runtime_catalog(&paths, &project.id).await;
    }

    assert!(
        result.is_err(),
        "expected removed track to fail before runtime start, got {result:#?}"
    );
    assert_failed_mailpit_runtime(&failure_snapshot.4);
    assert_with_normalized_runtime(
        tempdir.path(),
        "demanded_removed_track_fails_without_starting_runtime",
        failure_snapshot,
    )?;

    Ok(())
}

#[tokio::test]
async fn demanded_resource_records_failed_runtime_when_named_port_assignment_fails() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"mailpit:
  version: "1.0"
  env:
    MAIL_HOST: "${smtp_host}"
"#,
    )?;
    seed_fake_mailpit_artifact(&paths, FAKE_MAILPIT_TRACK)?;
    let catalog = invalid_default_port_runtime_catalog()?;
    let mut database = Database::open(&paths)?;

    let result = crate::project_env::reconcile_project_env_with_catalog(
        &paths,
        &mut database,
        &project.id,
        &catalog,
    )
    .await;
    let failure_snapshot = (
        format!("{result:#?}"),
        read_optional_dotenv(&project)?,
        database.assigned_ports()?,
        database.runtime_observed_states()?,
        database.project_env_observed_state(&project.id)?,
    );

    assert!(
        result.is_err(),
        "expected named port assignment failure, got {result:#?}"
    );
    assert_eq!(
        failure_snapshot.2,
        Vec::new(),
        "expected failed named port assignment to release earlier port rows"
    );
    assert_failed_mailpit_runtime(&failure_snapshot.3);
    assert_with_normalized_runtime(
        tempdir.path(),
        "demanded_resource_records_failed_runtime_when_named_port_assignment_fails",
        failure_snapshot,
    )?;

    Ok(())
}

#[tokio::test]
async fn production_demanded_resource_without_adapter_fails_before_env_rendering() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"mailpit:
  version: "1.0"
"#,
    )?;

    let catalog = empty_runtime_catalog();
    let mut database = Database::open(&paths)?;
    let result = crate::project_env::reconcile_project_env_with_catalog(
        &paths,
        &mut database,
        &project.id,
        &catalog,
    )
    .await;
    let failure_snapshot = {
        let database = Database::open(&paths)?;

        (
            format!("{result:#?}"),
            read_optional_dotenv(&project)?,
            database.project_managed_resources(&project.id)?,
            database.runtime_observed_states()?,
            database.project_env_observed_state(&project.id)?,
        )
    };

    assert!(
        result.is_err(),
        "expected unsupported production resource to fail, got {result:#?}"
    );
    assert_with_normalized_runtime(
        tempdir.path(),
        "production_demanded_resource_without_adapter_fails_before_env_rendering",
        failure_snapshot,
    )?;

    Ok(())
}

#[tokio::test]
async fn demand_change_stops_previous_runtime_when_new_runtime_readiness_fails() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"mailpit:
  version: "1.0"
  env:
    MAIL_HOST: "${smtp_host}"
    MAIL_PORT: "${smtp_port}"
    MAILPIT_DASHBOARD: "${dashboard_url}"
"#,
    )?;
    seed_fake_mailpit_artifact(&paths, FAKE_MAILPIT_TRACK)?;
    let mailpit_port_guards = seed_mailpit_runtime_ports(&paths, FAKE_MAILPIT_TRACK)?;

    drop(mailpit_port_guards);
    reconcile_project_env_with_fake_runtime_catalog(&paths, &project.id).await?;

    write_project_config(
        &project,
        r#"mailpit:
  version: "1.1"
  env:
    MAIL_HOST: "${smtp_host}"
    MAIL_PORT: "${smtp_port}"
    MAILPIT_DASHBOARD: "${dashboard_url}"
"#,
    )?;
    seed_unready_fake_mailpit_artifact(&paths, FAKE_MAILPIT_NEXT_TRACK)?;

    let result = reconcile_project_env_with_unready_fake_runtime_catalog(&paths, &project.id).await;
    let failure_snapshot = {
        let database = Database::open(&paths)?;
        let runtime_states = database.runtime_observed_states()?;

        (
            format!("{result:#?}"),
            read_dotenv(&project)?,
            database.managed_resource_track("mailpit", FAKE_MAILPIT_TRACK)?,
            database.managed_resource_track("mailpit", FAKE_MAILPIT_NEXT_TRACK)?,
            database.assigned_ports()?,
            runtime_states.clone(),
        )
    };
    let previous_runtime_stopped = runtime_has_status(
        &failure_snapshot.5,
        FAKE_MAILPIT_TRACK,
        RuntimeObservedStatus::Stopped,
    );

    if !previous_runtime_stopped {
        write_project_config(
            &project,
            r#"env:
  APP_URL: "${project_url}"
"#,
        )?;
        let _cleanup_result =
            reconcile_project_env_with_fake_runtime_catalog(&paths, &project.id).await;
    }

    assert!(
        result.is_err(),
        "expected readiness failure, got {result:#?}"
    );
    assert_runtime_status(
        &failure_snapshot.5,
        FAKE_MAILPIT_TRACK,
        RuntimeObservedStatus::Stopped,
    );
    assert_runtime_status(
        &failure_snapshot.5,
        FAKE_MAILPIT_NEXT_TRACK,
        RuntimeObservedStatus::Failed,
    );
    assert_with_normalized_runtime(
        tempdir.path(),
        "demand_change_stops_previous_runtime_when_new_runtime_readiness_fails",
        failure_snapshot,
    )?;

    Ok(())
}

#[tokio::test]
async fn demanded_resource_uses_async_readiness_and_allocation_hooks() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"mysql:
  version: "8.0"
  allocations:
    app-db:
      env:
        DATABASE_URL: "${url}"
"#,
    )?;
    seed_fake_sql_artifact(&paths, "mysql", FAKE_SQL_TRACK)?;
    let hook_events = Arc::new(Mutex::new(Vec::new()));
    let catalog = super::ManagedResourceRuntimeCatalog::with_adapter(
        super::ManagedResourceInstallOptions {
            manifest_url: super::DEFAULT_MANIFEST_URL.to_string(),
            target_platform: super::current_target_platform(),
        },
        AsyncSqlHookRuntimeAdapter::new(Arc::clone(&hook_events))?,
    );
    let mut database = Database::open(&paths)?;

    crate::project_env::reconcile_project_env_with_catalog(
        &paths,
        &mut database,
        &project.id,
        &catalog,
    )
    .await?;
    let started_snapshot = (
        read_dotenv(&project)?,
        database.resource_allocations(&project.id, "mysql")?,
        database.runtime_observed_states()?,
        cloned_hook_events(&hook_events)?,
    );

    write_project_config(
        &project,
        r#"env:
  APP_URL: "${project_url}"
"#,
    )?;
    crate::project_env::reconcile_project_env_with_catalog(
        &paths,
        &mut database,
        &project.id,
        &catalog,
    )
    .await?;

    assert_with_normalized_runtime(
        tempdir.path(),
        "demanded_resource_uses_async_readiness_and_allocation_hooks",
        started_snapshot,
    )?;

    Ok(())
}

#[tokio::test]
async fn async_readiness_reassigns_unowned_persisted_port_before_resource_readiness() -> Result<()>
{
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"mysql:
  version: "8.0"
  allocations:
    app-db:
      env:
        DATABASE_URL: "${url}"
"#,
    )?;
    seed_fake_sql_artifact(&paths, "mysql", FAKE_SQL_TRACK)?;
    let external_listener = TcpListener::bind(("127.0.0.1", 0))?;
    let occupied_port = external_listener.local_addr()?.port();
    let hook_events = Arc::new(Mutex::new(Vec::new()));
    let catalog = super::ManagedResourceRuntimeCatalog::with_adapter(
        super::ManagedResourceInstallOptions {
            manifest_url: super::DEFAULT_MANIFEST_URL.to_string(),
            target_platform: super::current_target_platform(),
        },
        AsyncSqlHookRuntimeAdapter::new(Arc::clone(&hook_events))?,
    );
    let mut database = Database::open(&paths)?;
    database.assign_port(
        PortRequest::resource_port(
            "mysql",
            FAKE_SQL_TRACK,
            "mysql",
            occupied_port,
            occupied_port,
            occupied_port,
        ),
        |_port| true,
    )?;

    crate::project_env::reconcile_project_env_with_catalog(
        &paths,
        &mut database,
        &project.id,
        &catalog,
    )
    .await?;
    let assigned_ports = database.assigned_ports()?;
    let resource_allocations = database.resource_allocations(&project.id, "mysql")?;
    let assigned_mysql_port = assigned_mysql_port(&assigned_ports)?;
    let dotenv = read_dotenv(&project)?;
    let uses_occupied_port = dotenv.contains(&format!(":{occupied_port}/"));

    assert_ne!(
        assigned_mysql_port, occupied_port,
        "expected async readiness resource to reassign the externally occupied persisted port"
    );
    assert!(
        !uses_occupied_port,
        "expected rendered env to use the reassigned resource port"
    );
    assert_with_normalized_runtime(
        tempdir.path(),
        "async_readiness_reassigns_unowned_persisted_port_before_resource_readiness",
        (
            (
                "persisted_port_reassigned",
                assigned_mysql_port != occupied_port,
            ),
            ("env_uses_occupied_port", uses_occupied_port),
            dotenv,
            assigned_ports,
            resource_allocations,
            database.runtime_observed_states()?,
            cloned_hook_events(&hook_events)?,
        ),
    )?;

    Ok(())
}

#[tokio::test]
async fn redis_reconciliation_marks_prefix_allocation_ready_and_renders_env() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        redis_project_config(),
    )?;
    seed_redis_fixture_artifact(&paths, REDIS_TRACK)?;

    crate::project_env::reconcile_project_env(&paths, &project.id).await?;
    let snapshot = {
        let database = Database::open(&paths)?;

        (
            read_dotenv(&project)?,
            database.managed_resource_track("redis", REDIS_TRACK)?,
            database.resource_allocations(&project.id, "redis")?,
            database.assigned_ports()?,
            database.runtime_observed_states()?,
        )
    };

    write_project_config(
        &project,
        r#"env:
  APP_URL: "${project_url}"
"#,
    )?;
    crate::project_env::reconcile_project_env(&paths, &project.id).await?;

    assert_with_normalized_runtime(
        tempdir.path(),
        "redis_reconciliation_marks_prefix_allocation_ready_and_renders_env",
        snapshot,
    )?;

    Ok(())
}

#[tokio::test]
async fn redis_reconciliation_reuses_ready_prefix_allocation() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        redis_project_config(),
    )?;
    let mut database = Database::open(&paths)?;
    database.replace_project_managed_resources(
        &project.id,
        &[ProjectManagedResourceInput {
            resource_name: "redis".to_string(),
            track: REDIS_TRACK.to_string(),
        }],
    )?;
    database.record_managed_resource_track_env_context(
        "redis",
        REDIS_TRACK,
        &BTreeMap::from([
            ("host".to_string(), "127.0.0.1".to_string()),
            ("port".to_string(), "6379".to_string()),
            ("url".to_string(), "redis://127.0.0.1:6379/0".to_string()),
        ]),
    )?;
    database.replace_project_resource_allocations(
        &project.id,
        "redis",
        REDIS_TRACK,
        &[ResourceAllocationInput {
            allocation_name: "cache".to_string(),
            generated_name: "acme-test-cache-".to_string(),
        }],
    )?;
    let adapter = super::redis::RedisRuntimeAdapter::new();
    let context = super::ManagedResourceRuntimeContext {
        resource_name: "redis".to_string(),
        track: REDIS_TRACK.to_string(),
        artifact_path: paths
            .resources()
            .join("redis")
            .join(REDIS_TRACK)
            .join(format!("releases/{REDIS_ARTIFACT_VERSION}")),
        data_dir: paths.resource_data_dir("redis", REDIS_TRACK),
        ports: BTreeMap::from([("redis".to_string(), 6379)]),
    };

    let desired_allocations = database.resource_allocations(&project.id, "redis")?;
    super::ManagedResourceRuntimeAdapter::reconcile_allocations(
        &adapter,
        &paths,
        &mut database,
        &context,
        &desired_allocations,
    )
    .await?;
    let first_snapshot = (
        database.resource_allocations(&project.id, "redis")?,
        database.project_env_context(&project.id)?,
    );

    let ready_allocations = database.resource_allocations(&project.id, "redis")?;
    super::ManagedResourceRuntimeAdapter::reconcile_allocations(
        &adapter,
        &paths,
        &mut database,
        &context,
        &ready_allocations,
    )
    .await?;
    let second_snapshot = (
        database.resource_allocations(&project.id, "redis")?,
        database.project_env_context(&project.id)?,
    );
    let [first_allocation] = first_snapshot.0.as_slice() else {
        bail!(
            "expected one Redis allocation after first reconciliation, got {:#?}",
            first_snapshot.0
        );
    };
    let [second_allocation] = second_snapshot.0.as_slice() else {
        bail!(
            "expected one Redis allocation after second reconciliation, got {:#?}",
            second_snapshot.0
        );
    };

    assert_eq!(first_allocation.status, ResourceAllocationStatus::Ready);
    assert_eq!(second_allocation.status, ResourceAllocationStatus::Ready);
    assert_eq!(
        first_allocation.generated_name, second_allocation.generated_name,
        "Redis prefix allocation should remain stable across reconciliations"
    );
    assert_eq!(
        first_allocation.env, second_allocation.env,
        "Redis allocation env should remain stable across reconciliations"
    );

    assert_with_normalized_runtime(
        tempdir.path(),
        "redis_reconciliation_reuses_ready_prefix_allocation",
        (first_snapshot, second_snapshot),
    )?;

    Ok(())
}

#[tokio::test]
async fn redis_project_demand_installs_missing_fixture_track_before_start() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        redis_project_config(),
    )?;
    seed_redis_cached_fixture(&paths, tempdir.path())?;

    crate::project_env::reconcile_project_env(&paths, &project.id).await?;
    let snapshot = {
        let database = Database::open(&paths)?;

        (
            read_dotenv(&project)?,
            database.managed_resource_track("redis", REDIS_TRACK)?,
            database.resource_allocations(&project.id, "redis")?,
            database.assigned_ports()?,
            database.runtime_observed_states()?,
        )
    };

    write_project_config(
        &project,
        r#"env:
  APP_URL: "${project_url}"
"#,
    )?;
    crate::project_env::reconcile_project_env(&paths, &project.id).await?;

    assert_with_normalized_runtime(
        tempdir.path(),
        "redis_project_demand_installs_missing_fixture_track_before_start",
        snapshot,
    )?;

    Ok(())
}

async fn reconcile_project_env_with_fake_runtime_catalog(
    paths: &PvPaths,
    project_id: &str,
) -> Result<()> {
    reconcile_project_env_with_fake_runtime_catalog_and_manifest_url(
        paths,
        project_id,
        super::DEFAULT_MANIFEST_URL,
    )
    .await
}

async fn reconcile_project_env_with_fake_runtime_catalog_and_manifest_url(
    paths: &PvPaths,
    project_id: &str,
    manifest_url: &str,
) -> Result<()> {
    let catalog = super::fake_runtime_catalog(manifest_url)?;
    let mut database = Database::open(paths)?;

    crate::project_env::reconcile_project_env_with_catalog(
        paths,
        &mut database,
        project_id,
        &catalog,
    )
    .await?;

    Ok(())
}

async fn reconcile_project_env_with_mailpit_runtime_catalog_and_manifest_url(
    paths: &PvPaths,
    project_id: &str,
    manifest_url: &str,
) -> Result<()> {
    let catalog = super::mailpit_runtime_catalog(manifest_url)?;
    let mut database = Database::open(paths)?;

    crate::project_env::reconcile_project_env_with_catalog(
        paths,
        &mut database,
        project_id,
        &catalog,
    )
    .await?;

    Ok(())
}

async fn reconcile_project_env_with_unready_fake_runtime_catalog(
    paths: &PvPaths,
    project_id: &str,
) -> Result<()> {
    let catalog = super::fake_unready_runtime_catalog(super::DEFAULT_MANIFEST_URL)?;
    let mut database = Database::open(paths)?;

    crate::project_env::reconcile_project_env_with_catalog(
        paths,
        &mut database,
        project_id,
        &catalog,
    )
    .await?;

    Ok(())
}

async fn reconcile_project_env_with_fast_exit_fake_runtime_catalog(
    paths: &PvPaths,
    project_id: &str,
) -> Result<()> {
    let catalog = super::ManagedResourceRuntimeCatalog::with_adapter(
        super::ManagedResourceInstallOptions {
            manifest_url: super::DEFAULT_MANIFEST_URL.to_string(),
            target_platform: super::current_target_platform(),
        },
        super::fake::FakeMailpitRuntimeAdapter::exits_after_readiness()?,
    );
    let mut database = Database::open(paths)?;

    crate::project_env::reconcile_project_env_with_catalog(
        paths,
        &mut database,
        project_id,
        &catalog,
    )
    .await?;

    Ok(())
}

fn invalid_default_port_runtime_catalog() -> Result<super::ManagedResourceRuntimeCatalog> {
    Ok(super::ManagedResourceRuntimeCatalog::with_adapter(
        super::ManagedResourceInstallOptions {
            manifest_url: super::DEFAULT_MANIFEST_URL.to_string(),
            target_platform: super::current_target_platform(),
        },
        InvalidDefaultPortRuntimeAdapter {
            artifact_adapter: super::ManagedResourceArtifactAdapter::new(
                "mailpit",
                "bin/pv-fake-mailpit",
            )?,
        },
    ))
}

fn link_project(
    paths: &PvPaths,
    project_path: &Utf8Path,
    primary_hostname: &str,
    config_source: &str,
) -> Result<ProjectRecord> {
    let config_path = project_path.join("pv.yml");

    state::fs::write_sensitive_file(&config_path, config_source)?;

    let mut database = Database::open(paths)?;
    let result = database.link_project(LinkProjectInput {
        path: project_path.to_path_buf(),
        original_path: project_path.to_path_buf(),
        primary_hostname: primary_hostname.to_string(),
        config_path,
        desired_php_track: None,
        additional_hostnames: Vec::new(),
    })?;

    Ok(result.project)
}

fn write_project_config(project: &ProjectRecord, config_source: &str) -> Result<()> {
    state::fs::write_sensitive_file(&project.config_path, config_source)?;

    Ok(())
}

fn read_dotenv(project: &ProjectRecord) -> Result<String> {
    state::fs::read_to_string(&project.path.join(".env")).map_err(Into::into)
}

fn read_optional_dotenv(project: &ProjectRecord) -> Result<Option<String>> {
    match state::fs::read_to_string(&project.path.join(".env")) {
        Ok(content) => Ok(Some(content)),
        Err(StateError::Filesystem { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

fn redis_project_config() -> &'static str {
    r#"redis:
  version: "7.2"
  env:
    REDIS_HOST: "${host}"
    REDIS_PORT: "${port}"
    REDIS_URL: "${url}"
  allocations:
    cache:
      env:
        CACHE_REDIS_HOST: "${host}"
        CACHE_REDIS_PORT: "${port}"
        CACHE_REDIS_PREFIX: "${prefix}"
        CACHE_REDIS_URL: "${url}"
"#
}

fn seed_fake_mailpit_artifact(paths: &PvPaths, track: &str) -> Result<()> {
    seed_fake_mailpit_artifact_with_script(paths, track, fake_mailpit_script())
}

fn seed_mailpit_fixture_artifact(paths: &PvPaths, track: &str) -> Result<()> {
    let release_path = paths
        .resources()
        .join("mailpit")
        .join(track)
        .join(format!("releases/{FAKE_MAILPIT_ARTIFACT_VERSION}"));
    let executable = release_path.join("bin/mailpit");

    state::fs::write_sensitive_file(&executable, mailpit_script())?;
    set_executable(&executable)?;
    let mut database = Database::open(paths)?;
    database.record_managed_resource_track_installed(
        "mailpit",
        track,
        FAKE_MAILPIT_ARTIFACT_VERSION,
        &release_path,
    )?;

    Ok(())
}

fn seed_fake_mailpit_artifact_with_script(
    paths: &PvPaths,
    track: &str,
    script: &str,
) -> Result<()> {
    let release_path = paths
        .resources()
        .join("mailpit")
        .join(track)
        .join(format!("releases/{FAKE_MAILPIT_ARTIFACT_VERSION}"));
    let executable = release_path.join("bin/pv-fake-mailpit");

    state::fs::write_sensitive_file(&executable, script)?;
    set_executable(&executable)?;
    let mut database = Database::open(paths)?;
    database.record_managed_resource_track_installed(
        "mailpit",
        track,
        FAKE_MAILPIT_ARTIFACT_VERSION,
        &release_path,
    )?;

    Ok(())
}

fn seed_unready_fake_mailpit_artifact(paths: &PvPaths, track: &str) -> Result<()> {
    seed_fake_mailpit_artifact_with_script(paths, track, unready_fake_mailpit_script())
}

fn seed_fast_exit_fake_mailpit_artifact(paths: &PvPaths, track: &str) -> Result<()> {
    seed_fake_mailpit_artifact_with_script(paths, track, fast_exit_fake_mailpit_script())
}

fn seed_mailpit_runtime_ports(paths: &PvPaths, track: &str) -> Result<[TcpListener; 2]> {
    let smtp_guard = seed_mailpit_runtime_port(paths, track, "smtp")?;
    let dashboard_guard = seed_mailpit_runtime_port(paths, track, "dashboard")?;

    Ok([smtp_guard, dashboard_guard])
}

fn seed_mailpit_runtime_port(paths: &PvPaths, track: &str, port_name: &str) -> Result<TcpListener> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();

    let mut database = Database::open(paths)?;
    database.assign_port(
        PortRequest::resource_port("mailpit", track, port_name, port, port, port),
        |candidate| candidate == port,
    )?;

    Ok(listener)
}

fn seed_fake_sql_artifact(paths: &PvPaths, resource: &str, track: &str) -> Result<()> {
    let release_path = paths
        .resources()
        .join(resource)
        .join(track)
        .join(format!("releases/{FAKE_SQL_ARTIFACT_VERSION}"));
    let executable = release_path.join("bin/pv-fake-sql");

    state::fs::write_sensitive_file(&executable, fake_sql_script())?;
    set_executable(&executable)?;
    let mut database = Database::open(paths)?;
    database.record_managed_resource_track_installed(
        resource,
        track,
        FAKE_SQL_ARTIFACT_VERSION,
        &release_path,
    )?;

    Ok(())
}

fn seed_redis_fixture_artifact(paths: &PvPaths, track: &str) -> Result<()> {
    let release_path = paths
        .resources()
        .join("redis")
        .join(track)
        .join(format!("releases/{REDIS_ARTIFACT_VERSION}"));
    let executable = release_path.join("bin/redis-server");

    state::fs::write_sensitive_file(&executable, redis_server_script())?;
    set_executable(&executable)?;
    let mut database = Database::open(paths)?;
    database.record_managed_resource_track_installed(
        "redis",
        track,
        REDIS_ARTIFACT_VERSION,
        &release_path,
    )?;

    Ok(())
}

fn empty_runtime_catalog() -> super::ManagedResourceRuntimeCatalog {
    super::ManagedResourceRuntimeCatalog {
        adapters: BTreeMap::new(),
        install_options: super::ManagedResourceInstallOptions {
            manifest_url: super::DEFAULT_MANIFEST_URL.to_string(),
            target_platform: super::current_target_platform(),
        },
    }
}

fn seed_fake_mailpit_cached_fixture(paths: &PvPaths, tempdir: &Utf8Path) -> Result<()> {
    let archive_path = tempdir.join(FAKE_MAILPIT_ARCHIVE_FILE_NAME);

    create_fake_mailpit_archive(tempdir, &archive_path)?;
    let sha256 = sha256_file(&archive_path)?;
    let cache_path = paths
        .downloads()
        .join(format!("{sha256}-{FAKE_MAILPIT_ARCHIVE_FILE_NAME}"));

    copy_file(&archive_path, &cache_path)?;
    let size = file_size(&cache_path)?;
    let manifest = fake_mailpit_manifest(&sha256, size);

    state::fs::write_sensitive_file(&paths.downloads().join("manifest.json"), &manifest)?;

    Ok(())
}

fn seed_mailpit_cached_fixture(paths: &PvPaths, tempdir: &Utf8Path) -> Result<()> {
    let archive_path = tempdir.join(MAILPIT_ARCHIVE_FILE_NAME);

    create_mailpit_archive(tempdir, &archive_path)?;
    let sha256 = sha256_file(&archive_path)?;
    let cache_path = paths
        .downloads()
        .join(format!("{sha256}-{MAILPIT_ARCHIVE_FILE_NAME}"));

    copy_file(&archive_path, &cache_path)?;
    let size = file_size(&cache_path)?;
    let manifest = mailpit_manifest(&sha256, size);

    state::fs::write_sensitive_file(&paths.downloads().join("manifest.json"), &manifest)?;

    Ok(())
}

fn seed_redis_cached_fixture(paths: &PvPaths, tempdir: &Utf8Path) -> Result<()> {
    let archive_path = tempdir.join(REDIS_ARCHIVE_FILE_NAME);

    create_redis_archive(tempdir, &archive_path)?;
    let sha256 = sha256_file(&archive_path)?;
    let cache_path = paths
        .downloads()
        .join(format!("{sha256}-{REDIS_ARCHIVE_FILE_NAME}"));

    copy_file(&archive_path, &cache_path)?;
    let size = file_size(&cache_path)?;
    let manifest = redis_manifest(&sha256, size);

    state::fs::write_sensitive_file(&paths.downloads().join("manifest.json"), &manifest)?;

    Ok(())
}

fn create_fake_mailpit_archive(tempdir: &Utf8Path, archive_path: &Utf8Path) -> Result<()> {
    let archive_parent = tempdir.join("archive-root");
    let root_name = format!("mailpit-{FAKE_MAILPIT_ARTIFACT_VERSION}");
    let root = archive_parent.join(&root_name);
    let executable = root.join("bin/pv-fake-mailpit");

    state::fs::write_sensitive_file(&executable, fake_mailpit_script())?;
    set_executable(&executable)?;
    run_fixture_command(
        "/usr/bin/tar",
        &[
            "-czf",
            archive_path.as_str(),
            "-C",
            archive_parent.as_str(),
            &root_name,
        ],
    )?;

    Ok(())
}

fn create_mailpit_archive(tempdir: &Utf8Path, archive_path: &Utf8Path) -> Result<()> {
    let archive_parent = tempdir.join("archive-root");
    let root_name = format!("mailpit-{FAKE_MAILPIT_ARTIFACT_VERSION}");
    let root = archive_parent.join(&root_name);
    let executable = root.join("bin/mailpit");

    state::fs::write_sensitive_file(&executable, mailpit_script())?;
    set_executable(&executable)?;
    run_fixture_command(
        "/usr/bin/tar",
        &[
            "-czf",
            archive_path.as_str(),
            "-C",
            archive_parent.as_str(),
            &root_name,
        ],
    )?;

    Ok(())
}

fn create_redis_archive(tempdir: &Utf8Path, archive_path: &Utf8Path) -> Result<()> {
    let archive_parent = tempdir.join("redis-archive-root");
    let root_name = format!("redis-{REDIS_ARTIFACT_VERSION}");
    let root = archive_parent.join(&root_name);
    let executable = root.join("bin/redis-server");

    state::fs::write_sensitive_file(&executable, redis_server_script())?;
    set_executable(&executable)?;
    run_fixture_command(
        "/usr/bin/tar",
        &[
            "-czf",
            archive_path.as_str(),
            "-C",
            archive_parent.as_str(),
            &root_name,
        ],
    )?;

    Ok(())
}

fn fake_mailpit_manifest(sha256: &str, size: u64) -> String {
    let dummy_sha256 = "0000000000000000000000000000000000000000000000000000000000000000";

    format!(
        r#"{{
  "schema_version": 1,
  "minimum_pv_version": "0.1.0",
  "resources": [
    {{
      "name": "mailpit",
      "default_track": "{FAKE_MAILPIT_TRACK}",
      "tracks": [
        {{
          "name": "{FAKE_MAILPIT_TRACK}",
          "artifacts": [
            {{
              "artifact_version": "{FAKE_MAILPIT_ARTIFACT_VERSION}",
              "upstream_version": "1.0.0",
              "pv_build_revision": "1",
              "platform": "any",
              "url": "https://artifacts.example.test/{FAKE_MAILPIT_ARCHIVE_FILE_NAME}",
              "sha256": "{sha256}",
              "size": {size},
              "published_at": "2026-06-08T00:00:00Z"
            }}
          ]
        }}
      ]
    }},
    {{
      "name": "php",
      "default_track": "8.4",
      "tracks": [
        {{
          "name": "8.4",
          "artifacts": [
            {{
              "artifact_version": "8.4.8-pv1",
              "upstream_version": "8.4.8",
              "pv_build_revision": "1",
              "platform": "any",
              "url": "https://artifacts.example.test/php-8.4.8-pv1-any.tar.gz",
              "sha256": "{dummy_sha256}",
              "size": 1,
              "published_at": "2026-06-08T00:00:00Z"
            }}
          ]
        }}
      ]
    }},
    {{
      "name": "frankenphp",
      "default_track": "8.4",
      "tracks": [
        {{
          "name": "8.4",
          "artifacts": [
            {{
              "artifact_version": "8.4.8-pv1",
              "upstream_version": "8.4.8",
              "pv_build_revision": "1",
              "platform": "any",
              "url": "https://artifacts.example.test/frankenphp-8.4.8-pv1-any.tar.gz",
              "sha256": "{dummy_sha256}",
              "size": 1,
              "published_at": "2026-06-08T00:00:00Z"
            }}
          ]
        }}
      ]
    }}
  ]
}}
"#
    )
}

fn mailpit_manifest(sha256: &str, size: u64) -> String {
    let dummy_sha256 = "0000000000000000000000000000000000000000000000000000000000000000";

    format!(
        r#"{{
  "schema_version": 1,
  "minimum_pv_version": "0.1.0",
  "resources": [
    {{
      "name": "mailpit",
      "default_track": "{FAKE_MAILPIT_TRACK}",
      "tracks": [
        {{
          "name": "{FAKE_MAILPIT_TRACK}",
          "artifacts": [
            {{
              "artifact_version": "{FAKE_MAILPIT_ARTIFACT_VERSION}",
              "upstream_version": "1.0.0",
              "pv_build_revision": "1",
              "platform": "any",
              "url": "https://artifacts.example.test/{MAILPIT_ARCHIVE_FILE_NAME}",
              "sha256": "{sha256}",
              "size": {size},
              "published_at": "2026-06-08T00:00:00Z"
            }}
          ]
        }}
      ]
    }},
    {{
      "name": "php",
      "default_track": "8.4",
      "tracks": [
        {{
          "name": "8.4",
          "artifacts": [
            {{
              "artifact_version": "8.4.8-pv1",
              "upstream_version": "8.4.8",
              "pv_build_revision": "1",
              "platform": "any",
              "url": "https://artifacts.example.test/php-8.4.8-pv1-any.tar.gz",
              "sha256": "{dummy_sha256}",
              "size": 1,
              "published_at": "2026-06-08T00:00:00Z"
            }}
          ]
        }}
      ]
    }},
    {{
      "name": "frankenphp",
      "default_track": "8.4",
      "tracks": [
        {{
          "name": "8.4",
          "artifacts": [
            {{
              "artifact_version": "8.4.8-pv1",
              "upstream_version": "8.4.8",
              "pv_build_revision": "1",
              "platform": "any",
              "url": "https://artifacts.example.test/frankenphp-8.4.8-pv1-any.tar.gz",
              "sha256": "{dummy_sha256}",
              "size": 1,
              "published_at": "2026-06-08T00:00:00Z"
            }}
          ]
        }}
      ]
    }}
  ]
}}
"#
    )
}

fn redis_manifest(sha256: &str, size: u64) -> String {
    let dummy_sha256 = "0000000000000000000000000000000000000000000000000000000000000000";

    format!(
        r#"{{
  "schema_version": 1,
  "minimum_pv_version": "0.1.0",
  "resources": [
    {{
      "name": "redis",
      "default_track": "{REDIS_TRACK}",
      "tracks": [
        {{
          "name": "{REDIS_TRACK}",
          "artifacts": [
            {{
              "artifact_version": "{REDIS_ARTIFACT_VERSION}",
              "upstream_version": "7.2.0",
              "pv_build_revision": "1",
              "platform": "any",
              "url": "https://artifacts.example.test/{REDIS_ARCHIVE_FILE_NAME}",
              "sha256": "{sha256}",
              "size": {size},
              "published_at": "2026-06-08T00:00:00Z"
            }}
          ]
        }}
      ]
    }},
    {{
      "name": "php",
      "default_track": "8.4",
      "tracks": [
        {{
          "name": "8.4",
          "artifacts": [
            {{
              "artifact_version": "8.4.8-pv1",
              "upstream_version": "8.4.8",
              "pv_build_revision": "1",
              "platform": "any",
              "url": "https://artifacts.example.test/php-8.4.8-pv1-any.tar.gz",
              "sha256": "{dummy_sha256}",
              "size": 1,
              "published_at": "2026-06-08T00:00:00Z"
            }}
          ]
        }}
      ]
    }},
    {{
      "name": "frankenphp",
      "default_track": "8.4",
      "tracks": [
        {{
          "name": "8.4",
          "artifacts": [
            {{
              "artifact_version": "8.4.8-pv1",
              "upstream_version": "8.4.8",
              "pv_build_revision": "1",
              "platform": "any",
              "url": "https://artifacts.example.test/frankenphp-8.4.8-pv1-any.tar.gz",
              "sha256": "{dummy_sha256}",
              "size": 1,
              "published_at": "2026-06-08T00:00:00Z"
            }}
          ]
        }}
      ]
    }}
  ]
}}
"#
    )
}

fn fake_mailpit_script() -> &'static str {
    r#"#!/bin/sh
set -eu

smtp_port="$1"
dashboard_port="$2"

python3 - "$smtp_port" "$dashboard_port" <<'PY'
import http.server
import signal
import socketserver
import sys
import threading

class SmtpHandler(socketserver.BaseRequestHandler):
    def handle(self):
        self.request.sendall(b"220 fake mailpit\r\n")

class TcpServer(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True

smtp = TcpServer(("127.0.0.1", int(sys.argv[1])), SmtpHandler)
dashboard = http.server.ThreadingHTTPServer(("127.0.0.1", int(sys.argv[2])), http.server.SimpleHTTPRequestHandler)

def stop(_signum, _frame):
    sys.exit(0)

signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)

threading.Thread(target=smtp.serve_forever, daemon=True).start()
dashboard.serve_forever()
PY
"#
}

fn unready_fake_mailpit_script() -> &'static str {
    r#"#!/bin/sh
set -eu

stop() {
  exit 0
}

trap stop TERM INT

while true; do
  sleep 1
done
"#
}

fn fast_exit_fake_mailpit_script() -> &'static str {
    r#"#!/bin/sh
set -eu

dashboard_port="$2"

python3 - "$dashboard_port" <<'PY'
import http.server
import os
import sys

class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"ready")
        self.wfile.flush()
        os._exit(0)

    def log_message(self, _format, *_args):
        pass

server = http.server.ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Handler)
server.serve_forever()
PY
"#
}

fn fake_sql_script() -> &'static str {
    r#"#!/bin/sh
set -eu

stop() {
  exit 0
}

trap stop TERM INT

while true; do
  sleep 1
done
"#
}

#[derive(Clone, Debug)]
struct AsyncSqlHookRuntimeAdapter {
    artifact_adapter: super::ManagedResourceArtifactAdapter,
    hook_events: Arc<Mutex<Vec<String>>>,
}

impl AsyncSqlHookRuntimeAdapter {
    fn new(hook_events: Arc<Mutex<Vec<String>>>) -> Result<Self> {
        Ok(Self {
            artifact_adapter: super::ManagedResourceArtifactAdapter::new(
                "mysql",
                "bin/pv-fake-sql",
            )?,
            hook_events,
        })
    }
}

impl super::ManagedResourceRuntimeAdapter for AsyncSqlHookRuntimeAdapter {
    fn resource_name(&self) -> &'static str {
        "mysql"
    }

    fn artifact_adapter(
        &self,
    ) -> Result<super::ManagedResourceArtifactAdapter, crate::DaemonError> {
        Ok(self.artifact_adapter.clone())
    }

    fn port_specs(&self) -> &'static [super::ManagedResourcePortSpec] {
        &[super::ManagedResourcePortSpec {
            name: "mysql",
            preferred_port: 3306,
        }]
    }

    fn build_process_spec(
        &self,
        paths: &PvPaths,
        context: &super::ManagedResourceRuntimeContext,
    ) -> Result<crate::ProcessSpec, crate::DaemonError> {
        let config_path = paths.resource_runtime_config(&context.resource_name, &context.track);
        state::fs::write_sensitive_file(&config_path, "{}")?;

        Ok(crate::ProcessSpec {
            name: format!("{}-{}", context.resource_name, context.track),
            command: self
                .artifact_adapter
                .executable_path(&context.artifact_path),
            arguments: Vec::new(),
            config_path,
            log_path: paths.resource_log(&context.resource_name, &context.track),
            pid_path: paths.resource_pid(&context.resource_name, &context.track),
            metadata_path: paths.resource_runtime_metadata(&context.resource_name, &context.track),
            resource_name: context.resource_name.clone(),
            track: context.track.clone(),
        })
    }

    fn readiness(
        &self,
        _context: &super::ManagedResourceRuntimeContext,
    ) -> Result<super::ManagedResourceReadiness, crate::DaemonError> {
        let hook_events = Arc::clone(&self.hook_events);

        Ok(super::ManagedResourceReadiness::async_check(
            "sql:mysql:admin",
            move || {
                let hook_events = Arc::clone(&hook_events);

                Box::pin(async move {
                    push_hook_event(&hook_events, "readiness")?;

                    Ok(())
                })
            },
        ))
    }

    fn resource_env(
        &self,
        context: &super::ManagedResourceRuntimeContext,
    ) -> Result<EnvContextValues, crate::DaemonError> {
        Ok(BTreeMap::from([
            ("host".to_string(), "127.0.0.1".to_string()),
            ("port".to_string(), required_sql_port(context)?.to_string()),
        ]))
    }

    fn reconcile_allocations<'a>(
        &'a self,
        _paths: &'a PvPaths,
        database: &'a mut Database,
        context: &'a super::ManagedResourceRuntimeContext,
        allocations: &'a [ResourceAllocationRecord],
    ) -> super::ManagedResourceAllocationFuture<'a> {
        let hook_events = Arc::clone(&self.hook_events);

        Box::pin(async move {
            push_hook_event(&hook_events, "allocation")?;
            let port = required_sql_port(context)?;

            for allocation in allocations {
                database.mark_resource_allocation_ready(
                    &allocation.project_id,
                    &allocation.resource_name,
                    &allocation.track,
                    &allocation.allocation_name,
                    &BTreeMap::from([
                        ("database".to_string(), allocation.generated_name.clone()),
                        ("host".to_string(), "127.0.0.1".to_string()),
                        ("password".to_string(), "secret".to_string()),
                        ("port".to_string(), port.to_string()),
                        (
                            "url".to_string(),
                            format!(
                                "mysql://root:secret@127.0.0.1:{port}/{}",
                                allocation.generated_name
                            ),
                        ),
                        ("username".to_string(), "root".to_string()),
                    ]),
                )?;
            }

            Ok(())
        })
    }
}

fn mailpit_script() -> &'static str {
    r#"#!/bin/sh
set -eu

smtp=""
listen=""
database=""
disable_version_check=false

while [ "$#" -gt 0 ]; do
  case "$1" in
    --smtp)
      smtp="$2"
      shift 2
      ;;
    --listen)
      listen="$2"
      shift 2
      ;;
    --database)
      database="$2"
      shift 2
      ;;
    --disable-version-check)
      disable_version_check=true
      shift
      ;;
    *)
      echo "unexpected argument: $1" >&2
      exit 2
      ;;
  esac
done

if [ -z "$smtp" ] || [ -z "$listen" ] || [ -z "$database" ]; then
  echo "missing required mailpit argument" >&2
  exit 2
fi

if [ "$disable_version_check" != true ]; then
  echo "missing --disable-version-check" >&2
  exit 2
fi

case "$database" in
  */mailpit.db)
    ;;
  *)
    echo "unexpected database path: $database" >&2
    exit 2
    ;;
esac

database_dir="$(dirname "$database")"
if [ ! -d "$database_dir" ]; then
  echo "database directory does not exist: $database_dir" >&2
  exit 2
fi

python3 - "$smtp" "$listen" <<'PY'
import http.server
import signal
import socketserver
import sys
import threading

def host_port(value):
    host, port = value.rsplit(":", 1)
    return host, int(port)

class SmtpHandler(socketserver.BaseRequestHandler):
    def handle(self):
        self.request.sendall(b"220 mailpit fixture\r\n")

class TcpServer(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True

smtp = TcpServer(host_port(sys.argv[1]), SmtpHandler)
dashboard = http.server.ThreadingHTTPServer(
    host_port(sys.argv[2]),
    http.server.SimpleHTTPRequestHandler,
)

def stop(_signum, _frame):
    sys.exit(0)

signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)

threading.Thread(target=smtp.serve_forever, daemon=True).start()
dashboard.serve_forever()
PY
"#
}

fn redis_server_script() -> &'static str {
    r#"#!/bin/sh
set -eu

port=""
data_dir=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    --port)
      port="$2"
      shift 2
      ;;
    --dir)
      data_dir="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done

if [ -n "$data_dir" ]; then
  mkdir -p "$data_dir"
fi

python3 - "$port" <<'PY'
import signal
import socketserver
import sys

class RedisPingHandler(socketserver.BaseRequestHandler):
    def handle(self):
        while True:
            data = self.request.recv(4096)
            if not data:
                return
            upper = data.upper()
            responses = []
            for _ in range(upper.count(b"CLIENT")):
                responses.append(b"+OK\r\n")
            for _ in range(upper.count(b"PING")):
                responses.append(b"+PONG\r\n")
            if not responses:
                responses.append(b"+OK\r\n")
            self.request.sendall(b"".join(responses))

class RedisServer(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True

def stop(_signum, _frame):
    server.shutdown()

server = RedisServer(("127.0.0.1", int(sys.argv[1])), RedisPingHandler)
signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)
server.serve_forever()
PY
"#
}

fn assert_with_normalized_runtime(
    tempdir: &Utf8Path,
    name: &'static str,
    snapshot: impl std::fmt::Debug,
) -> Result<()> {
    let mut settings = Settings::clone_current();
    settings.add_filter(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", "<timestamp>");
    settings.add_filter(&regex_literal(tempdir.as_str()), "<tempdir>");
    settings.add_filter(
        r#"project_id: "[a-z0-9]{10}""#,
        r#"project_id: "<project_id>""#,
    );
    settings.add_filter(
        r"MAILPIT_DASHBOARD=http://127\.0\.0\.1:\d+",
        "MAILPIT_DASHBOARD=http://127.0.0.1:<dashboard_port>",
    );
    settings.add_filter(r"MAIL_PORT=\d+", "MAIL_PORT=<smtp_port>");
    settings.add_filter(r"REDIS_PORT=\d+", "REDIS_PORT=<redis_port>");
    settings.add_filter(
        r"REDIS_URL=redis://127\.0\.0\.1:\d+/0",
        "REDIS_URL=redis://127.0.0.1:<redis_port>/0",
    );
    settings.add_filter(r"CACHE_REDIS_PORT=\d+", "CACHE_REDIS_PORT=<redis_port>");
    settings.add_filter(
        r"CACHE_REDIS_URL=redis://127\.0\.0\.1:\d+/0",
        "CACHE_REDIS_URL=redis://127.0.0.1:<redis_port>/0",
    );
    settings.add_filter(
        r#""dashboard_url": "http://127\.0\.0\.1:\d+""#,
        r#""dashboard_url": "http://127.0.0.1:<dashboard_port>""#,
    );
    settings.add_filter(r#""smtp_port": "\d+""#, r#""smtp_port": "<smtp_port>""#);
    settings.add_filter(
        r#""url": "redis://127\.0\.0\.1:\d+/0""#,
        r#""url": "redis://127.0.0.1:<redis_port>/0""#,
    );
    settings.add_filter(
        r"redis-ping:127\.0\.0\.1:\d+",
        "redis-ping:127.0.0.1:<redis_port>",
    );
    settings.add_filter(r#""port": "\d+""#, r#""port": "<port>""#);
    settings.add_filter(r"tcp:127\.0\.0\.1:\d+", "tcp:127.0.0.1:<readiness_port>");
    settings.add_filter(
        r"http:127\.0\.0\.1:\d+/__pv_unready_fixture__",
        "http:127.0.0.1:<readiness_port>/__pv_unready_fixture__",
    );
    settings.add_filter(
        r"DATABASE_URL=mysql://root:secret@127\.0\.0\.1:\d+/acme_test_app_db",
        "DATABASE_URL=mysql://root:secret@127.0.0.1:<mysql_port>/acme_test_app_db",
    );
    settings.add_filter(
        r#"mysql://root:secret@127\.0\.0\.1:\d+/acme_test_app_db"#,
        "mysql://root:secret@127.0.0.1:<mysql_port>/acme_test_app_db",
    );
    settings.add_filter(r"timeout_ms: \d+", "timeout_ms: <timeout_ms>");
    settings.add_filter(r"os error \d+", "os error <code>");
    settings.add_filter(
        r"I/O error: Connection refused \(os error <code>\)|I/O error: HTTP readiness returned non-success status|deadline has elapsed",
        "I/O error: readiness unavailable",
    );
    settings.add_filter(r"port: \d+", "port: <port>");
    settings.bind(|| {
        assert_debug_snapshot!(name, snapshot);
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

fn assert_failed_mailpit_runtime(states: &[state::RuntimeObservedStateRecord]) {
    assert_runtime_status(states, FAKE_MAILPIT_TRACK, RuntimeObservedStatus::Failed);
}

fn push_hook_event(
    hook_events: &Arc<Mutex<Vec<String>>>,
    event: &str,
) -> Result<(), crate::DaemonError> {
    let mut events =
        hook_events
            .lock()
            .map_err(|_error| crate::DaemonError::UnexpectedProtocolResponse {
                reason: "async hook event log was poisoned".to_string(),
            })?;
    events.push(event.to_string());

    Ok(())
}

fn cloned_hook_events(hook_events: &Arc<Mutex<Vec<String>>>) -> Result<Vec<String>> {
    let events = hook_events
        .lock()
        .map_err(|_error| anyhow::anyhow!("async hook event log was poisoned"))?;

    Ok(events.clone())
}

fn required_sql_port(
    context: &super::ManagedResourceRuntimeContext,
) -> Result<u16, crate::DaemonError> {
    context.ports.get("mysql").copied().ok_or_else(|| {
        crate::DaemonError::ManagedResourcePortMissing {
            resource: context.resource_name.clone(),
            track: context.track.clone(),
            port: "mysql".to_string(),
        }
    })
}

fn assigned_mysql_port(assignments: &[state::PortAssignment]) -> Result<u16> {
    assignments
        .iter()
        .find_map(|assignment| match &assignment.owner {
            PortOwner::Resource {
                name,
                track: owner_track,
                port,
            } if name == "mysql" && owner_track == FAKE_SQL_TRACK && port == "mysql" => {
                Some(assignment.port)
            }
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("missing assigned mysql {FAKE_SQL_TRACK} mysql port"))
}

fn assert_runtime_status(
    states: &[state::RuntimeObservedStateRecord],
    track: &str,
    status: RuntimeObservedStatus,
) {
    let found = runtime_has_status(states, track, status);
    assert!(
        found,
        "expected mailpit track {track:?} runtime status {status:?}, got {states:#?}"
    );
}

fn runtime_has_status(
    states: &[state::RuntimeObservedStateRecord],
    track: &str,
    status: RuntimeObservedStatus,
) -> bool {
    let expected_subject = RuntimeSubject::Resource {
        name: "mailpit".to_string(),
        track: track.to_string(),
    };

    states
        .iter()
        .any(|record| record.subject == expected_subject && record.status == status)
}

fn runtime_files_exist(paths: &PvPaths, track: &str) -> Result<RuntimeFilePresence> {
    Ok(RuntimeFilePresence {
        pid: path_exists(&paths.resource_pid("mailpit", track))?,
        metadata: path_exists(&paths.resource_runtime_metadata("mailpit", track))?,
        config: path_exists(&paths.resource_runtime_config("mailpit", track))?,
    })
}

#[expect(
    clippy::disallowed_methods,
    reason = "daemon runtime tests assert runtime file cleanup directly"
)]
fn path_exists(path: &Utf8Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn sha256_file(path: &Utf8Path) -> Result<String> {
    let output = run_fixture_command("/usr/bin/shasum", &["-a", "256", path.as_str()])?;
    let text = String::from_utf8(output)?;
    let Some((sha256, _path)) = text.split_once(' ') else {
        bail!("shasum output did not include a sha256 digest");
    };

    Ok(sha256.to_string())
}

#[expect(
    clippy::disallowed_types,
    reason = "daemon runtime tests shell out to build archive fixtures without extra dev-dependencies"
)]
fn run_fixture_command(program: &str, args: &[&str]) -> Result<Vec<u8>> {
    let output = std::process::Command::new(program)
        .env("COPYFILE_DISABLE", "1")
        .args(args)
        .output()?;
    if !output.status.success() {
        bail!(
            "fixture command `{program}` failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(output.stdout)
}

#[expect(
    clippy::disallowed_methods,
    reason = "daemon runtime tests seed cached artifact fixtures directly"
)]
fn copy_file(from: &Utf8Path, to: &Utf8Path) -> Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(from, to)?;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "daemon runtime tests read fixture archive metadata for manifest size"
)]
fn file_size(path: &Utf8Path) -> Result<u64> {
    Ok(std::fs::metadata(path)?.len())
}

#[expect(
    clippy::disallowed_methods,
    reason = "daemon runtime tests set fixture executable bits directly"
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

#[derive(Clone, Debug)]
struct InvalidDefaultPortRuntimeAdapter {
    artifact_adapter: super::ManagedResourceArtifactAdapter,
}

impl ManagedResourceRuntimeAdapter for InvalidDefaultPortRuntimeAdapter {
    fn resource_name(&self) -> &'static str {
        "mailpit"
    }

    fn artifact_adapter(&self) -> Result<super::ManagedResourceArtifactAdapter, DaemonError> {
        Ok(self.artifact_adapter.clone())
    }

    fn port_specs(&self) -> &'static [super::ManagedResourcePortSpec] {
        INVALID_DEFAULT_PORT_SPECS
    }

    fn build_process_spec(
        &self,
        _paths: &PvPaths,
        _context: &super::ManagedResourceRuntimeContext,
    ) -> Result<ProcessSpec, DaemonError> {
        Err(DaemonError::UnexpectedProtocolResponse {
            reason: "invalid default-port test adapter should fail during port assignment"
                .to_string(),
        })
    }

    fn readiness(
        &self,
        _context: &super::ManagedResourceRuntimeContext,
    ) -> Result<super::ManagedResourceReadiness, DaemonError> {
        Err(DaemonError::UnexpectedProtocolResponse {
            reason: "invalid default-port test adapter should fail during port assignment"
                .to_string(),
        })
    }

    fn resource_env(
        &self,
        _context: &super::ManagedResourceRuntimeContext,
    ) -> Result<EnvContextValues, DaemonError> {
        Ok(BTreeMap::new())
    }
}
