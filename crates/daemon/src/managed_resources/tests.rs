use std::collections::BTreeMap;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, bail};
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::{Settings, assert_debug_snapshot};
use resources::{
    ManagedResourceCommandError, ResourceName, ResourcesError, RuntimeArtifactAdapter,
};
use serde::Deserialize;
use state::{
    Database, EnvContextValues, LinkProjectInput, PortOwner, PortRequest,
    ProjectManagedResourceInput, ProjectRecord, PvPaths, ResourceAllocationInput,
    ResourceAllocationRecord, ResourceAllocationStatus, RuntimeObservedStatus, RuntimeSubject,
    StateError,
};

use crate::{
    DaemonError, ProcessSpec, ProcessSupervisor, ReadinessCheck,
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
const RUSTFS_TRACK: &str = "1.0";
const RUSTFS_ARTIFACT_VERSION: &str = "1.0.0-pv1";
const RUSTFS_ARCHIVE_FILE_NAME: &str = "rustfs-1.0.0-pv1-any.tar.gz";
const POSTGRES_TRACK: &str = "16";
const POSTGRES_ARTIFACT_VERSION: &str = "16.0-pv1";
const POSTGRES_ARCHIVE_FILE_NAME: &str = "postgres-16.0-pv1-any.tar.gz";
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

#[derive(Debug, PartialEq)]
struct RustfsRuntimeCredentialSnapshot {
    process_env: BTreeMap<String, String>,
    recorded_access_key: String,
    recorded_secret_key: String,
    runtime_metadata: RustfsRuntimeMetadataSnapshot,
}

#[derive(Debug, Deserialize, PartialEq)]
struct RustfsRuntimeMetadataSnapshot {
    name: String,
    command: String,
    arguments: Vec<String>,
    config_path: String,
    resource_name: String,
    track: String,
    log_path: String,
}

#[tokio::test]
async fn postgres_reconciliation_creates_database_allocation_and_renders_env() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project_with_postgres_database_env(&paths, &tempdir.path().join("project"))?;
    seed_postgres_fixture_artifact(&paths, POSTGRES_TRACK)?;
    reserve_postgres_port(&paths, 19_060)?;

    crate::project_env::reconcile_project_env(&paths, &project.id).await?;
    let snapshot = {
        let database = Database::open(&paths)?;

        (
            read_dotenv(&project)?,
            database.managed_resource_track("postgres", POSTGRES_TRACK)?,
            database.assigned_ports()?,
            database.resource_allocations(&project.id, "postgres")?,
            database.runtime_observed_states()?,
        )
    };

    assert_with_normalized_postgres_runtime(
        tempdir.path(),
        "postgres_reconciliation_creates_database_allocation_and_renders_env",
        snapshot,
    )?;
    stop_postgres_runtime(&paths, &project).await?;

    Ok(())
}

#[tokio::test]
async fn postgres_project_demand_installs_missing_fixture_track_before_start() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project_with_postgres_database_env(&paths, &tempdir.path().join("project"))?;
    seed_postgres_cached_fixture(&paths, tempdir.path())?;
    reserve_postgres_port(&paths, 19_061)?;

    reconcile_project_env_with_postgres_runtime_catalog_and_manifest_url(
        &paths,
        &project.id,
        OFFLINE_TEST_MANIFEST_URL,
    )
    .await?;
    let snapshot = {
        let database = Database::open(&paths)?;

        (
            read_dotenv(&project)?,
            database.managed_resource_track("postgres", POSTGRES_TRACK)?,
            database.assigned_ports()?,
            database.resource_allocations(&project.id, "postgres")?,
            database.runtime_observed_states()?,
        )
    };

    assert_with_normalized_postgres_runtime(
        tempdir.path(),
        "postgres_project_demand_installs_missing_fixture_track_before_start",
        snapshot,
    )?;
    stop_postgres_runtime_with_manifest_url(&paths, &project, OFFLINE_TEST_MANIFEST_URL).await?;

    Ok(())
}

#[tokio::test]
async fn postgres_project_demand_rejects_cached_fixture_missing_support_files() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project_with_postgres_database_env(&paths, &tempdir.path().join("project"))?;
    seed_postgres_cached_fixture_without_support_files(&paths, tempdir.path())?;
    let catalog = super::postgres_runtime_catalog(OFFLINE_TEST_MANIFEST_URL)?;
    let mut database = Database::open(&paths)?;

    let result = crate::project_env::reconcile_project_env_with_catalog(
        &paths,
        &mut database,
        &project.id,
        &catalog,
    )
    .await;

    let Err(DaemonError::ManagedResourceCommand(ManagedResourceCommandError::Resources(
        ResourcesError::InvalidArtifactLayout { resource, reason },
    ))) = result
    else {
        bail!("expected missing Postgres support files to reject project-demand install");
    };
    let runtime_states = database.runtime_observed_states()?;

    assert_eq!(resource, "postgres");
    assert_eq!(
        reason,
        "missing required file `share/postgresql/postgres.bki`"
    );
    assert_eq!(
        read_optional_dotenv(&project)?,
        None,
        "expected rejected Postgres artifact to leave Project env withheld"
    );
    assert_runtime_status_for_resource(
        &runtime_states,
        "postgres",
        POSTGRES_TRACK,
        RuntimeObservedStatus::Failed,
    );

    Ok(())
}

#[tokio::test]
async fn postgres_reconciliation_writes_tcp_only_runtime_config() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project_with_postgres_database_env(&paths, &tempdir.path().join("project"))?;
    seed_postgres_fixture_artifact(&paths, POSTGRES_TRACK)?;
    reserve_postgres_port(&paths, 19_064)?;

    crate::project_env::reconcile_project_env(&paths, &project.id).await?;
    let data_config = state::fs::read_to_string(
        &paths
            .resource_data_dir("postgres", POSTGRES_TRACK)
            .join("postgresql.conf"),
    )?;
    let runtime_config =
        state::fs::read_to_string(&paths.resource_runtime_config("postgres", POSTGRES_TRACK))?;

    stop_postgres_runtime(&paths, &project).await?;

    assert!(
        data_config.contains("unix_socket_directories = ''"),
        "expected Postgres data config to disable Unix socket listeners"
    );
    assert!(
        runtime_config.contains("unix_socket_directories = ''"),
        "expected recorded Postgres runtime config to disable Unix socket listeners"
    );

    Ok(())
}

#[tokio::test]
async fn postgres_reconciliation_replaces_stale_admin_username_from_track_env() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project_with_postgres_database_env(&paths, &tempdir.path().join("project"))?;
    seed_postgres_fixture_artifact(&paths, POSTGRES_TRACK)?;
    reserve_postgres_port(&paths, 19_062)?;
    {
        let mut database = Database::open(&paths)?;
        database.record_managed_resource_track_env_context(
            "postgres",
            POSTGRES_TRACK,
            &BTreeMap::from([
                ("host".to_string(), "127.0.0.1".to_string()),
                (
                    "password".to_string(),
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                ),
                ("port".to_string(), "19032".to_string()),
                (
                    "url".to_string(),
                    "postgres://pv_postgres:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa@127.0.0.1:19032"
                        .to_string(),
                ),
                ("username".to_string(), "pv_postgres".to_string()),
            ]),
        )?;
    }

    crate::project_env::reconcile_project_env(&paths, &project.id).await?;
    let snapshot = {
        let database = Database::open(&paths)?;

        (
            read_dotenv(&project)?,
            database.managed_resource_track("postgres", POSTGRES_TRACK)?,
            database.resource_allocations(&project.id, "postgres")?,
            database.runtime_observed_states()?,
        )
    };
    stop_postgres_runtime(&paths, &project).await?;

    assert_eq!(
        snapshot.1.env.get("username").map(String::as_str),
        Some("pv_root"),
        "expected reconciliation to replace stale Postgres admin username"
    );
    assert_with_normalized_postgres_runtime(
        tempdir.path(),
        "postgres_reconciliation_replaces_stale_admin_username_from_track_env",
        snapshot,
    )?;

    Ok(())
}

#[tokio::test]
async fn postgres_reconciliation_retries_with_initialized_password_after_config_write_failure()
-> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project_with_postgres_database_env(&paths, &tempdir.path().join("project"))?;
    seed_postgres_fixture_artifact(&paths, POSTGRES_TRACK)?;
    reserve_postgres_port(&paths, 19_065)?;
    let config_parent_blocker = paths.config().join("resources");
    state::fs::write_sensitive_file(&config_parent_blocker, "not a directory")?;

    let failed_result = crate::project_env::reconcile_project_env(&paths, &project.id).await;
    let initdb_password = state::fs::read_to_string(
        &paths
            .resource_data_dir("postgres", POSTGRES_TRACK)
            .join("initdb.password"),
    )?;
    let failed_track_env = {
        let database = Database::open(&paths)?;
        database
            .managed_resource_track("postgres", POSTGRES_TRACK)?
            .env
    };

    assert!(
        failed_result.is_err(),
        "expected config write failure after initdb, got {failed_result:#?}"
    );
    assert_eq!(
        read_optional_dotenv(&project)?,
        None,
        "expected failed retry setup to leave Project env withheld"
    );
    assert_eq!(
        failed_track_env.get("password").map(String::as_str),
        Some(initdb_password.as_str()),
        "expected initdb password to be persisted before config writes can fail"
    );

    state::fs::delete_file(&config_parent_blocker)?;
    crate::project_env::reconcile_project_env(&paths, &project.id).await?;
    let recovered_dotenv = read_dotenv(&project)?;
    let (recovered_track, recovered_allocations, runtime_states) = {
        let database = Database::open(&paths)?;
        (
            database.managed_resource_track("postgres", POSTGRES_TRACK)?,
            database.resource_allocations(&project.id, "postgres")?,
            database.runtime_observed_states()?,
        )
    };
    stop_postgres_runtime(&paths, &project).await?;

    assert!(
        recovered_dotenv.contains(&format!("DB_PASSWORD={initdb_password}\n")),
        "expected retry to render Project env with the password from initialized PGDATA"
    );
    assert_eq!(
        recovered_track.env.get("password").map(String::as_str),
        Some(initdb_password.as_str()),
        "expected retry to reuse the password from initialized PGDATA"
    );
    let recovered_database_password = recovered_allocations
        .iter()
        .find(|allocation| allocation.allocation_name == "app-db")
        .and_then(|allocation| allocation.env.get("password"))
        .map(String::as_str);
    assert_eq!(
        recovered_database_password,
        Some(initdb_password.as_str()),
        "expected retry to render database allocation env with the persisted password"
    );
    assert_runtime_status_for_resource(
        &runtime_states,
        "postgres",
        POSTGRES_TRACK,
        RuntimeObservedStatus::Running,
    );

    Ok(())
}

#[tokio::test]
async fn postgres_reconciliation_records_generated_env_when_readiness_fails_after_initdb()
-> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project_with_postgres_database_env(&paths, &tempdir.path().join("project"))?;
    seed_unready_postgres_fixture_artifact(&paths, POSTGRES_TRACK)?;
    reserve_postgres_port(&paths, 19_063)?;

    let result =
        reconcile_project_env_with_unready_postgres_runtime_catalog(&paths, &project.id).await;
    let initdb_password = state::fs::read_to_string(
        &paths
            .resource_data_dir("postgres", POSTGRES_TRACK)
            .join("initdb.password"),
    )?;
    let snapshot = {
        let database = Database::open(&paths)?;

        (
            format!("{result:#?}"),
            read_optional_dotenv(&project)?,
            database.managed_resource_track("postgres", POSTGRES_TRACK)?,
            database.runtime_observed_states()?,
            runtime_files_exist_for_resource(&paths, "postgres", POSTGRES_TRACK)?,
        )
    };

    assert!(
        result.is_err(),
        "expected readiness failure, got {result:#?}"
    );
    assert_eq!(
        snapshot.2.env.get("username").map(String::as_str),
        Some("pv_root"),
        "expected readiness failure to preserve generated Postgres admin username"
    );
    assert_eq!(
        snapshot.2.env.get("password").map(String::as_str),
        Some(initdb_password.as_str()),
        "expected readiness failure to persist the initdb password for retry"
    );
    assert_runtime_status_for_resource(
        &snapshot.3,
        "postgres",
        POSTGRES_TRACK,
        RuntimeObservedStatus::Failed,
    );
    assert_eq!(
        snapshot.4,
        RuntimeFilePresence {
            pid: false,
            metadata: false,
            config: false,
        },
        "expected readiness failure cleanup to remove runtime files"
    );
    assert_with_normalized_postgres_runtime(
        tempdir.path(),
        "postgres_reconciliation_records_generated_env_when_readiness_fails_after_initdb",
        snapshot,
    )?;

    Ok(())
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
        env: BTreeMap::new(),
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
        env: BTreeMap::new(),
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
async fn rustfs_reconciliation_creates_bucket_and_renders_env() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project_with_rustfs_bucket_env(&paths, &tempdir.path().join("project"))?;
    seed_rustfs_fixture_artifact(&paths, RUSTFS_TRACK)?;
    reserve_rustfs_ports(&paths, 19_000, 19_001)?;

    reconcile_project_env_with_rustfs_runtime_catalog(&paths, &project.id).await?;
    let snapshot = {
        let database = Database::open(&paths)?;
        let allocations = database.resource_allocations(&project.id, "rustfs")?;
        let Some(allocation) = allocations.first() else {
            bail!("RustFS reconciliation did not record an allocation");
        };

        (
            read_dotenv(&project)?,
            database.managed_resource_track("rustfs", RUSTFS_TRACK)?,
            read_optional_rustfs_probe(&paths, RUSTFS_TRACK, &allocation.generated_name)?,
            allocations,
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
    let _cleanup_result =
        reconcile_project_env_with_rustfs_runtime_catalog(&paths, &project.id).await;

    assert_with_normalized_runtime(
        tempdir.path(),
        "rustfs_reconciliation_creates_bucket_and_renders_env",
        snapshot,
    )?;

    Ok(())
}

#[tokio::test]
async fn rustfs_project_demand_installs_missing_fixture_track_before_start() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project_with_rustfs_bucket_env(&paths, &tempdir.path().join("project"))?;
    seed_rustfs_cached_fixture(&paths, tempdir.path())?;
    reserve_rustfs_ports(&paths, 19_010, 19_011)?;

    reconcile_project_env_with_rustfs_runtime_catalog_and_manifest_url(
        &paths,
        &project.id,
        OFFLINE_TEST_MANIFEST_URL,
    )
    .await?;
    let snapshot = {
        let database = Database::open(&paths)?;

        (
            read_dotenv(&project)?,
            database.managed_resource_track("rustfs", RUSTFS_TRACK)?,
            database.resource_allocations(&project.id, "rustfs")?,
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
    let _cleanup_result = reconcile_project_env_with_rustfs_runtime_catalog_and_manifest_url(
        &paths,
        &project.id,
        OFFLINE_TEST_MANIFEST_URL,
    )
    .await;

    assert_with_normalized_runtime(
        tempdir.path(),
        "rustfs_project_demand_installs_missing_fixture_track_before_start",
        snapshot,
    )?;

    Ok(())
}

#[tokio::test]
async fn rustfs_ready_allocation_reconciliation_repairs_missing_bucket_and_preserves_env()
-> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project_with_rustfs_bucket_env(&paths, &tempdir.path().join("project"))?;
    seed_rustfs_fixture_artifact(&paths, RUSTFS_TRACK)?;
    reserve_rustfs_ports(&paths, 19_020, 19_021)?;

    reconcile_project_env_with_rustfs_runtime_catalog(&paths, &project.id).await?;
    let (bucket, allocation_env_before) = {
        let database = Database::open(&paths)?;
        let allocations = database.resource_allocations(&project.id, "rustfs")?;
        let Some(allocation) = allocations.first() else {
            bail!("RustFS reconciliation did not record an allocation");
        };

        (allocation.generated_name.clone(), allocation.env.clone())
    };
    state::fs::delete_dir_all(&rustfs_bucket_path(&paths, RUSTFS_TRACK, &bucket))?;

    reconcile_project_env_with_rustfs_runtime_catalog(&paths, &project.id).await?;
    let repaired_probe = read_optional_rustfs_probe(&paths, RUSTFS_TRACK, &bucket)?;
    let snapshot = {
        let database = Database::open(&paths)?;
        let allocations = database.resource_allocations(&project.id, "rustfs")?;
        let Some(allocation) = allocations.first() else {
            bail!("RustFS reconciliation did not preserve an allocation");
        };

        assert_eq!(
            allocation.env, allocation_env_before,
            "Ready RustFS allocation env should not change during drift repair"
        );
        assert_eq!(
            repaired_probe,
            Some("pv rustfs probe".to_string()),
            "Ready RustFS allocation reconciliation should verify object access"
        );

        (
            read_dotenv(&project)?,
            allocations,
            rustfs_bucket_exists(&paths, RUSTFS_TRACK, &bucket)?,
            repaired_probe,
        )
    };

    write_project_config(
        &project,
        r#"env:
  APP_URL: "${project_url}"
"#,
    )?;
    let _cleanup_result =
        reconcile_project_env_with_rustfs_runtime_catalog(&paths, &project.id).await;

    assert_with_normalized_runtime(
        tempdir.path(),
        "rustfs_ready_allocation_reconciliation_repairs_missing_bucket_and_preserves_env",
        snapshot,
    )?;

    Ok(())
}

#[tokio::test]
async fn rustfs_port_reassignment_renders_current_endpoint_for_ready_allocation() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project_with_rustfs_bucket_env(&paths, &tempdir.path().join("project"))?;
    seed_rustfs_fixture_artifact(&paths, RUSTFS_TRACK)?;
    reserve_rustfs_ports(&paths, 19_030, 19_031)?;

    reconcile_project_env_with_rustfs_runtime_catalog(&paths, &project.id).await?;
    let (bucket, initial_endpoint) = {
        let database = Database::open(&paths)?;
        let track = database.managed_resource_track("rustfs", RUSTFS_TRACK)?;
        let allocations = database.resource_allocations(&project.id, "rustfs")?;
        let Some(allocation) = allocations.first() else {
            bail!("RustFS reconciliation did not record an allocation");
        };

        (
            allocation.generated_name.clone(),
            required_env_value(&track.env, "endpoint")?,
        )
    };
    stop_recorded_rustfs_runtime(&paths).await?;
    let _api_port_guard = TcpListener::bind(("127.0.0.1", 19_030))?;
    let _console_port_guard = TcpListener::bind(("127.0.0.1", 19_031))?;

    reconcile_project_env_with_rustfs_runtime_catalog(&paths, &project.id).await?;
    let snapshot = {
        let database = Database::open(&paths)?;
        let track = database.managed_resource_track("rustfs", RUSTFS_TRACK)?;
        let current_endpoint = required_env_value(&track.env, "endpoint")?;
        let allocations = database.resource_allocations(&project.id, "rustfs")?;
        let Some(allocation) = allocations.first() else {
            bail!("RustFS reconciliation did not preserve an allocation");
        };
        let dotenv = read_dotenv(&project)?;

        assert_eq!(allocation.generated_name, bucket);
        assert_ne!(
            initial_endpoint, current_endpoint,
            "test setup should force RustFS onto a new API endpoint"
        );
        assert!(
            !allocation.env.contains_key("endpoint"),
            "Ready allocation env should not persist resource-level endpoint values"
        );
        assert!(
            dotenv.contains(&format!("S3_ENDPOINT={current_endpoint}")),
            "resource-level endpoint should render from current RustFS resource env"
        );
        assert!(
            dotenv.contains(&format!("AWS_ENDPOINT={current_endpoint}")),
            "allocation endpoint should render from current RustFS resource env"
        );
        assert!(
            dotenv.contains(&format!("AWS_URL={current_endpoint}")),
            "allocation URL should render from current RustFS resource env"
        );

        (
            dotenv,
            track,
            allocations,
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
    let _cleanup_result =
        reconcile_project_env_with_rustfs_runtime_catalog(&paths, &project.id).await;

    assert_with_normalized_runtime(
        tempdir.path(),
        "rustfs_port_reassignment_renders_current_endpoint_for_ready_allocation",
        snapshot,
    )?;

    Ok(())
}

#[tokio::test]
async fn rustfs_allocation_failure_preserves_project_env_and_records_failed_runtime() -> Result<()>
{
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project_with_rustfs_bucket_env(&paths, &tempdir.path().join("project"))?;
    state::fs::write_sensitive_file(&project.path.join(".env"), "EXISTING=value\n")?;
    seed_auth_rejecting_rustfs_fixture_artifact(&paths, RUSTFS_TRACK)?;
    reserve_available_rustfs_ports(&paths)?;

    let result = reconcile_project_env_with_rustfs_runtime_catalog(&paths, &project.id).await;
    let snapshot = {
        let database = Database::open(&paths)?;

        (
            format!("{result:#?}"),
            read_dotenv(&project)?,
            database.resource_allocations(&project.id, "rustfs")?,
            database.runtime_observed_states()?,
            database.project_env_observed_state(&project.id)?,
        )
    };

    assert!(
        result.is_err(),
        "expected RustFS allocation failure, got {result:#?}"
    );
    assert!(
        snapshot
            .2
            .iter()
            .all(|allocation| allocation.status != ResourceAllocationStatus::Ready),
        "allocation failure should not mark RustFS allocations Ready"
    );
    assert!(
        snapshot
            .3
            .iter()
            .any(|state| state.status == RuntimeObservedStatus::Failed),
        "allocation failure should record a failed RustFS runtime"
    );
    assert_with_normalized_runtime(
        tempdir.path(),
        "rustfs_allocation_failure_preserves_project_env_and_records_failed_runtime",
        snapshot,
    )?;

    Ok(())
}

#[tokio::test]
async fn rustfs_runtime_receives_private_credentials_without_persisting_them() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project_with_rustfs_bucket_env(&paths, &tempdir.path().join("project"))?;
    seed_rustfs_fixture_artifact(&paths, RUSTFS_TRACK)?;
    reserve_available_rustfs_ports(&paths)?;

    reconcile_project_env_with_rustfs_runtime_catalog(&paths, &project.id).await?;
    let first_snapshot = rustfs_runtime_credential_snapshot(&paths, &project.id)?;
    assert_process_credentials_match_recorded_env(&first_snapshot)?;
    assert_runtime_metadata_omits_credentials(&first_snapshot)?;

    write_project_config(
        &project,
        r#"env:
  APP_URL: "${project_url}"
"#,
    )?;
    reconcile_project_env_with_rustfs_runtime_catalog(&paths, &project.id).await?;
    delete_optional_file(&rustfs_process_env_path(&paths, RUSTFS_TRACK))?;

    write_project_config(
        &project,
        r#"rustfs:
  version: "1.0"
  env:
    S3_ENDPOINT: "${endpoint}"
    S3_ACCESS_KEY: "${access_key}"
    S3_SECRET_KEY: "${secret_key}"
  allocations:
    uploads:
      env:
        AWS_BUCKET: "${bucket}"
        AWS_ENDPOINT: "${endpoint}"
        AWS_ACCESS_KEY_ID: "${access_key}"
        AWS_SECRET_ACCESS_KEY: "${secret_key}"
        AWS_URL: "${url}"
"#,
    )?;
    reconcile_project_env_with_rustfs_runtime_catalog(&paths, &project.id).await?;
    let restarted_snapshot = rustfs_runtime_credential_snapshot(&paths, &project.id)?;
    assert_process_credentials_match_recorded_env(&restarted_snapshot)?;
    assert_runtime_metadata_omits_credentials(&restarted_snapshot)?;
    assert_eq!(
        first_snapshot.recorded_access_key, restarted_snapshot.recorded_access_key,
        "RustFS access key should remain stable from recorded context"
    );
    assert_eq!(
        first_snapshot.recorded_secret_key, restarted_snapshot.recorded_secret_key,
        "RustFS secret key should remain stable from recorded context"
    );

    write_project_config(
        &project,
        r#"env:
  APP_URL: "${project_url}"
"#,
    )?;
    let _cleanup_result =
        reconcile_project_env_with_rustfs_runtime_catalog(&paths, &project.id).await;

    assert_with_normalized_runtime(
        tempdir.path(),
        "rustfs_runtime_receives_private_credentials_without_persisting_them",
        (first_snapshot, restarted_snapshot),
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
async fn demanded_resource_persists_env_before_runtime_side_effects() -> Result<()> {
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
    let hook_events = cloned_hook_events(&hook_events)?;

    assert!(
        hook_events.contains(&"build_process_spec:persisted_env".to_string()),
        "expected managed resource env to be persisted before runtime spec/build side effects: {hook_events:#?}"
    );

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
    let redis_port_guard = seed_redis_runtime_port(&paths)?;

    drop(redis_port_guard);
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
        env: BTreeMap::new(),
    };
    let resource_env = BTreeMap::new();

    let desired_allocations = database.resource_allocations(&project.id, "redis")?;
    super::ManagedResourceRuntimeAdapter::reconcile_allocations(
        &adapter,
        &paths,
        &mut database,
        &context,
        &resource_env,
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
        &resource_env,
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

#[test]
fn redis_process_arguments_disable_snapshots_without_empty_argv() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let adapter = super::redis::RedisRuntimeAdapter::new();
    let context = super::ManagedResourceRuntimeContext {
        resource_name: "redis".to_string(),
        track: REDIS_TRACK.to_string(),
        artifact_path: tempdir.path().join("redis-artifact"),
        data_dir: paths.resource_data_dir("redis", REDIS_TRACK),
        ports: BTreeMap::from([("redis".to_string(), 6380)]),
        env: BTreeMap::new(),
    };

    let spec =
        super::ManagedResourceRuntimeAdapter::build_process_spec(&adapter, &paths, &context)?;

    assert!(
        !spec.arguments.iter().any(String::is_empty),
        "Redis process arguments must not contain empty argv tokens: {:#?}",
        spec.arguments
    );
    let config = state::fs::read_to_string(&spec.config_path)?;
    assert_with_normalized_runtime(
        tempdir.path(),
        "redis_process_arguments_disable_snapshots_without_empty_argv",
        (spec.arguments, config),
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
    let redis_port_guard = seed_redis_runtime_port(&paths)?;

    drop(redis_port_guard);
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

async fn reconcile_project_env_with_rustfs_runtime_catalog(
    paths: &PvPaths,
    project_id: &str,
) -> Result<()> {
    reconcile_project_env_with_rustfs_runtime_catalog_and_manifest_url(
        paths,
        project_id,
        super::DEFAULT_MANIFEST_URL,
    )
    .await
}

async fn reconcile_project_env_with_rustfs_runtime_catalog_and_manifest_url(
    paths: &PvPaths,
    project_id: &str,
    manifest_url: &str,
) -> Result<()> {
    let catalog = super::rustfs_runtime_catalog(manifest_url)?;
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

async fn reconcile_project_env_with_postgres_runtime_catalog_and_manifest_url(
    paths: &PvPaths,
    project_id: &str,
    manifest_url: &str,
) -> Result<()> {
    let catalog = super::postgres_runtime_catalog(manifest_url)?;
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
            artifact_adapter: RuntimeArtifactAdapter::new(
                ResourceName::new("mailpit")?,
                "bin/pv-fake-mailpit",
            ),
        },
    ))
}

async fn reconcile_project_env_with_unready_postgres_runtime_catalog(
    paths: &PvPaths,
    project_id: &str,
) -> Result<()> {
    let catalog = super::postgres_runtime_catalog_with_readiness_timeout(
        super::DEFAULT_MANIFEST_URL,
        Duration::from_millis(100),
    )?;
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

fn link_project_with_rustfs_bucket_env(
    paths: &PvPaths,
    project_path: &Utf8Path,
) -> Result<ProjectRecord> {
    link_project(
        paths,
        project_path,
        "acme.test",
        r#"rustfs:
  version: "1.0"
  env:
    S3_ENDPOINT: "${endpoint}"
    S3_ACCESS_KEY: "${access_key}"
    S3_SECRET_KEY: "${secret_key}"
  allocations:
    uploads:
      env:
        AWS_BUCKET: "${bucket}"
        AWS_ENDPOINT: "${endpoint}"
        AWS_ACCESS_KEY_ID: "${access_key}"
        AWS_SECRET_ACCESS_KEY: "${secret_key}"
        AWS_URL: "${url}"
"#,
    )
}

fn link_project_with_postgres_database_env(
    paths: &PvPaths,
    project_path: &Utf8Path,
) -> Result<ProjectRecord> {
    link_project(
        paths,
        project_path,
        "acme.test",
        r#"postgres:
  version: "16"
  env:
    PGHOST: "${host}"
    PGPASSWORD: "${password}"
    PGPORT: "${port}"
    PGUSER: "${username}"
  allocations:
    app-db:
      env:
        DATABASE_URL: "${url}"
        DB_DATABASE: "${database}"
        DB_HOST: "${host}"
        DB_PASSWORD: "${password}"
        DB_PORT: "${port}"
        DB_USERNAME: "${username}"
"#,
    )
}

fn write_project_config(project: &ProjectRecord, config_source: &str) -> Result<()> {
    state::fs::write_sensitive_file(&project.config_path, config_source)?;

    Ok(())
}

async fn stop_postgres_runtime(paths: &PvPaths, project: &ProjectRecord) -> Result<()> {
    write_project_config(
        project,
        r#"env:
  APP_URL: "${project_url}"
"#,
    )?;
    let _summary = crate::project_env::reconcile_project_env(paths, &project.id).await?;

    Ok(())
}

async fn stop_postgres_runtime_with_manifest_url(
    paths: &PvPaths,
    project: &ProjectRecord,
    manifest_url: &str,
) -> Result<()> {
    write_project_config(
        project,
        r#"env:
  APP_URL: "${project_url}"
"#,
    )?;
    reconcile_project_env_with_postgres_runtime_catalog_and_manifest_url(
        paths,
        &project.id,
        manifest_url,
    )
    .await?;

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

fn reserve_rustfs_ports(paths: &PvPaths, api_port: u16, console_port: u16) -> Result<()> {
    let mut database = Database::open(paths)?;

    reserve_rustfs_port(&mut database, "api", api_port)?;
    reserve_rustfs_port(&mut database, "console", console_port)?;

    Ok(())
}

fn reserve_available_rustfs_ports(paths: &PvPaths) -> Result<()> {
    let api_listener = TcpListener::bind(("127.0.0.1", 0))?;
    let console_listener = TcpListener::bind(("127.0.0.1", 0))?;
    let api_port = api_listener.local_addr()?.port();
    let console_port = console_listener.local_addr()?.port();
    drop(api_listener);
    drop(console_listener);

    reserve_rustfs_ports(paths, api_port, console_port)
}

fn reserve_rustfs_port(database: &mut Database, port_name: &str, port: u16) -> Result<()> {
    database.assign_port(
        PortRequest::resource_port("rustfs", RUSTFS_TRACK, port_name, port, port, port),
        local_loopback_port_available,
    )?;

    Ok(())
}

fn reserve_postgres_port(paths: &PvPaths, port: u16) -> Result<()> {
    let mut database = Database::open(paths)?;
    database.assign_port(
        PortRequest::resource_port("postgres", POSTGRES_TRACK, "postgres", port, port, port),
        local_loopback_port_available,
    )?;

    Ok(())
}

fn local_loopback_port_available(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

fn rustfs_bucket_path(paths: &PvPaths, track: &str, bucket: &str) -> camino::Utf8PathBuf {
    paths
        .resource_data_dir("rustfs", track)
        .join("buckets")
        .join(bucket)
}

fn rustfs_probe_path(paths: &PvPaths, track: &str, bucket: &str) -> camino::Utf8PathBuf {
    rustfs_bucket_path(paths, track, bucket).join("__pv_rustfs_probe")
}

fn rustfs_process_env_path(paths: &PvPaths, track: &str) -> camino::Utf8PathBuf {
    paths.resource_data_dir("rustfs", track).join("process-env")
}

async fn stop_recorded_rustfs_runtime(paths: &PvPaths) -> Result<()> {
    if let Some(process) = ProcessSupervisor::new(paths.clone()).adopt_recorded(
        &paths.resource_pid("rustfs", RUSTFS_TRACK),
        &paths.resource_runtime_metadata("rustfs", RUSTFS_TRACK),
    )? {
        process.stop(Duration::from_secs(1)).await?;
    }

    Ok(())
}

fn rustfs_bucket_exists(paths: &PvPaths, track: &str, bucket: &str) -> Result<bool> {
    path_exists(&rustfs_bucket_path(paths, track, bucket))
}

fn read_optional_rustfs_probe(
    paths: &PvPaths,
    track: &str,
    bucket: &str,
) -> Result<Option<String>> {
    match state::fs::read_to_string(&rustfs_probe_path(paths, track, bucket)) {
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

fn rustfs_runtime_credential_snapshot(
    paths: &PvPaths,
    _project_id: &str,
) -> Result<RustfsRuntimeCredentialSnapshot> {
    let database = Database::open(paths)?;
    let track = database.managed_resource_track("rustfs", RUSTFS_TRACK)?;
    let process_env = read_rustfs_process_env(paths, RUSTFS_TRACK)?;
    let runtime_metadata_source =
        state::fs::read_to_string(&paths.resource_runtime_metadata("rustfs", RUSTFS_TRACK))?;
    let recorded_access_key = required_env_value(&track.env, "access_key")?;
    let recorded_secret_key = required_env_value(&track.env, "secret_key")?;
    assert!(
        !runtime_metadata_source.contains(&recorded_access_key),
        "raw RustFS runtime metadata should not contain the access key"
    );
    assert!(
        !runtime_metadata_source.contains(&recorded_secret_key),
        "raw RustFS runtime metadata should not contain the secret key"
    );
    let runtime_metadata = serde_json::from_str(&runtime_metadata_source)?;

    Ok(RustfsRuntimeCredentialSnapshot {
        process_env,
        recorded_access_key,
        recorded_secret_key,
        runtime_metadata,
    })
}

fn read_rustfs_process_env(paths: &PvPaths, track: &str) -> Result<BTreeMap<String, String>> {
    let content = state::fs::read_to_string(&rustfs_process_env_path(paths, track))?;
    let mut env = BTreeMap::new();

    for line in content.lines().filter(|line| !line.is_empty()) {
        let Some((key, value)) = line.split_once('=') else {
            bail!("invalid RustFS process env probe line `{line}`");
        };
        env.insert(key.to_string(), value.to_string());
    }

    Ok(env)
}

fn required_env_value(env: &BTreeMap<String, String>, key: &str) -> Result<String> {
    let Some(value) = env.get(key).filter(|value| !value.is_empty()) else {
        bail!("missing env value `{key}`");
    };

    Ok(value.clone())
}

fn assert_process_credentials_match_recorded_env(
    snapshot: &RustfsRuntimeCredentialSnapshot,
) -> Result<()> {
    assert_eq!(
        required_env_value(&snapshot.process_env, "RUSTFS_ACCESS_KEY")?,
        snapshot.recorded_access_key,
        "RustFS child process access key should match recorded resource env"
    );
    assert_eq!(
        required_env_value(&snapshot.process_env, "RUSTFS_SECRET_KEY")?,
        snapshot.recorded_secret_key,
        "RustFS child process secret key should match recorded resource env"
    );
    Ok(())
}

fn assert_runtime_metadata_omits_credentials(
    snapshot: &RustfsRuntimeCredentialSnapshot,
) -> Result<()> {
    let runtime_metadata = format!("{:#?}", snapshot.runtime_metadata);

    assert!(
        !runtime_metadata.contains(&snapshot.recorded_access_key),
        "RustFS runtime metadata should not contain the access key"
    );
    assert!(
        !runtime_metadata.contains(&snapshot.recorded_secret_key),
        "RustFS runtime metadata should not contain the secret key"
    );

    Ok(())
}

fn delete_optional_file(path: &Utf8Path) -> Result<()> {
    match state::fs::delete_file(path) {
        Ok(()) => Ok(()),
        Err(StateError::Filesystem { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
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

fn seed_postgres_fixture_artifact(paths: &PvPaths, track: &str) -> Result<()> {
    seed_postgres_fixture_artifact_with_script(paths, track, fake_postgres_script())
}

fn seed_unready_postgres_fixture_artifact(paths: &PvPaths, track: &str) -> Result<()> {
    seed_postgres_fixture_artifact_with_script(paths, track, unready_fake_postgres_script())
}

fn seed_postgres_fixture_artifact_with_script(
    paths: &PvPaths,
    track: &str,
    postgres_script: &str,
) -> Result<()> {
    let release_path = paths
        .resources()
        .join("postgres")
        .join(track)
        .join(format!("releases/{POSTGRES_ARTIFACT_VERSION}"));

    write_postgres_fixture_binaries_with_script(&release_path, postgres_script)?;
    let mut database = Database::open(paths)?;
    database.record_managed_resource_track_installed(
        "postgres",
        track,
        POSTGRES_ARTIFACT_VERSION,
        &release_path,
    )?;

    Ok(())
}

fn seed_postgres_cached_fixture(paths: &PvPaths, tempdir: &Utf8Path) -> Result<()> {
    let archive_path = tempdir.join(POSTGRES_ARCHIVE_FILE_NAME);

    create_postgres_archive(tempdir, &archive_path)?;
    seed_postgres_cached_fixture_from_archive(paths, &archive_path)
}

fn seed_postgres_cached_fixture_without_support_files(
    paths: &PvPaths,
    tempdir: &Utf8Path,
) -> Result<()> {
    let archive_path = tempdir.join(POSTGRES_ARCHIVE_FILE_NAME);

    create_postgres_archive_without_support_files(tempdir, &archive_path)?;
    seed_postgres_cached_fixture_from_archive(paths, &archive_path)
}

fn seed_postgres_cached_fixture_from_archive(
    paths: &PvPaths,
    archive_path: &Utf8Path,
) -> Result<()> {
    let sha256 = sha256_file(archive_path)?;
    let cache_path = paths
        .downloads()
        .join(format!("{sha256}-{POSTGRES_ARCHIVE_FILE_NAME}"));

    copy_file(archive_path, &cache_path)?;
    let size = file_size(&cache_path)?;
    let manifest = postgres_fixture_manifest(&sha256, size);

    state::fs::write_sensitive_file(&paths.downloads().join("manifest.json"), &manifest)?;

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

fn seed_rustfs_fixture_artifact(paths: &PvPaths, track: &str) -> Result<()> {
    seed_rustfs_fixture_artifact_with_script(paths, track, &rustfs_script())
}

fn seed_auth_rejecting_rustfs_fixture_artifact(paths: &PvPaths, track: &str) -> Result<()> {
    seed_rustfs_fixture_artifact_with_script(paths, track, &auth_rejecting_rustfs_script())
}

fn seed_rustfs_fixture_artifact_with_script(
    paths: &PvPaths,
    track: &str,
    script: &str,
) -> Result<()> {
    let release_path = paths
        .resources()
        .join("rustfs")
        .join(track)
        .join(format!("releases/{RUSTFS_ARTIFACT_VERSION}"));
    let executable = release_path.join("bin/rustfs");

    state::fs::write_sensitive_file(&executable, script)?;
    set_executable(&executable)?;
    let mut database = Database::open(paths)?;
    database.record_managed_resource_track_installed(
        "rustfs",
        track,
        RUSTFS_ARTIFACT_VERSION,
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

fn seed_rustfs_cached_fixture(paths: &PvPaths, tempdir: &Utf8Path) -> Result<()> {
    let archive_path = tempdir.join(RUSTFS_ARCHIVE_FILE_NAME);

    create_rustfs_archive(tempdir, &archive_path)?;
    let sha256 = sha256_file(&archive_path)?;
    let cache_path = paths
        .downloads()
        .join(format!("{sha256}-{RUSTFS_ARCHIVE_FILE_NAME}"));

    copy_file(&archive_path, &cache_path)?;
    let size = file_size(&cache_path)?;
    let manifest = rustfs_manifest(&sha256, size);

    state::fs::write_sensitive_file(&paths.downloads().join("manifest.json"), &manifest)?;

    Ok(())
}

fn seed_redis_runtime_port(paths: &PvPaths) -> Result<TcpListener> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();

    let mut database = Database::open(paths)?;
    database.assign_port(
        PortRequest::resource_port("redis", REDIS_TRACK, "redis", port, port, port),
        |candidate| candidate == port,
    )?;

    Ok(listener)
}

fn create_postgres_archive(tempdir: &Utf8Path, archive_path: &Utf8Path) -> Result<()> {
    let archive_parent = tempdir.join("postgres-archive-root");
    let root_name = format!("postgres-{POSTGRES_ARTIFACT_VERSION}");
    let root = archive_parent.join(&root_name);

    write_postgres_fixture_binaries(&root)?;
    create_archive(&archive_parent, archive_path, &root_name)
}

fn create_postgres_archive_without_support_files(
    tempdir: &Utf8Path,
    archive_path: &Utf8Path,
) -> Result<()> {
    let archive_parent = tempdir.join("postgres-archive-root-missing-support");
    let root_name = format!("postgres-{POSTGRES_ARTIFACT_VERSION}");
    let root = archive_parent.join(&root_name);

    write_postgres_fixture_binaries_without_support_files(&root)?;
    create_archive(&archive_parent, archive_path, &root_name)
}

fn create_archive(
    archive_parent: &Utf8Path,
    archive_path: &Utf8Path,
    root_name: &str,
) -> Result<()> {
    run_fixture_command(
        "/usr/bin/tar",
        &[
            "-czf",
            archive_path.as_str(),
            "-C",
            archive_parent.as_str(),
            root_name,
        ],
    )?;

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

fn create_rustfs_archive(tempdir: &Utf8Path, archive_path: &Utf8Path) -> Result<()> {
    let archive_parent = tempdir.join("rustfs-archive-root");
    let root_name = format!("rustfs-{RUSTFS_ARTIFACT_VERSION}");
    let root = archive_parent.join(&root_name);
    let executable = root.join("bin/rustfs");

    state::fs::write_sensitive_file(&executable, &rustfs_script())?;
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

fn write_postgres_fixture_binaries(release_path: &Utf8Path) -> Result<()> {
    write_postgres_fixture_binaries_with_script(release_path, fake_postgres_script())
}

fn write_postgres_fixture_binaries_with_script(
    release_path: &Utf8Path,
    postgres_script: &str,
) -> Result<()> {
    write_postgres_fixture_binaries_without_support_files(release_path)?;
    write_postgres_support_files(release_path)?;
    state::fs::write_sensitive_file(&release_path.join("bin/postgres"), postgres_script)?;
    set_executable(&release_path.join("bin/postgres"))?;

    Ok(())
}

fn write_postgres_fixture_binaries_without_support_files(release_path: &Utf8Path) -> Result<()> {
    let initdb = release_path.join("bin/initdb");
    let postgres = release_path.join("bin/postgres");

    state::fs::write_sensitive_file(&initdb, fake_postgres_initdb_script())?;
    state::fs::write_sensitive_file(&postgres, fake_postgres_script())?;
    set_executable(&initdb)?;
    set_executable(&postgres)?;

    Ok(())
}

fn write_postgres_support_files(release_path: &Utf8Path) -> Result<()> {
    state::fs::write_sensitive_file(
        &release_path.join("share/postgresql/postgres.bki"),
        "postgres catalog",
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

fn rustfs_manifest(sha256: &str, size: u64) -> String {
    let dummy_sha256 = "0000000000000000000000000000000000000000000000000000000000000000";

    format!(
        r#"{{
  "schema_version": 1,
  "minimum_pv_version": "0.1.0",
  "resources": [
    {{
      "name": "rustfs",
      "default_track": "{RUSTFS_TRACK}",
      "tracks": [
        {{
          "name": "{RUSTFS_TRACK}",
          "artifacts": [
            {{
              "artifact_version": "{RUSTFS_ARTIFACT_VERSION}",
              "upstream_version": "1.0.0",
              "pv_build_revision": "1",
              "platform": "any",
              "url": "https://artifacts.example.test/{RUSTFS_ARCHIVE_FILE_NAME}",
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

fn postgres_fixture_manifest(sha256: &str, size: u64) -> String {
    let dummy_sha256 = "0000000000000000000000000000000000000000000000000000000000000000";

    format!(
        r#"{{
  "schema_version": 1,
  "minimum_pv_version": "0.1.0",
  "resources": [
    {{
      "name": "postgres",
      "default_track": "{POSTGRES_TRACK}",
      "tracks": [
        {{
          "name": "{POSTGRES_TRACK}",
          "artifacts": [
            {{
              "artifact_version": "{POSTGRES_ARTIFACT_VERSION}",
              "upstream_version": "16.0",
              "pv_build_revision": "1",
              "platform": "any",
              "url": "https://artifacts.example.test/{POSTGRES_ARCHIVE_FILE_NAME}",
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

fn fake_postgres_initdb_script() -> &'static str {
    r#"#!/bin/sh
set -eu

data_dir=""
username=""
password_file=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    -D)
      data_dir="$2"
      shift 2
      ;;
    -U)
      username="$2"
      shift 2
      ;;
    --username)
      username="$2"
      shift 2
      ;;
    --pwfile)
      password_file="$2"
      shift 2
      ;;
    --auth-host|--auth-local)
      shift 2
      ;;
    *)
      echo "unexpected initdb argument: $1" >&2
      exit 64
      ;;
  esac
done

if [ -z "$data_dir" ] || [ -z "$username" ] || [ -z "$password_file" ]; then
  echo "missing initdb inputs" >&2
  exit 64
fi

if [ -d "$data_dir" ] && [ "$(find "$data_dir" -mindepth 1 -maxdepth 1 | wc -l)" -gt 0 ]; then
  echo "PGDATA is not empty before initdb" >&2
  exit 65
fi

mkdir -p "$data_dir/databases"
printf '16\n' > "$data_dir/PG_VERSION"
printf '%s\n' "$username" > "$data_dir/initdb.username"
cat "$password_file" > "$data_dir/initdb.password"
"#
}

fn fake_postgres_script() -> &'static str {
    r#"#!/bin/sh
set -eu

data_dir=""
argument_host=""
argument_port=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    -D)
      data_dir="$2"
      shift 2
      ;;
    -h)
      argument_host="$2"
      shift 2
      ;;
    -p)
      argument_port="$2"
      shift 2
      ;;
    *)
      echo "unexpected postgres argument: $1" >&2
      exit 64
      ;;
  esac
done

if [ -z "$data_dir" ] || [ -z "$argument_host" ] || [ -z "$argument_port" ] || [ ! -f "$data_dir/PG_VERSION" ]; then
  echo "postgres data dir is not initialized" >&2
  exit 64
fi

python3 - "$data_dir" "$argument_host" "$argument_port" <<'PY'
import os
import signal
import socketserver
import struct
import sys
import threading

data_dir = sys.argv[1]
argument_host = sys.argv[2]
argument_port = int(sys.argv[3])
config_path = os.path.join(data_dir, "postgresql.conf")
database_dir = os.path.join(data_dir, "databases")

host = "127.0.0.1"
port = None

with open(config_path, "r", encoding="utf-8") as config:
    for line in config:
        line = line.strip()
        if line.startswith("listen_addresses"):
            host = line.split("=", 1)[1].strip().strip("'\"")
        if line.startswith("port"):
            port = int(line.split("=", 1)[1].strip())

if host != "127.0.0.1" or port is None:
    raise SystemExit("postgresql.conf did not set loopback host and port")
if argument_host != host or argument_port != port:
    raise SystemExit("postgres arguments did not match generated config")

os.makedirs(database_dir, exist_ok=True)
with open(os.path.join(data_dir, "postgres.started"), "w", encoding="utf-8") as started:
    started.write(f"{host}:{port}\n")

def packet(message_type, payload=b""):
    return message_type + struct.pack("!I", len(payload) + 4) + payload

def auth_ok():
    return packet(b"R", struct.pack("!I", 0))

def parameter_status(key, value):
    return packet(b"S", key.encode() + b"\0" + value.encode() + b"\0")

def backend_key_data():
    return packet(b"K", struct.pack("!II", os.getpid() & 0x7fffffff, 1))

def ready():
    return packet(b"Z", b"I")

def parameter_description(query):
    if "$1" in query:
        return packet(b"t", struct.pack("!H", 1) + struct.pack("!I", 25))
    return packet(b"t", struct.pack("!H", 0))

def command_complete(tag):
    return packet(b"C", tag.encode() + b"\0")

def parse_complete():
    return packet(b"1")

def bind_complete():
    return packet(b"2")

def close_complete():
    return packet(b"3")

def no_data():
    return packet(b"n")

def row_description():
    field = b"?column?\0" + struct.pack("!IhIhih", 0, 0, 23, 4, -1, 0)
    return packet(b"T", struct.pack("!H", 1) + field)

def data_row(value):
    data = str(value).encode()
    return packet(b"D", struct.pack("!H", 1) + struct.pack("!I", len(data)) + data)

def error_response(message):
    return packet(b"E", b"SERROR\0CXX000\0M" + message.encode() + b"\0\0")

def cstring(payload, start):
    end = payload.index(b"\0", start)
    return payload[start:end].decode(), end + 1

def read_exact(stream, length):
    data = b""
    while len(data) < length:
        chunk = stream.recv(length - len(data))
        if not chunk:
            raise EOFError
        data += chunk
    return data

def read_startup(stream):
    length = struct.unpack("!I", read_exact(stream, 4))[0]
    payload = read_exact(stream, length - 4)
    code = struct.unpack("!I", payload[:4])[0]
    if code == 80877103:
        stream.sendall(b"N")
        return read_startup(stream)
    return payload

def startup_response():
    return b"".join([
        auth_ok(),
        parameter_status("server_version", "16.0"),
        parameter_status("server_encoding", "UTF8"),
        parameter_status("client_encoding", "UTF8"),
        parameter_status("DateStyle", "ISO, MDY"),
        parameter_status("integer_datetimes", "on"),
        parameter_status("standard_conforming_strings", "on"),
        backend_key_data(),
        ready(),
    ])

def database_file(database):
    safe = "".join(ch for ch in database if ch.isalnum() or ch == "_")
    if safe != database:
        raise ValueError("unsafe database name")
    return os.path.join(database_dir, database)

def database_exists(database):
    return os.path.exists(database_file(database))

def create_database(database):
    with open(database_file(database), "w", encoding="utf-8") as marker:
        marker.write(database + "\n")

def database_from_create(query):
    quoted = query.split("CREATE DATABASE", 1)[1].strip()
    if quoted.startswith('"') and quoted.endswith('"'):
        return quoted[1:-1]
    return quoted

def query_response(query, params):
    normalized = " ".join(query.strip().split())
    if normalized.upper() in {"SELECT 1", "SELECT $1"}:
        return row_description() + data_row(1) + command_complete("SELECT 1")
    if "FROM pg_database WHERE datname" in normalized:
        database = params[0] if params else ""
        if database_exists(database):
            return row_description() + data_row(1) + command_complete("SELECT 1")
        return row_description() + command_complete("SELECT 0")
    if normalized.upper().startswith("CREATE DATABASE"):
        create_database(database_from_create(normalized))
        return command_complete("CREATE DATABASE")
    if normalized.upper().startswith("SET "):
        return command_complete("SET")
    return error_response("unsupported fixture query: " + normalized)

class Handler(socketserver.BaseRequestHandler):
    def handle(self):
        statements = {}
        portals = {}
        try:
            read_startup(self.request)
            self.request.sendall(startup_response())
            while True:
                message_type = read_exact(self.request, 1)
                length = struct.unpack("!I", read_exact(self.request, 4))[0]
                payload = read_exact(self.request, length - 4)
                if message_type == b"X":
                    return
                if message_type == b"Q":
                    query = payload[:-1].decode()
                    self.request.sendall(query_response(query, []) + ready())
                    continue
                if message_type == b"P":
                    statement, offset = cstring(payload, 0)
                    query, _offset = cstring(payload, offset)
                    statements[statement] = query
                    self.request.sendall(parse_complete())
                    continue
                if message_type == b"B":
                    portal, offset = cstring(payload, 0)
                    statement, offset = cstring(payload, offset)
                    format_count = struct.unpack("!H", payload[offset:offset + 2])[0]
                    offset += 2 + (format_count * 2)
                    param_count = struct.unpack("!H", payload[offset:offset + 2])[0]
                    offset += 2
                    params = []
                    for _index in range(param_count):
                        size = struct.unpack("!i", payload[offset:offset + 4])[0]
                        offset += 4
                        if size == -1:
                            params.append(None)
                        else:
                            params.append(payload[offset:offset + size].decode())
                            offset += size
                    portals[portal] = (statements.get(statement, ""), params)
                    self.request.sendall(bind_complete())
                    continue
                if message_type == b"D":
                    describe_kind = payload[:1]
                    name = payload[1:-1].decode()
                    query, _params = portals.get(name, (statements.get(name, ""), []))
                    response = b""
                    if describe_kind == b"S":
                        response += parameter_description(query)
                    if query.strip().upper().startswith("CREATE DATABASE"):
                        response += no_data()
                    else:
                        response += row_description()
                    self.request.sendall(response)
                    continue
                if message_type == b"E":
                    portal, offset = cstring(payload, 0)
                    _max_rows = struct.unpack("!I", payload[offset:offset + 4])[0]
                    query, params = portals.get(portal, ("", []))
                    self.request.sendall(query_response(query, params))
                    continue
                if message_type == b"S":
                    self.request.sendall(ready())
                    continue
                if message_type == b"H":
                    continue
                if message_type == b"C":
                    self.request.sendall(close_complete())
                    continue
                self.request.sendall(error_response("unsupported message type"))
        except (EOFError, ConnectionResetError, BrokenPipeError):
            return

class Server(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True

server = Server((host, port), Handler)

def stop(_signum, _frame):
    server.shutdown()

signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)

threading.Thread(target=server.serve_forever, daemon=True).start()
signal.pause()
PY
"#
}

fn unready_fake_postgres_script() -> &'static str {
    r#"#!/bin/sh
set -eu

data_dir=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    -D)
      data_dir="$2"
      shift 2
      ;;
    -h|-p)
      shift 2
      ;;
    *)
      echo "unexpected postgres argument: $1" >&2
      exit 64
      ;;
  esac
done

if [ -z "$data_dir" ] || [ ! -f "$data_dir/PG_VERSION" ]; then
  echo "postgres data dir is not initialized" >&2
  exit 64
fi

stop() {
  exit 0
}

trap stop TERM INT

while true; do
  sleep 1
done
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
    artifact_adapter: RuntimeArtifactAdapter,
    hook_events: Arc<Mutex<Vec<String>>>,
}

impl AsyncSqlHookRuntimeAdapter {
    fn new(hook_events: Arc<Mutex<Vec<String>>>) -> Result<Self> {
        Ok(Self {
            artifact_adapter: RuntimeArtifactAdapter::new(
                ResourceName::new("mysql")?,
                "bin/pv-fake-sql",
            ),
            hook_events,
        })
    }
}

impl super::ManagedResourceRuntimeAdapter for AsyncSqlHookRuntimeAdapter {
    fn resource_name(&self) -> &'static str {
        "mysql"
    }

    fn artifact_adapter(&self) -> Result<RuntimeArtifactAdapter, crate::DaemonError> {
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
        let track_env_persisted = Database::open(paths)?
            .managed_resource_track(&context.resource_name, &context.track)?
            .env
            .contains_key("host");
        let event = if track_env_persisted {
            "build_process_spec:persisted_env"
        } else {
            "build_process_spec:missing_env"
        };

        push_hook_event(&self.hook_events, event)?;
        state::fs::write_sensitive_file(&config_path, "{}")?;

        Ok(crate::ProcessSpec {
            name: format!("{}-{}", context.resource_name, context.track),
            command: self
                .artifact_adapter
                .executable_path(&context.artifact_path),
            arguments: Vec::new(),
            private_environment: BTreeMap::new(),
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
        _resource_env: &'a EnvContextValues,
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

python3 - "$@" <<'PY'
import os
import signal
import shlex
import socketserver
import sys

def redis_config(argv):
    port = None
    data_dir = None
    args = list(argv)
    while args:
        arg = args.pop(0)
        if arg == "--port" and args:
            port = int(args.pop(0))
        elif arg == "--dir" and args:
            data_dir = args.pop(0)
        elif os.path.isfile(arg):
            with open(arg, "r", encoding="utf-8") as config:
                for line in config:
                    parts = shlex.split(line)
                    if len(parts) == 2 and parts[0] == "port":
                        port = int(parts[1])
                    elif len(parts) == 2 and parts[0] == "dir":
                        data_dir = parts[1]
    if port is None:
        raise RuntimeError("missing Redis port")
    return port, data_dir

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

port, data_dir = redis_config(sys.argv[1:])
if data_dir:
    os.makedirs(data_dir, exist_ok=True)

server = RedisServer(("127.0.0.1", port), RedisPingHandler)
signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)
server.serve_forever()
PY
"#
}

fn rustfs_script() -> String {
    rustfs_script_source("False")
}

fn auth_rejecting_rustfs_script() -> String {
    rustfs_script_source("True")
}

fn rustfs_script_source(reject_s3: &str) -> String {
    r#"#!/bin/sh
set -eu

address=""
console_address=""
data_dir=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    --address)
      address="$2"
      shift 2
      ;;
    --console-address)
      console_address="$2"
      shift 2
      ;;
    *)
      data_dir="$1"
      shift
      ;;
  esac
done

python3 - "$address" "$console_address" "$data_dir" <<'PY'
import hashlib
import http.server
import os
import posixpath
import signal
import sys
import threading
import urllib.parse

api_address = sys.argv[1]
console_address = sys.argv[2]
data_dir = sys.argv[3]
reject_s3 = __PV_REJECT_S3__
buckets_dir = os.path.join(data_dir, "buckets")
os.makedirs(buckets_dir, exist_ok=True)
with open(os.path.join(data_dir, "process-env"), "w", encoding="utf-8") as file:
    file.write(f"RUSTFS_ACCESS_KEY={os.environ.get('RUSTFS_ACCESS_KEY', '')}\n")
    file.write(f"RUSTFS_SECRET_KEY={os.environ.get('RUSTFS_SECRET_KEY', '')}\n")

def split_address(value):
    host, port = value.rsplit(":", 1)
    return host, int(port)

def bucket_path(bucket):
    return os.path.join(buckets_dir, bucket)

def object_path(bucket, key):
    clean_key = posixpath.normpath("/" + key).lstrip("/")
    return os.path.join(bucket_path(bucket), clean_key)

class RustfsHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        path = urllib.parse.urlparse(self.path).path
        if path == "/":
            self.send_response(200)
            self.end_headers()
            self.wfile.write(b"rustfs")
            return

        self.send_response(404)
        self.end_headers()

    def do_PUT(self):
        if reject_s3:
            self.send_response(403)
            self.end_headers()
            return

        path = urllib.parse.urlparse(self.path).path.strip("/")
        parts = path.split("/", 1)
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length)
        if not parts or not parts[0]:
            self.send_response(400)
            self.end_headers()
            return

        bucket = parts[0]
        if len(parts) == 1:
            os.makedirs(bucket_path(bucket), exist_ok=True)
            self.send_response(200)
            self.end_headers()
            return

        target = object_path(bucket, parts[1])
        os.makedirs(os.path.dirname(target), exist_ok=True)
        with open(target, "wb") as file:
            file.write(body)
        self.send_response(200)
        self.send_header("ETag", hashlib.md5(body).hexdigest())
        self.end_headers()

    def do_HEAD(self):
        if reject_s3:
            self.send_response(403)
            self.end_headers()
            return

        path = urllib.parse.urlparse(self.path).path.strip("/")
        parts = path.split("/", 1)
        if len(parts) != 2:
            exists = bool(parts and parts[0] and os.path.isdir(bucket_path(parts[0])))
            self.send_response(200 if exists else 404)
            self.end_headers()
            return

        target = object_path(parts[0], parts[1])
        if not os.path.exists(target):
            self.send_response(404)
            self.end_headers()
            return

        size = os.path.getsize(target)
        with open(target, "rb") as file:
            digest = hashlib.md5(file.read()).hexdigest()
        self.send_response(200)
        self.send_header("Content-Length", str(size))
        self.send_header("ETag", digest)
        self.end_headers()

    def log_message(self, _format, *_args):
        return

class ConsoleHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"rustfs console")

    def log_message(self, _format, *_args):
        return

class Server(http.server.ThreadingHTTPServer):
    allow_reuse_address = True

api = Server(split_address(api_address), RustfsHandler)
console = Server(split_address(console_address), ConsoleHandler)

def stop(_signum, _frame):
    api.shutdown()
    console.shutdown()
    sys.exit(0)

signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)

threading.Thread(target=console.serve_forever, daemon=True).start()
api.serve_forever()
PY
"#
    .replace("__PV_REJECT_S3__", reject_s3)
}

fn assert_with_normalized_postgres_runtime(
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
    settings.add_filter(r"[0-9a-f]{32}", "<postgres_password>");
    settings.add_filter(
        r"postgres://pv_root:<postgres_password>@127\.0\.0\.1:\d+",
        "postgres://pv_root:<postgres_password>@127.0.0.1:<postgres_port>",
    );
    settings.add_filter(r"DB_PORT=\d+", "DB_PORT=<postgres_port>");
    settings.add_filter(r"PGPORT=\d+", "PGPORT=<postgres_port>");
    settings.add_filter(r#""port": "\d+""#, r#""port": "<postgres_port>""#);
    settings.add_filter(r"port: \d+", "port: <postgres_port>");
    settings.bind(|| {
        assert_debug_snapshot!(name, snapshot);
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
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
    settings.add_filter(
        r"S3_ENDPOINT=http://127\.0\.0\.1:\d+",
        "S3_ENDPOINT=http://127.0.0.1:<api_port>",
    );
    settings.add_filter(
        r"AWS_ENDPOINT=http://127\.0\.0\.1:\d+",
        "AWS_ENDPOINT=http://127.0.0.1:<api_port>",
    );
    settings.add_filter(
        r"AWS_URL=http://127\.0\.0\.1:\d+",
        "AWS_URL=http://127.0.0.1:<api_port>",
    );
    settings.add_filter(
        r"(S3_SECRET_KEY|AWS_SECRET_ACCESS_KEY)=[0-9a-f]{32}",
        "$1=<secret_key>",
    );
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
        r#""RUSTFS_SECRET_KEY": "[0-9a-f]{32}""#,
        r#""RUSTFS_SECRET_KEY": "<secret_key>""#,
    );
    settings.add_filter(
        r#"recorded_secret_key: "[0-9a-f]{32}""#,
        r#"recorded_secret_key: "<secret_key>""#,
    );
    settings.add_filter(
        r#""dashboard_url": "http://127\.0\.0\.1:\d+""#,
        r#""dashboard_url": "http://127.0.0.1:<dashboard_port>""#,
    );
    settings.add_filter(
        r#""endpoint": "http://127\.0\.0\.1:\d+""#,
        r#""endpoint": "http://127.0.0.1:<api_port>""#,
    );
    settings.add_filter(
        r#""url": "http://127\.0\.0\.1:\d+""#,
        r#""url": "http://127.0.0.1:<api_port>""#,
    );
    settings.add_filter(
        r#""secret_key": "[0-9a-f]{32}""#,
        r#""secret_key": "<secret_key>""#,
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
    settings.add_filter(r"port \d+", "port <redis_port>");
    if name.starts_with("redis_") {
        settings.add_filter(r#""port": "\d+""#, r#""port": "<redis_port>""#);
    } else if name.starts_with("rustfs_") {
        settings.add_filter(r#""port": "\d+""#, r#""port": "<api_port>""#);
    } else {
        settings.add_filter(r#""port": "\d+""#, r#""port": "<port>""#);
    }
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
    settings.add_filter(r"http:127\.0\.0\.1:\d+/", "http:127.0.0.1:<api_port>/");
    settings.add_filter(r"http://127\.0\.0\.1:\d+/", "http://127.0.0.1:<api_port>/");
    settings.add_filter(r"127\.0\.0\.1:\d+", "127.0.0.1:<port>");
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
    assert_runtime_status_for_resource(states, "mailpit", track, status);
}

fn assert_runtime_status_for_resource(
    states: &[state::RuntimeObservedStateRecord],
    resource_name: &str,
    track: &str,
    status: RuntimeObservedStatus,
) {
    let found = runtime_has_status_for_resource(states, resource_name, track, status);
    assert!(
        found,
        "expected {resource_name} track {track:?} runtime status {status:?}, got {states:#?}"
    );
}

fn runtime_has_status(
    states: &[state::RuntimeObservedStateRecord],
    track: &str,
    status: RuntimeObservedStatus,
) -> bool {
    runtime_has_status_for_resource(states, "mailpit", track, status)
}

fn runtime_has_status_for_resource(
    states: &[state::RuntimeObservedStateRecord],
    resource_name: &str,
    track: &str,
    status: RuntimeObservedStatus,
) -> bool {
    let expected_subject = RuntimeSubject::Resource {
        name: resource_name.to_string(),
        track: track.to_string(),
    };

    states
        .iter()
        .any(|record| record.subject == expected_subject && record.status == status)
}

fn runtime_files_exist(paths: &PvPaths, track: &str) -> Result<RuntimeFilePresence> {
    runtime_files_exist_for_resource(paths, "mailpit", track)
}

fn runtime_files_exist_for_resource(
    paths: &PvPaths,
    resource_name: &str,
    track: &str,
) -> Result<RuntimeFilePresence> {
    Ok(RuntimeFilePresence {
        pid: path_exists(&paths.resource_pid(resource_name, track))?,
        metadata: path_exists(&paths.resource_runtime_metadata(resource_name, track))?,
        config: path_exists(&paths.resource_runtime_config(resource_name, track))?,
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
    artifact_adapter: RuntimeArtifactAdapter,
}

impl ManagedResourceRuntimeAdapter for InvalidDefaultPortRuntimeAdapter {
    fn resource_name(&self) -> &'static str {
        "mailpit"
    }

    fn artifact_adapter(&self) -> Result<RuntimeArtifactAdapter, DaemonError> {
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
