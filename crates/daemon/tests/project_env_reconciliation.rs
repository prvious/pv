use std::collections::BTreeMap;
use std::io;
use std::net::{Ipv4Addr, TcpListener, UdpSocket};
use std::os::unix::fs::PermissionsExt;

use anyhow::{Result, anyhow};
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::{Settings, assert_debug_snapshot};
use platform::GeneratedLocalCa;
use rcgen::{
    CertificateParams, DnType, ExtendedKeyUsagePurpose, Issuer, KeyPair, KeyUsagePurpose,
    PKCS_ECDSA_P256_SHA256,
};
use serde_json::{Value, json};
use state::fs::write_sensitive_file;
use state::{
    Database, EnvContextValues, JobRecord, LinkProjectInput, PortOwner, PortRequest,
    ProjectEnvObservedStatus, ProjectManagedResourceInput, ProjectMode, ProjectRecord, PvPaths,
    ResourceAllocationInput, StateError,
};
use time::{Duration, OffsetDateTime};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

#[tokio::test]
async fn resource_only_project_uses_custom_env_file_and_no_php_worker() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project_path = tempdir.path().join("Resource Project");
    state::fs::ensure_user_dir(&project_path.join("config"))?;
    let project = link_resource_only_project(
        &paths,
        &project_path,
        r#"serve: false
env_file: config/development.env
php: "8.4"
env:
  APP_NAME: resource-project
  APP_URL: "${project_url}"
"#,
    )?;

    let lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;
    let reconciled = database
        .project_by_id(&project.id)?
        .ok_or_else(|| anyhow!("expected reconciled resource-only Project"))?;
    let runtime_plan = daemon::gateway::build_runtime_plan(&paths)?;
    let custom_env = state::fs::read_to_string(&project.path.join("config/development.env"))?;

    assert!(runtime_plan.workers.is_empty());
    assert!(!state::fs::path_entry_exists(&project.path.join(".env"))?);
    assert!(!state::fs::path_entry_exists(
        &paths.project_tls_certificate(&project.id)
    )?);
    assert_eq!(reconciled.mode, ProjectMode::ResourceOnly);
    assert_eq!(reconciled.primary_hostname, None);
    assert_eq!(reconciled.php_runtime.track.as_deref(), Some("8.4"));
    assert_eq!(
        custom_env,
        "# >>> PV MANAGED\nAPP_NAME=resource-project\n# <<< PV MANAGED\n"
    );

    assert_with_normalized_timestamps_and_tempdir(
        "resource_only_project_uses_custom_env_file_and_no_php_worker",
        (
            lines,
            reconciled,
            database.project_env_observed_state(&project.id)?,
        ),
        tempdir.path(),
    )?;

    Ok(())
}

#[tokio::test]
async fn invalid_resource_only_transition_preserves_served_mode() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        "env:\n  APP_NAME: acme\n",
    )?;
    write_project_config(
        &project,
        r#"serve: false
postgres:
  version: "8.0"
  allocations:
    analytics:
      env:
        DATABASE_URL: "postgres://${database}"
    app:
      env:
        DATABASE_URL: "postgres://${database}"
"#,
    )?;

    let lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;
    let reconciled = database
        .project_by_id(&project.id)?
        .ok_or_else(|| anyhow!("expected failed transition to preserve Project state"))?;

    assert_eq!(reconciled.mode, ProjectMode::Served);
    assert_eq!(reconciled.primary_hostname.as_deref(), Some("acme.test"));
    assert_with_normalized_timestamps_and_tempdir(
        "invalid_resource_only_transition_preserves_served_mode",
        (
            lines,
            reconciled,
            database.project_env_observed_state(&project.id)?,
            latest_job(&database, &format!("project:{}", project.id))?,
        ),
        tempdir.path(),
    )?;

    Ok(())
}

#[tokio::test]
async fn failed_resource_only_transition_after_preflight_preserves_served_mode() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(&paths, &tempdir.path().join("project"), "acme.test", "")?;
    write_project_config(
        &project,
        r#"serve: false
postgres:
  version: "8.0"
  allocations:
    app-db:
      env:
        DB_DATABASE: "${database}"
"#,
    )?;

    let lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;
    let reconciled = database
        .project_by_id(&project.id)?
        .ok_or_else(|| anyhow!("expected failed transition to preserve Project state"))?;
    let job = latest_job(&database, &format!("project:{}", project.id))?;

    assert!(!lines.is_empty());
    assert_eq!(job.status, state::JobStatus::Failed);
    assert_eq!(reconciled.mode, ProjectMode::Served);
    assert_eq!(reconciled.primary_hostname.as_deref(), Some("acme.test"));

    Ok(())
}

#[tokio::test]
async fn failed_served_transition_after_preflight_preserves_resource_only_mode() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project =
        link_resource_only_project(&paths, &tempdir.path().join("project"), "serve: false\n")?;
    write_project_config(
        &project,
        r#"postgres:
  version: "8.0"
  allocations:
    app-db:
      env:
        DB_DATABASE: "${database}"
"#,
    )?;

    let lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;
    let reconciled = database
        .project_by_id(&project.id)?
        .ok_or_else(|| anyhow!("expected failed transition to preserve Project state"))?;
    let job = latest_job(&database, &format!("project:{}", project.id))?;

    assert!(!lines.is_empty());
    assert_eq!(job.status, state::JobStatus::Failed);
    assert_eq!(reconciled.mode, ProjectMode::ResourceOnly);
    assert_eq!(reconciled.primary_hostname, None);

    Ok(())
}

#[tokio::test]
async fn failed_transition_after_php_resolution_preserves_served_runtime() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        "php: \"8.4\"\n",
    )?;
    run_project_reconciliation(&paths, &project).await?;
    let locked_directory = project.path.join("locked");
    state::fs::ensure_user_dir(&locked_directory)?;
    set_file_mode(&locked_directory, 0o500)?;
    write_project_config(
        &project,
        r#"serve: false
php: "8.3"
env_file: locked/.env
env:
  APP_NAME: acme
"#,
    )?;

    let lines = run_project_reconciliation(&paths, &project).await?;
    set_file_mode(&locked_directory, 0o700)?;
    let database = Database::open(&paths)?;
    let reconciled = database
        .project_by_id(&project.id)?
        .ok_or_else(|| anyhow!("expected failed transition to preserve Project state"))?;
    let job = latest_job(&database, &format!("project:{}", project.id))?;

    assert!(!lines.is_empty());
    assert_eq!(job.status, state::JobStatus::Failed);
    assert_eq!(reconciled.mode, ProjectMode::Served);
    assert_eq!(reconciled.php_runtime.track.as_deref(), Some("8.4"));

    Ok(())
}

#[tokio::test]
async fn root_only_env_rendering_writes_dotenv_and_records_rendered_state() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"env:
  APP_URL: "${project_url}"
  APP_NAME: acme
  VITE_DEV_SERVER_KEY: "${tls_key}"
  VITE_DEV_SERVER_CERT: "${tls_cert}"
  PV_TLS_CA: "${tls_ca}"
"#,
    )?;
    let local_ca = seed_local_ca(&paths)?;
    write_project_certificate_with_remaining_days(&paths, &project, &local_ca, 365)?;

    let lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;
    let (certificate_pem, private_key_pem) = read_project_tls_files(&paths, &project)?;
    let project_scope = format!("project:{}", project.id);
    let project_jobs = database
        .recent_jobs()?
        .into_iter()
        .filter(|job| job.scope == project_scope)
        .collect::<Vec<_>>();

    assert_project_certificate_matches(&certificate_pem, &private_key_pem, "acme.test", &local_ca);

    assert_with_normalized_timestamps_and_tempdir(
        "root_only_env_rendering_writes_dotenv_and_records_rendered_state",
        (
            lines,
            read_dotenv(&project)?,
            database.project_env_observed_state(&project.id)?,
            project_jobs,
        ),
        tempdir.path(),
    )?;

    Ok(())
}

#[tokio::test]
async fn changing_env_file_and_unlinking_leave_previous_managed_blocks_untouched() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        "env:\n  APP_NAME: first\n",
    )?;

    run_project_reconciliation(&paths, &project).await?;
    let original_env = read_dotenv(&project)?;
    write_project_config(&project, "env_file: .env.local\nenv:\n  APP_NAME: second\n")?;
    run_project_reconciliation(&paths, &project).await?;
    let alternate_env = state::fs::read_to_string(&project.path.join(".env.local"))?;
    let mut database = Database::open(&paths)?;
    database.unlink_project(&project.id)?;

    assert_eq!(read_dotenv(&project)?, original_env);
    assert_eq!(
        state::fs::read_to_string(&project.path.join(".env.local"))?,
        alternate_env
    );
    assert_eq!(
        original_env,
        "# >>> PV MANAGED\nAPP_NAME=first\n# <<< PV MANAGED\n"
    );
    assert_eq!(
        alternate_env,
        "# >>> PV MANAGED\nAPP_NAME=second\n# <<< PV MANAGED\n"
    );

    Ok(())
}

#[tokio::test]
async fn tls_placeholders_generate_and_refresh_primary_hostname_certificate() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"env:
  VITE_DEV_SERVER_KEY: "${tls_key}"
  VITE_DEV_SERVER_CERT: "${tls_cert}"
"#,
    )?;
    let local_ca = seed_local_ca(&paths)?;

    run_project_reconciliation(&paths, &project).await?;
    let (initial_certificate_pem, initial_private_key_pem) =
        read_project_tls_files(&paths, &project)?;

    assert_eq!(certificate_pem_block_count(&initial_certificate_pem), 2);
    assert_project_certificate_matches(
        &initial_certificate_pem,
        &initial_private_key_pem,
        "acme.test",
        &local_ca,
    );

    run_project_reconciliation(&paths, &project).await?;
    let (rerun_certificate_pem, rerun_private_key_pem) = read_project_tls_files(&paths, &project)?;
    assert_eq!(
        (
            initial_certificate_pem.as_str(),
            initial_private_key_pem.as_str()
        ),
        (
            rerun_certificate_pem.as_str(),
            rerun_private_key_pem.as_str()
        ),
        "unchanged TLS placeholders should not rewrite Project cert files"
    );

    let renamed_project = update_project_primary_hostname(&paths, &project, "renamed.test")?;
    run_project_reconciliation(&paths, &renamed_project).await?;
    let (renamed_certificate_pem, renamed_private_key_pem) =
        read_project_tls_files(&paths, &renamed_project)?;

    assert_ne!(
        renamed_certificate_pem, initial_certificate_pem,
        "primary hostname changes should refresh the Project certificate"
    );
    assert_project_certificate_matches(
        &renamed_certificate_pem,
        &renamed_private_key_pem,
        "renamed.test",
        &local_ca,
    );
    assert!(!platform::project_certificate_matches(
        &renamed_certificate_pem,
        &renamed_private_key_pem,
        "acme.test",
        &local_ca.certificate_pem
    ));

    Ok(())
}

#[tokio::test]
async fn disabling_serving_retains_tls_files_without_refreshing_them() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        "env:\n  VITE_DEV_SERVER_CERT: \"${tls_cert}\"\n",
    )?;
    let local_ca = seed_local_ca(&paths)?;
    write_project_certificate_with_remaining_days(&paths, &project, &local_ca, 7)?;
    let certificate_before =
        state::fs::read_to_string(&paths.project_tls_certificate(&project.id))?;
    let private_key_before =
        state::fs::read_to_string(&paths.project_tls_private_key(&project.id))?;
    write_project_config(
        &project,
        "serve: false\nenv:\n  VITE_DEV_SERVER_CERT: \"${tls_cert}\"\n",
    )?;

    run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;
    let reconciled = database
        .project_by_id(&project.id)?
        .ok_or_else(|| anyhow!("expected resource-only Project after disabling serving"))?;

    assert_eq!(reconciled.mode, ProjectMode::ResourceOnly);
    assert_eq!(
        state::fs::read_to_string(&paths.project_tls_certificate(&project.id))?,
        certificate_before
    );
    assert_eq!(
        state::fs::read_to_string(&paths.project_tls_private_key(&project.id))?,
        private_key_before
    );
    assert_eq!(read_optional_dotenv(&project)?, None);

    Ok(())
}

#[tokio::test]
async fn serving_transition_clears_and_restores_all_dormant_env_entries() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        "env:\n  APP_URL: \"${project_url}\"\n",
    )?;
    write_sensitive_file(&project.path.join(".env"), "USER_VALUE=kept\n")?;

    run_project_reconciliation(&paths, &project).await?;
    assert_eq!(
        read_dotenv(&project)?,
        "USER_VALUE=kept\n# >>> PV MANAGED\nAPP_URL=https://acme.test\n# <<< PV MANAGED\n"
    );

    write_project_config(
        &project,
        "serve: false\nenv:\n  APP_URL: \"${project_url}\"\n",
    )?;
    run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;
    let resource_only = database
        .project_by_id(&project.id)?
        .ok_or_else(|| anyhow!("expected resource-only Project"))?;
    assert_eq!(resource_only.mode, ProjectMode::ResourceOnly);
    assert_eq!(
        read_dotenv(&project)?,
        "USER_VALUE=kept\n# >>> PV MANAGED\n# <<< PV MANAGED\n"
    );
    drop(database);

    write_project_config(&project, "env:\n  APP_URL: \"${project_url}\"\n")?;
    run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;
    let served = database
        .project_by_id(&project.id)?
        .ok_or_else(|| anyhow!("expected served Project"))?;
    assert_eq!(served.mode, ProjectMode::Served);
    assert_eq!(
        read_dotenv(&project)?,
        "USER_VALUE=kept\n# >>> PV MANAGED\nAPP_URL=https://acme.test\n# <<< PV MANAGED\n"
    );

    Ok(())
}

#[tokio::test]
async fn project_tls_reconciliation_replaces_overlong_leaf_once() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"env:
  PV_TLS_CA: "${tls_ca}"
  VITE_DEV_SERVER_CERT: "${tls_cert}"
  VITE_DEV_SERVER_KEY: "${tls_key}"
"#,
    )?;
    let local_ca = seed_local_ca(&paths)?;
    let ca_certificate_before = state::fs::read_to_string(&paths.ca_certificate())?;
    let ca_private_key_before = state::fs::read_to_string(&paths.ca_private_key())?;
    let certificate_path = paths.project_tls_certificate(&project.id);
    let private_key_path = paths.project_tls_private_key(&project.id);
    write_project_certificate_with_remaining_days(&paths, &project, &local_ca, 3650)?;
    let stale_certificate_chain = state::fs::read_to_string(&certificate_path)?;
    let stale_private_key = state::fs::read_to_string(&private_key_path)?;

    assert_eq!(certificate_pem_block_count(&stale_certificate_chain), 2);
    assert!(!platform::project_certificate_matches(
        &stale_certificate_chain,
        &stale_private_key,
        served_project_hostname(&project)?,
        &local_ca.certificate_pem
    ));
    run_project_reconciliation(&paths, &project).await?;
    let (certificate_after_reconciliation, private_key_after_reconciliation) =
        read_project_tls_files(&paths, &project)?;
    let dotenv_after_reconciliation = read_dotenv(&project)?;

    assert_ne!(
        certificate_after_reconciliation, stale_certificate_chain,
        "a ten-year Project certificate should be replaced"
    );
    assert_ne!(
        private_key_after_reconciliation, stale_private_key,
        "replacement should issue a new Project private key"
    );
    assert_eq!(
        certificate_pem_block_count(&certificate_after_reconciliation),
        2
    );
    assert_project_certificate_matches(
        &certificate_after_reconciliation,
        &private_key_after_reconciliation,
        served_project_hostname(&project)?,
        &local_ca,
    );
    assert_eq!(
        state::fs::read_to_string(&paths.ca_certificate())?,
        ca_certificate_before,
        "reconciliation should preserve the PV CA certificate"
    );
    assert_eq!(
        state::fs::read_to_string(&paths.ca_private_key())?,
        ca_private_key_before,
        "reconciliation should preserve the PV CA private key"
    );
    assert_eq!(
        dotenv_after_reconciliation,
        format!(
            "# >>> PV MANAGED\nPV_TLS_CA={}\nVITE_DEV_SERVER_CERT={}\nVITE_DEV_SERVER_KEY={}\n# <<< PV MANAGED\n",
            paths.ca_certificate(),
            certificate_path,
            private_key_path
        )
    );

    run_project_reconciliation(&paths, &project).await?;
    let (certificate_after_second_reconciliation, private_key_after_second_reconciliation) =
        read_project_tls_files(&paths, &project)?;
    let dotenv_after_second_reconciliation = read_dotenv(&project)?;

    assert_eq!(
        certificate_after_second_reconciliation, certificate_after_reconciliation,
        "a valid short-lived Project certificate should be byte-idempotent"
    );
    assert_eq!(
        private_key_after_second_reconciliation, private_key_after_reconciliation,
        "a valid Project private key should be byte-idempotent"
    );
    assert_eq!(
        dotenv_after_second_reconciliation,
        format!(
            "# >>> PV MANAGED\nPV_TLS_CA={}\nVITE_DEV_SERVER_CERT={}\nVITE_DEV_SERVER_KEY={}\n# <<< PV MANAGED\n",
            paths.ca_certificate(),
            certificate_path,
            private_key_path
        )
    );
    assert_eq!(
        state::fs::read_to_string(&paths.ca_certificate())?,
        ca_certificate_before
    );
    assert_eq!(
        state::fs::read_to_string(&paths.ca_private_key())?,
        ca_private_key_before
    );

    Ok(())
}

#[tokio::test]
async fn daemon_health_tick_replaces_expiring_tls_certificate_without_explicit_reconciliation()
-> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"env:
  VITE_DEV_SERVER_CERT: "${tls_cert}"
  VITE_DEV_SERVER_KEY: "${tls_key}"
"#,
    )?;
    let local_ca = seed_local_ca(&paths)?;
    write_project_certificate_with_remaining_days(&paths, &project, &local_ca, 7)?;
    let stale_certificate_pem =
        state::fs::read_to_string(&paths.project_tls_certificate(&project.id))?;
    let config_modified_at = state::fs::modified_at(&project.config_path)?;

    let finished_system_jobs = finished_system_job_count(&paths)?;
    let daemon =
        daemon::RunningDaemon::start_without_managed_resource_adapters(paths.clone()).await?;
    wait_for_new_finished_system_job(&paths, finished_system_jobs).await?;
    let jobs_before_health_tick = Database::open(&paths)?.recent_jobs()?;
    assert_eq!(jobs_before_health_tick.len(), 1);
    assert_eq!(jobs_before_health_tick[0].scope, "system");
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(30)).await;
    tokio::task::yield_now().await;
    tokio::time::resume();
    let job = wait_for_succeeded_project_job(&paths, &project.id).await?;
    let (certificate_pem, private_key_pem) = read_project_tls_files(&paths, &project)?;
    daemon.shutdown().await?;

    assert_eq!(job.scope, format!("project:{}", project.id));
    assert_ne!(certificate_pem, stale_certificate_pem);
    assert_project_certificate_matches(
        &certificate_pem,
        &private_key_pem,
        served_project_hostname(&project)?,
        &local_ca,
    );
    assert_eq!(
        state::fs::modified_at(&project.config_path)?,
        config_modified_at,
        "TLS health discovery must not rewrite the Project config"
    );

    let database = Database::open(&paths)?;
    let jobs = database.recent_jobs()?;
    assert_eq!(
        jobs.iter().filter(|job| job.scope == "system").count(),
        1,
        "the health tick should not add another System reconciliation after startup"
    );

    Ok(())
}

#[tokio::test]
async fn project_env_reconciliation_uses_project_root_not_config_path_for_dotenv() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project_root = tempdir.path().join("canonical-project");
    let original_path = tempdir.path().join("typed-project-path");
    let stored_config_path = tempdir.path().join("stale-config-location/pv.yml");

    write_sensitive_file(
        &project_root.join("pv.yml"),
        "env:\n  APP_URL: \"${project_url}\"\n  APP_NAME: canonical\n",
    )?;
    write_sensitive_file(
        &original_path.join(".env"),
        "ORIGINAL_PATH_VALUE=must-not-change\n",
    )?;
    write_sensitive_file(&stored_config_path, "env:\n  APP_NAME: stale-config-path\n")?;

    let mut database = Database::open(&paths)?;
    let project = database.link_project(LinkProjectInput {
        path: project_root.clone(),
        original_path: original_path.clone(),
        primary_hostname: "acme.test".to_owned(),
        config_path: stored_config_path.clone(),
        desired_php_track: None,
        additional_hostnames: Vec::new(),
    })?;
    drop(database);

    let lines = run_project_reconciliation(&paths, &project.project).await?;
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "project_env_reconciliation_uses_project_root_not_config_path_for_dotenv",
        (
            lines,
            read_dotenv(&project.project)?,
            state::fs::read_to_string(&original_path.join(".env"))?,
            state::fs::read_to_string(&stored_config_path)?,
            database.project_env_observed_state(&project.project.id)?,
            latest_job(&database, &format!("project:{}", project.project.id))?,
        ),
    )?;

    Ok(())
}

#[tokio::test]
async fn project_env_reconciliation_updates_persisted_php_track() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        "php: \"8.4\"\n",
    )?;

    run_project_reconciliation(&paths, &project).await?;
    write_project_config(&project, "php: \"8.3\"\n")?;
    run_project_reconciliation(&paths, &project).await?;

    let database = Database::open(&paths)?;
    let project = database
        .project_by_id(&project.id)?
        .ok_or_else(|| anyhow!("expected linked project"))?;

    assert_eq!(project.desired_php_track.as_deref(), Some("8.3"));

    Ok(())
}

#[tokio::test]
async fn project_env_reconciliation_persists_latest_php_as_concrete_track() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        "php: latest\n",
    )?;
    seed_manifest(&paths, "8.4")?;

    run_project_reconciliation(&paths, &project).await?;

    let database = Database::open(&paths)?;
    let project = database
        .project_by_id(&project.id)?
        .ok_or_else(|| anyhow!("expected linked project"))?;

    assert_eq!(project.desired_php_track.as_deref(), Some("8.4"));

    Ok(())
}

#[tokio::test]
async fn project_env_reconciliation_persists_php_extension_runtime() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        "php:\n  version: \"8.4\"\n  extensions: [redis, missing]\n",
    )?;
    let release = tempdir.path().join("php-release");
    write_sensitive_file(&release.join("bin/php"), "#!/bin/sh\n")?;
    write_sensitive_file(
        &release.join("share/pv/php-extensions.json"),
        r#"[{"name":"redis","load_kind":"extension","path":"lib/php/extensions/redis.so"}]"#,
    )?;
    write_sensitive_file(&release.join("lib/php/extensions/redis.so"), "")?;
    {
        let mut database = Database::open(&paths)?;
        database.record_managed_resource_track_installed("php", "8.4", "8.4.8-pv1", &release)?;
    }

    run_project_reconciliation(&paths, &project).await?;

    let database = Database::open(&paths)?;
    let project = database
        .project_by_id(&project.id)?
        .ok_or_else(|| anyhow!("expected linked project"))?;
    let observed = database
        .project_env_observed_state(&project.id)?
        .ok_or_else(|| anyhow!("expected observed project env state"))?;

    assert_eq!(project.php_runtime.track.as_deref(), Some("8.4"));
    assert_eq!(
        project.php_runtime.requested_extensions,
        ["redis", "missing"]
    );
    assert_eq!(project.php_runtime.loaded_extensions, ["redis"]);
    assert_eq!(project.php_runtime.ignored_extensions, ["missing"]);
    assert_eq!(observed.status, ProjectEnvObservedStatus::Warning);
    assert_eq!(observed.warnings[0].kind, "ignored_php_extension");

    Ok(())
}

#[tokio::test]
async fn project_env_reconciliation_persists_non_identity_ignored_php_extension() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        "php:\n  version: \"8.4\"\n  extensions: [\"not-supported-yet\"]\n",
    )?;
    let release = tempdir.path().join("php-release");
    write_sensitive_file(&release.join("bin/php"), "#!/bin/sh\n")?;
    write_sensitive_file(
        &release.join("share/pv/php-extensions.json"),
        r#"[{"name":"redis","load_kind":"extension","path":"lib/php/extensions/redis.so"}]"#,
    )?;
    write_sensitive_file(&release.join("lib/php/extensions/redis.so"), "")?;
    {
        let mut database = Database::open(&paths)?;
        database.record_managed_resource_track_installed("php", "8.4", "8.4.8-pv1", &release)?;
    }

    run_project_reconciliation(&paths, &project).await?;

    let database = Database::open(&paths)?;
    let project = database
        .project_by_id(&project.id)?
        .ok_or_else(|| anyhow!("expected linked project"))?;
    let observed = database
        .project_env_observed_state(&project.id)?
        .ok_or_else(|| anyhow!("expected observed project env state"))?;

    assert_eq!(project.php_runtime.track.as_deref(), Some("8.4"));
    assert_eq!(
        project.php_runtime.requested_extensions,
        ["not-supported-yet"]
    );
    assert!(project.php_runtime.loaded_extensions.is_empty());
    assert_eq!(
        project.php_runtime.ignored_extensions,
        ["not-supported-yet"]
    );
    assert_eq!(observed.status, ProjectEnvObservedStatus::Warning);
    assert_eq!(observed.warnings[0].kind, "ignored_php_extension");
    assert_eq!(
        observed.warnings[0].message,
        "ignored unsupported PHP extension `not-supported-yet`"
    );

    Ok(())
}

#[tokio::test]
async fn project_env_reconciliation_reuses_concrete_track_for_latest_php() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        "php: latest\n",
    )?;
    seed_manifest(&paths, "8.4")?;

    run_project_reconciliation(&paths, &project).await?;
    seed_manifest(&paths, "8.3")?;
    run_project_reconciliation(&paths, &project).await?;

    let database = Database::open(&paths)?;
    let project = database
        .project_by_id(&project.id)?
        .ok_or_else(|| anyhow!("expected linked project"))?;

    assert_eq!(project.desired_php_track.as_deref(), Some("8.4"));

    Ok(())
}

#[tokio::test]
async fn project_env_reconciliation_persists_default_php_track_when_config_omits_php() -> Result<()>
{
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        "php: \"8.4\"\n",
    )?;

    run_project_reconciliation(&paths, &project).await?;
    seed_manifest(&paths, "8.0")?;
    write_project_config(&project, "hostnames: []\n")?;
    run_project_reconciliation(&paths, &project).await?;

    let database = Database::open(&paths)?;
    let project = database
        .project_by_id(&project.id)?
        .ok_or_else(|| anyhow!("expected linked project"))?;

    assert_eq!(project.desired_php_track.as_deref(), Some("8.0"));

    Ok(())
}

#[tokio::test]
async fn seeded_resource_and_allocation_contexts_render_dotenv() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"postgres:
  version: "8.0"
  allocations:
    app-db:
      env:
        DATABASE_URL: "postgres://${username}:${password}@${host}:${port}/${database}"
        DB_DATABASE: "${database}"
        DB_HOST: "${host}"
        DB_PORT: "${port}"
        DB_USERNAME: "${username}"
"#,
    )?;
    seed_postgres_context(&paths, &project)?;

    let lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "seeded_resource_and_allocation_contexts_render_dotenv",
        (
            lines,
            read_optional_dotenv(&project)?,
            database.project_managed_resources(&project.id)?,
            database.resource_allocations(&project.id, "postgres")?,
            database.project_env_observed_state(&project.id)?,
            latest_job(&database, &format!("project:{}", project.id))?,
        ),
    )?;

    Ok(())
}

#[tokio::test]
async fn existing_allocation_name_survives_primary_hostname_change() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"postgres:
  version: "8.0"
  allocations:
    app-db:
      env:
        DB_DATABASE: "${database}"
"#,
    )?;
    seed_postgres_context(&paths, &project)?;
    let project = update_project_primary_hostname(
        &paths,
        &project,
        "renamed-primary-hostname-that-would-exceed-db-name-limit.test",
    )?;

    let lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "existing_allocation_name_survives_primary_hostname_change",
        (
            lines,
            read_optional_dotenv(&project)?,
            database.resource_allocations(&project.id, "postgres")?,
            database.project_env_observed_state(&project.id)?,
            latest_job(&database, &format!("project:{}", project.id))?,
        ),
    )?;

    Ok(())
}

#[tokio::test]
async fn missing_context_leaves_dotenv_unchanged_and_records_failure() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        "postgres:\n  version: \"8.0\"\n  env:\n    DB_HOST: \"${host}\"\n",
    )?;
    write_sensitive_file(&project.path.join(".env"), "USER_VALUE=kept\n")?;

    let lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "missing_context_leaves_dotenv_unchanged_and_records_failure",
        (
            lines,
            read_dotenv(&project)?,
            database.project_env_observed_state(&project.id)?,
            latest_job(&database, &format!("project:{}", project.id))?,
        ),
    )?;

    Ok(())
}

#[tokio::test]
async fn root_env_with_resource_waits_for_resource_context_before_dotenv() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        "env:\n  APP_URL: \"${project_url}\"\npostgres:\n  version: \"8.0\"\n",
    )?;
    write_sensitive_file(&project.path.join(".env"), "USER_VALUE=kept\n")?;

    let lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "root_env_with_resource_waits_for_resource_context_before_dotenv",
        (
            lines,
            read_dotenv(&project)?,
            database.project_managed_resources(&project.id)?,
            database.project_env_observed_state(&project.id)?,
            latest_job(&database, &format!("project:{}", project.id))?,
        ),
    )?;

    Ok(())
}

#[tokio::test]
async fn first_allocation_reconciliation_records_desired_state_before_context_failure() -> Result<()>
{
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"postgres:
  version: "8.0"
  allocations:
    app-db:
      env:
        DB_DATABASE: "${database}"
"#,
    )?;

    let lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "first_allocation_reconciliation_records_desired_state_before_context_failure",
        (
            lines,
            read_optional_dotenv(&project)?,
            database.project_managed_resources(&project.id)?,
            database.resource_allocations(&project.id, "postgres")?,
            database.project_env_observed_state(&project.id)?,
            latest_job(&database, &format!("project:{}", project.id))?,
        ),
    )?;

    Ok(())
}

#[tokio::test]
async fn malformed_pv_block_leaves_dotenv_unchanged_and_records_failure() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        "env:\n  APP_URL: \"${project_url}\"\n",
    )?;
    write_sensitive_file(
        &project.path.join(".env"),
        "USER_VALUE=kept\n# >>> PV MANAGED\nAPP_URL=https://old.test\n",
    )?;

    let lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "malformed_pv_block_leaves_dotenv_unchanged_and_records_failure",
        (
            lines,
            read_dotenv(&project)?,
            database.project_env_observed_state(&project.id)?,
            latest_job(&database, &format!("project:{}", project.id))?,
        ),
    )?;

    Ok(())
}

#[tokio::test]
async fn malformed_pv_block_preflight_preserves_resource_and_hostname_state() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"hostnames:
  - api.acme.test
postgres:
  version: "8.0"
  env:
    DB_HOST: "${host}"
"#,
    )?;
    write_sensitive_file(
        &project.path.join(".env"),
        "USER_VALUE=kept\n# >>> PV MANAGED\nAPP_URL=https://old.test\n",
    )?;

    let lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;
    let hostnames = database
        .project_by_id(&project.id)?
        .map(|project| project.additional_hostnames);

    assert_with_normalized_timestamps(
        "malformed_pv_block_preflight_preserves_resource_and_hostname_state",
        (
            lines,
            read_dotenv(&project)?,
            hostnames,
            database.project_managed_resources(&project.id)?,
            database.resource_allocations(&project.id, "postgres")?,
            database.project_env_observed_state(&project.id)?,
            latest_job(&database, &format!("project:{}", project.id))?,
        ),
    )?;

    Ok(())
}

#[tokio::test]
async fn duplicate_user_owned_key_writes_block_and_records_warning() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        "env:\n  APP_URL: \"${project_url}\"\n",
    )?;
    write_sensitive_file(&project.path.join(".env"), "APP_URL=https://user.test\n")?;

    let lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "duplicate_user_owned_key_writes_block_and_records_warning",
        (
            lines,
            read_dotenv(&project)?,
            database.project_env_observed_state(&project.id)?,
            latest_job(&database, &format!("project:{}", project.id))?,
        ),
    )?;

    Ok(())
}

#[tokio::test]
async fn duplicate_rendered_env_key_leaves_resource_state_unchanged() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"postgres:
  version: "8.0"
  allocations:
    analytics:
      env:
        DATABASE_URL: "postgres://${database}"
    app:
      env:
        DATABASE_URL: "postgres://${database}"
"#,
    )?;

    let lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "duplicate_rendered_env_key_leaves_resource_state_unchanged",
        (
            lines,
            read_optional_dotenv(&project)?,
            database.project_managed_resources(&project.id)?,
            database.resource_allocations(&project.id, "postgres")?,
            database.project_env_observed_state(&project.id)?,
            latest_job(&database, &format!("project:{}", project.id))?,
        ),
    )?;

    Ok(())
}

#[tokio::test]
async fn generated_allocation_name_too_long_leaves_resource_state_unchanged() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let allocation_name = "a".repeat(57);
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "a.test",
        &format!(
            r#"postgres:
  version: "8.0"
  allocations:
    {allocation_name}:
      env:
        DB_DATABASE: "${{database}}"
"#
        ),
    )?;

    let lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "generated_allocation_name_too_long_leaves_resource_state_unchanged",
        (
            lines,
            read_optional_dotenv(&project)?,
            database.project_managed_resources(&project.id)?,
            database.resource_allocations(&project.id, "postgres")?,
            database.project_env_observed_state(&project.id)?,
            latest_job(&database, &format!("project:{}", project.id))?,
        ),
    )?;

    Ok(())
}

#[tokio::test]
async fn invalid_config_failure_rolls_back_resource_state_mutations() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"hostnames:
  - api.acme.test
postgres:
  version: "8.0"
  allocations:
    app-db:
      env:
        DB_DATABASE: "${database}"
"#,
    )?;
    seed_postgres_context(&paths, &project)?;
    let initial_lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;
    let managed_resources_before = database.project_managed_resources(&project.id)?;
    let allocations_before = database.resource_allocations(&project.id, "postgres")?;
    let hostnames_before = database
        .project_by_id(&project.id)?
        .map(|project| project.additional_hostnames);
    let dotenv_before = read_dotenv(&project)?;

    write_project_config(
        &project,
        r#"hostnames:
  - changed.acme.test
redis:
  version: "7.2"
  env:
    REDIS_HOST: "${missing_value}"
"#,
    )?;

    let invalid_lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;
    let managed_resources_after = database.project_managed_resources(&project.id)?;
    let allocations_after = database.resource_allocations(&project.id, "postgres")?;
    let hostnames_after = database
        .project_by_id(&project.id)?
        .map(|project| project.additional_hostnames);
    let dotenv_after = read_dotenv(&project)?;

    assert_eq!(
        hostnames_before, hostnames_after,
        "invalid Project config must preserve the last valid additional hostnames"
    );
    assert_eq!(
        managed_resources_before, managed_resources_after,
        "invalid Project config must preserve the last valid managed resources"
    );
    assert_eq!(
        allocations_before, allocations_after,
        "invalid Project config must preserve the last valid Resource allocations"
    );
    assert_eq!(
        dotenv_before, dotenv_after,
        "invalid Project config must preserve the last rendered .env block"
    );

    assert_with_normalized_timestamps(
        "invalid_config_failure_rolls_back_resource_state_mutations",
        (
            initial_lines,
            invalid_lines,
            hostnames_after,
            managed_resources_after,
            allocations_after,
            database.resource_allocations(&project.id, "redis")?,
            dotenv_after,
            database.project_env_observed_state(&project.id)?,
            latest_job(&database, &format!("project:{}", project.id))?,
        ),
    )?;

    Ok(())
}

#[tokio::test]
async fn malformed_config_with_existing_tls_is_renewed_by_daemon_health() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"env:
  VITE_DEV_SERVER_KEY: "${tls_key}"
  VITE_DEV_SERVER_CERT: "${tls_cert}"
"#,
    )?;
    let local_ca = seed_local_ca(&paths)?;
    write_project_certificate_with_remaining_days(&paths, &project, &local_ca, 7)?;
    let (certificate_before, private_key_before) = read_project_tls_files(&paths, &project)?;
    assert!(!platform::project_certificate_matches(
        &certificate_before,
        &private_key_before,
        served_project_hostname(&project)?,
        &local_ca.certificate_pem,
    ));
    state::fs::remove_file(&paths.project_tls_private_key(&project.id))?;
    write_sensitive_file(&project.path.join(".env"), "USER_VALUE=kept\n")?;
    let dotenv_before = read_dotenv(&project)?;
    let malformed_config = "env: [\n";
    write_project_config(&project, malformed_config)?;
    let config_before = read_project_config(&project)?;
    let config_error = match config::ProjectConfigFile::read_from_root(&project.path) {
        Ok(_) => return Err(anyhow!("malformed config should remain invalid")),
        Err(error) => error.to_string(),
    };

    let finished_system_jobs = finished_system_job_count(&paths)?;
    let daemon =
        daemon::RunningDaemon::start_without_managed_resource_adapters(paths.clone()).await?;
    wait_for_new_finished_system_job(&paths, finished_system_jobs).await?;
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(30)).await;
    tokio::task::yield_now().await;
    tokio::time::resume();
    let mut failed_job = None;
    for _attempt in 0..50 {
        let certificate_exists =
            state::fs::path_entry_exists(&paths.project_tls_certificate(&project.id))?;
        let private_key_exists =
            state::fs::path_entry_exists(&paths.project_tls_private_key(&project.id))?;
        if certificate_exists && private_key_exists {
            let (certificate, private_key) = read_project_tls_files(&paths, &project)?;
            let renewed = platform::project_certificate_matches(
                &certificate,
                &private_key,
                served_project_hostname(&project)?,
                &local_ca.certificate_pem,
            );
            let database = Database::open(&paths)?;
            if renewed
                && let Some(job) = database.recent_jobs()?.into_iter().find(|job| {
                    job.scope == format!("project:{}", project.id)
                        && job.status == state::JobStatus::Failed
                })
            {
                failed_job = Some(job);
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    daemon.shutdown().await?;

    let database = Database::open(&paths)?;
    let job = failed_job.ok_or_else(|| anyhow!("daemon health did not renew and fail Project"))?;
    let observed = database
        .project_env_observed_state(&project.id)?
        .ok_or_else(|| anyhow!("missing Project env observation"))?;

    assert_eq!(job.status, state::JobStatus::Failed);
    let expected_error = format!("Project config error: {config_error}");
    assert_eq!(job.error.as_deref(), Some(expected_error.as_str()));
    assert_eq!(observed.status, ProjectEnvObservedStatus::Failed);
    assert_eq!(read_dotenv(&project)?, dotenv_before);
    assert_eq!(read_project_config(&project)?, config_before);

    Ok(())
}

#[tokio::test]
async fn legacy_url_placeholder_failure_preserves_last_valid_desired_state() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"hostnames:
  - api.acme.test
env:
  APP_URL: "${project_url}"
postgres:
  version: "8.0"
  allocations:
    app-db:
      env:
        DB_DATABASE: "${database}"
"#,
    )?;
    seed_postgres_context(&paths, &project)?;
    let initial_lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;
    let managed_resources_before = database.project_managed_resources(&project.id)?;
    let allocations_before = database.resource_allocations(&project.id, "postgres")?;
    let hostnames_before = database
        .project_by_id(&project.id)?
        .map(|project| project.additional_hostnames);
    let dotenv_before = read_dotenv(&project)?;

    write_project_config(
        &project,
        r#"hostnames:
  - changed.acme.test
env:
  BAD_URL: "${url}"
redis:
  version: "7.2"
  allocations:
    cache: {}
"#,
    )?;

    let invalid_lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;
    let managed_resources_after = database.project_managed_resources(&project.id)?;
    let allocations_after = database.resource_allocations(&project.id, "postgres")?;
    let hostnames_after = database
        .project_by_id(&project.id)?
        .map(|project| project.additional_hostnames);
    let dotenv_after = read_dotenv(&project)?;

    assert_eq!(
        hostnames_before, hostnames_after,
        "invalid legacy URL placeholder config must preserve additional hostnames"
    );
    assert_eq!(
        managed_resources_before, managed_resources_after,
        "invalid legacy URL placeholder config must preserve managed resources"
    );
    assert_eq!(
        allocations_before, allocations_after,
        "invalid legacy URL placeholder config must preserve Resource allocations"
    );
    assert_eq!(
        dotenv_before, dotenv_after,
        "invalid legacy URL placeholder config must preserve the last rendered .env block"
    );

    assert_with_normalized_timestamps(
        "legacy_url_placeholder_failure_preserves_last_valid_desired_state",
        (
            initial_lines,
            invalid_lines,
            hostnames_after,
            managed_resources_after,
            allocations_after,
            database.resource_allocations(&project.id, "redis")?,
            dotenv_after,
            database.project_env_observed_state(&project.id)?,
            latest_job(&database, &format!("project:{}", project.id))?,
        ),
    )?;

    Ok(())
}

#[tokio::test]
async fn no_mappings_do_not_touch_existing_dotenv_and_record_noop_success() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        "php: \"8.4\"\n",
    )?;
    write_sensitive_file(&project.path.join(".env"), "USER_VALUE=kept\n")?;

    let lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "no_mappings_do_not_touch_existing_dotenv_and_record_noop_success",
        (
            lines,
            read_dotenv(&project)?,
            database.project_env_observed_state(&project.id)?,
            latest_job(&database, &format!("project:{}", project.id))?,
        ),
    )?;

    Ok(())
}

#[tokio::test]
async fn resources_and_empty_allocations_without_env_mappings_update_state_without_dotenv()
-> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"postgres:
  version: "8.0"
  allocations:
    app-db: {}
"#,
    )?;

    let lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "resources_and_empty_allocations_without_env_mappings_update_state_without_dotenv",
        (
            lines,
            read_optional_dotenv(&project)?,
            database.project_managed_resources(&project.id)?,
            database.resource_allocations(&project.id, "postgres")?,
            database.project_env_observed_state(&project.id)?,
            latest_job(&database, &format!("project:{}", project.id))?,
        ),
    )?;

    Ok(())
}

#[tokio::test]
async fn missing_dotenv_is_created_with_private_permissions() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        "env:\n  APP_URL: \"${project_url}\"\n",
    )?;

    let lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;
    let dotenv_path = project.path.join(".env");

    assert_with_normalized_timestamps(
        "missing_dotenv_is_created_with_private_permissions",
        (
            lines,
            read_dotenv(&project)?,
            mode_string(&dotenv_path)?,
            database.project_env_observed_state(&project.id)?,
            latest_job(&database, &format!("project:{}", project.id))?,
        ),
    )?;

    Ok(())
}

#[tokio::test]
async fn multiple_managed_dotenv_blocks_fold_to_one_and_preserve_permissions() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        "env:\n  APP_URL: \"${project_url}\"\n",
    )?;
    let dotenv_path = project.path.join(".env");
    write_sensitive_file(
        &dotenv_path,
        r#"BEFORE=1
# >>> PV MANAGED
OLD_ONE=stale
# <<< PV MANAGED
BETWEEN=1
# >>> PV MANAGED
OLD_TWO=stale
# <<< PV MANAGED
AFTER=1
"#,
    )?;
    set_file_mode(&dotenv_path, 0o640)?;

    let lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "multiple_managed_dotenv_blocks_fold_to_one_and_preserve_permissions",
        (
            lines,
            read_dotenv(&project)?,
            mode_string(&dotenv_path)?,
            database.project_env_observed_state(&project.id)?,
            latest_job(&database, &format!("project:{}", project.id))?,
        ),
    )?;

    Ok(())
}

#[tokio::test]
async fn tls_renews_before_malformed_dotenv_validation_without_mutating_primary_error_or_state()
-> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"env:
  VITE_DEV_SERVER_CERT: "${tls_cert}"
postgres:
  version: "8.0"
  env:
    DB_HOST: "${host}"
"#,
    )?;
    let local_ca = seed_local_ca(&paths)?;
    write_project_certificate_with_remaining_days(&paths, &project, &local_ca, 7)?;
    let dotenv = "USER_VALUE=kept\n# >>> PV MANAGED\n";
    write_sensitive_file(&project.path.join(".env"), dotenv)?;
    let database = Database::open(&paths)?;
    let project_before = database
        .project_by_id(&project.id)?
        .ok_or_else(|| anyhow!("expected linked Project"))?;
    let resources_before = database.project_managed_resources(&project.id)?;
    let allocations_before = database.resource_allocations(&project.id, "postgres")?;
    let expected_validation_error = match config::validate_managed_env_block(Some(dotenv)) {
        Ok(()) => return Err(anyhow!("malformed managed .env should fail validation")),
        Err(error) => error,
    };

    let lines = run_project_reconciliation(&paths, &project).await?;
    let (certificate, private_key) = read_project_tls_files(&paths, &project)?;
    let database = Database::open(&paths)?;
    let project_after = database
        .project_by_id(&project.id)?
        .ok_or_else(|| anyhow!("expected linked Project after reconciliation"))?;
    let job = latest_job(&database, &format!("project:{}", project.id))?;

    assert!(platform::project_certificate_matches(
        &certificate,
        &private_key,
        served_project_hostname(&project)?,
        &local_ca.certificate_pem,
    ));
    assert_eq!(
        job.error.as_deref(),
        Some(format!("Project config error: {expected_validation_error}").as_str())
    );
    assert_eq!(read_dotenv(&project)?, dotenv);
    assert_eq!(project_after, project_before);
    assert_eq!(
        database.project_managed_resources(&project.id)?,
        resources_before
    );
    assert_eq!(
        database.resource_allocations(&project.id, "postgres")?,
        allocations_before
    );
    assert!(!lines.is_empty());

    Ok(())
}

#[tokio::test]
async fn masked_tls_maintenance_failure_is_written_to_structured_log() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        "env:\n  CERT: \"${tls_cert}\"\n",
    )?;
    seed_local_ca(&paths)?;
    write_sensitive_file(
        &project.path.join(".env"),
        "USER_VALUE=kept\n# >>> PV MANAGED\n",
    )?;
    state::fs::remove_file(&paths.ca_private_key())?;
    let expected_tls_error = match state::fs::read_to_string(&paths.ca_private_key()) {
        Ok(_) => return Err(anyhow!("missing CA private key should fail to read")),
        Err(error) => daemon::DaemonError::from(error).to_string(),
    };

    run_project_reconciliation(&paths, &project).await?;

    let content = state::fs::read_to_string(&paths.daemon_log())?;
    let events = content
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<Result<Vec<_>, _>>()?;
    assert!(events.iter().any(|event| {
        event["event"] == "project_tls_maintenance_failed"
            && event["project_id"] == project.id
            && event["error"] == expected_tls_error
    }));

    Ok(())
}

#[tokio::test]
async fn project_env_reconciliation_uses_global_php_default_for_extension_only_config() -> Result<()>
{
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        "php:\n  extensions: [redis]\n",
    )?;
    seed_manifest(&paths, "8.5")?;
    {
        let mut database = Database::open(&paths)?;
        database.record_global_php_default_track("8.3")?;
    }

    run_project_reconciliation(&paths, &project).await?;

    let database = Database::open(&paths)?;
    let project = database
        .project_by_id(&project.id)?
        .ok_or_else(|| anyhow!("expected linked project"))?;

    assert_eq!(project.desired_php_track.as_deref(), Some("8.3"));
    assert_eq!(
        project.php_runtime.requested_extensions,
        vec!["redis".to_string()]
    );
    assert_eq!(
        project.php_runtime.ignored_extensions,
        vec!["redis".to_string()]
    );

    Ok(())
}

#[tokio::test]
async fn config_declared_hostnames_are_persisted_during_reconciliation() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        "hostnames:\n  - api.acme.test\nphp: \"8.4\"\n",
    )?;

    let lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;
    let linked_hostnames = database
        .project_by_id(&project.id)?
        .map(|project| project.additional_hostnames);
    let resolved_primary = database
        .project_by_hostname("api.acme.test")?
        .map(|project| project.primary_hostname);

    assert_with_normalized_timestamps(
        "config_declared_hostnames_are_persisted_during_reconciliation",
        (
            lines,
            linked_hostnames,
            resolved_primary,
            latest_job(&database, &format!("project:{}", project.id))?,
        ),
    )?;

    Ok(())
}

#[tokio::test]
async fn latest_resource_track_resolves_default_track_before_state_and_dotenv_writes() -> Result<()>
{
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        "postgres:\n  version: latest\n  env:\n    DB_HOST: \"${host}\"\n",
    )?;
    seed_manifest(&paths, "8.0")?;
    seed_postgres_resource_context(&paths)?;

    let lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "latest_resource_track_resolves_default_track_before_state_and_dotenv_writes",
        (
            lines,
            read_project_config(&project)?,
            read_optional_dotenv(&project)?,
            database.project_managed_resources(&project.id)?,
            database.resource_allocations(&project.id, "postgres")?,
            database.project_env_observed_state(&project.id)?,
            latest_job(&database, &format!("project:{}", project.id))?,
        ),
    )?;

    Ok(())
}

#[tokio::test]
async fn latest_resource_track_reuses_stored_track_when_manifest_default_changes() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        "postgres:\n  version: latest\n  env:\n    DB_HOST: \"${host}\"\n    DB_PORT: \"${port}\"\n",
    )?;
    seed_manifest(&paths, "8.0")?;
    seed_postgres_resource_context(&paths)?;
    let initial_lines = run_project_reconciliation(&paths, &project).await?;

    seed_manifest(&paths, "8.4")?;
    seed_postgres_resource_context_for_track(&paths, "8.4", "3406")?;
    let rerun_lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "latest_resource_track_reuses_stored_track_when_manifest_default_changes",
        (
            initial_lines,
            rerun_lines,
            read_project_config(&project)?,
            read_optional_dotenv(&project)?,
            database.project_managed_resources(&project.id)?,
            database.project_env_observed_state(&project.id)?,
            latest_job(&database, &format!("project:{}", project.id))?,
        ),
    )?;

    Ok(())
}

#[tokio::test]
async fn omitted_resource_track_resolves_manifest_default_track() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        "postgres:\n  env:\n    DB_HOST: \"${host}\"\n",
    )?;
    seed_manifest(&paths, "8.0")?;
    seed_postgres_resource_context(&paths)?;

    let lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "omitted_resource_track_resolves_manifest_default_track",
        (
            lines,
            read_project_config(&project)?,
            read_optional_dotenv(&project)?,
            database.project_managed_resources(&project.id)?,
            database.project_env_observed_state(&project.id)?,
            latest_job(&database, &format!("project:{}", project.id))?,
        ),
    )?;

    Ok(())
}

#[tokio::test]
async fn omitted_resource_track_reuses_stored_track_when_manifest_default_changes() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        "postgres:\n  env:\n    DB_HOST: \"${host}\"\n    DB_PORT: \"${port}\"\n",
    )?;
    seed_manifest(&paths, "8.0")?;
    seed_postgres_resource_context(&paths)?;
    let initial_lines = run_project_reconciliation(&paths, &project).await?;

    seed_manifest(&paths, "8.4")?;
    seed_postgres_resource_context_for_track(&paths, "8.4", "3406")?;
    let rerun_lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "omitted_resource_track_reuses_stored_track_when_manifest_default_changes",
        (
            initial_lines,
            rerun_lines,
            read_project_config(&project)?,
            read_optional_dotenv(&project)?,
            database.project_managed_resources(&project.id)?,
            database.project_env_observed_state(&project.id)?,
            latest_job(&database, &format!("project:{}", project.id))?,
        ),
    )?;

    Ok(())
}

#[tokio::test]
async fn omitted_resource_track_without_mappings_updates_state_without_dotenv() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        "postgres:\n",
    )?;
    seed_manifest(&paths, "8.0")?;

    let lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "omitted_resource_track_without_mappings_updates_state_without_dotenv",
        (
            lines,
            read_optional_dotenv(&project)?,
            database.project_managed_resources(&project.id)?,
            database.resource_allocations(&project.id, "postgres")?,
            database.project_env_observed_state(&project.id)?,
            latest_job(&database, &format!("project:{}", project.id))?,
        ),
    )?;

    Ok(())
}

async fn run_project_reconciliation(
    paths: &PvPaths,
    project: &ProjectRecord,
) -> Result<Vec<Value>> {
    ensure_reconciliation_dns_port(paths)?;

    let finished_system_jobs = finished_system_job_count(paths)?;
    let daemon =
        daemon::RunningDaemon::start_without_managed_resource_adapters(paths.clone()).await?;
    wait_for_new_finished_system_job(paths, finished_system_jobs).await?;
    let lines = request_lines(
        paths,
        json!({
            "protocol_version": daemon::PROTOCOL_VERSION,
            "command": "run_job",
            "kind": "reconcile",
            "scope": format!("project:{}", project.id),
        }),
    )
    .await?;

    daemon.shutdown().await?;

    Ok(lines)
}

async fn wait_for_succeeded_project_job(paths: &PvPaths, project_id: &str) -> Result<JobRecord> {
    let scope = format!("project:{project_id}");
    for _attempt in 0..50 {
        let database = Database::open(paths)?;
        if let Some(job) = database
            .recent_jobs()?
            .into_iter()
            .find(|job| job.scope == scope && job.status == state::JobStatus::Succeeded)
        {
            return Ok(job);
        }

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    Err(anyhow!(
        "succeeded job with scope {scope:?} was not recorded"
    ))
}

fn finished_system_job_count(paths: &PvPaths) -> Result<usize> {
    let database = Database::open(paths)?;

    Ok(database
        .recent_jobs()?
        .into_iter()
        .filter(|job| {
            job.scope == "system"
                && matches!(
                    job.status,
                    state::JobStatus::Succeeded | state::JobStatus::Failed
                )
        })
        .count())
}

async fn wait_for_new_finished_system_job(paths: &PvPaths, existing_count: usize) -> Result<()> {
    for _attempt in 0..100 {
        if finished_system_job_count(paths)? > existing_count {
            return Ok(());
        }

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    Err(anyhow!("new finished startup System job was not recorded"))
}

fn ensure_reconciliation_dns_port(paths: &PvPaths) -> Result<()> {
    let mut database = Database::open(paths)?;
    if database
        .assigned_ports()?
        .into_iter()
        .any(|assignment| assignment.owner == PortOwner::Dns)
    {
        return Ok(());
    }

    let (dns_port, _tcp_listener, _udp_socket) = bind_loopback_tcp_udp_pair()?;
    database.assign_port(
        PortRequest::dns(dns_port, dns_port, dns_port),
        |candidate| candidate == dns_port,
    )?;

    Ok(())
}

fn bind_loopback_tcp_udp_pair() -> Result<(u16, TcpListener, UdpSocket)> {
    for _attempt in 0..100 {
        let tcp_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = tcp_listener.local_addr()?.port();
        let Ok(udp_socket) = UdpSocket::bind((Ipv4Addr::LOCALHOST, port)) else {
            continue;
        };

        return Ok((port, tcp_listener, udp_socket));
    }

    Err(anyhow!("could not bind a loopback TCP/UDP port pair"))
}

async fn request_lines(paths: &PvPaths, request: Value) -> Result<Vec<Value>> {
    let mut stream = UnixStream::connect(paths.daemon_socket()).await?;
    let request = serde_json::to_string(&request)?;
    stream.write_all(request.as_bytes()).await?;
    stream.write_all(b"\n").await?;

    let mut reader = BufReader::new(stream);
    let mut lines = Vec::new();

    loop {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line).await?;

        if bytes == 0 {
            break;
        }

        lines.push(serde_json::from_str(line.trim_end())?);
    }

    Ok(lines)
}

fn link_project(
    paths: &PvPaths,
    project_path: &Utf8Path,
    primary_hostname: &str,
    config_source: &str,
) -> Result<ProjectRecord> {
    let config_path = project_path.join("pv.yml");

    write_sensitive_file(&config_path, config_source)?;

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

fn link_resource_only_project(
    paths: &PvPaths,
    project_path: &Utf8Path,
    config_source: &str,
) -> Result<ProjectRecord> {
    let config_path = project_path.join("pv.yml");
    write_sensitive_file(&config_path, config_source)?;

    let mut database = Database::open(paths)?;
    let result = database.link_project_with_mode(
        LinkProjectInput {
            path: project_path.to_path_buf(),
            original_path: project_path.to_path_buf(),
            primary_hostname: "ignored.test".to_string(),
            config_path,
            desired_php_track: None,
            additional_hostnames: Vec::new(),
        },
        ProjectMode::ResourceOnly,
    )?;

    Ok(result.project)
}

fn write_project_config(project: &ProjectRecord, config_source: &str) -> Result<()> {
    write_sensitive_file(&project.config_path, config_source)?;

    Ok(())
}

fn update_project_primary_hostname(
    paths: &PvPaths,
    project: &ProjectRecord,
    primary_hostname: &str,
) -> Result<ProjectRecord> {
    let mut database = Database::open(paths)?;
    let result = database.link_project(LinkProjectInput {
        path: project.path.clone(),
        original_path: project.original_path.clone(),
        primary_hostname: primary_hostname.to_string(),
        config_path: project.config_path.clone(),
        desired_php_track: project.desired_php_track.clone(),
        additional_hostnames: project.additional_hostnames.clone(),
    })?;

    Ok(result.project)
}

fn seed_postgres_context(paths: &PvPaths, project: &ProjectRecord) -> Result<()> {
    let mut database = Database::open(paths)?;

    seed_postgres_resource_context_in_database(&mut database)?;
    database.replace_project_managed_resources(
        &project.id,
        &[ProjectManagedResourceInput {
            resource_name: "postgres".to_string(),
            track: "8.0".to_string(),
        }],
    )?;
    database.replace_project_resource_allocations(
        &project.id,
        "postgres",
        "8.0",
        &[ResourceAllocationInput {
            allocation_name: "app-db".to_string(),
            generated_name: "acme_test_app_db".to_string(),
        }],
    )?;
    database.mark_resource_allocation_ready(
        &project.id,
        "postgres",
        "8.0",
        "app-db",
        &env_context(&[("database", "acme_test_app_db")]),
    )?;

    Ok(())
}

fn seed_postgres_resource_context(paths: &PvPaths) -> Result<()> {
    seed_postgres_resource_context_for_track(paths, "8.0", "3306")
}

fn seed_postgres_resource_context_for_track(
    paths: &PvPaths,
    track: &str,
    port: &str,
) -> Result<()> {
    let mut database = Database::open(paths)?;
    seed_postgres_resource_context_for_track_in_database(&mut database, track, port)
}

fn seed_postgres_resource_context_in_database(database: &mut Database) -> Result<()> {
    seed_postgres_resource_context_for_track_in_database(database, "8.0", "3306")
}

fn seed_postgres_resource_context_for_track_in_database(
    database: &mut Database,
    track: &str,
    port: &str,
) -> Result<()> {
    database.record_managed_resource_track_env_context(
        "postgres",
        track,
        &env_context(&[
            ("host", "127.0.0.1"),
            ("password", "secret"),
            ("port", port),
            ("username", "root"),
        ]),
    )?;

    Ok(())
}

fn seed_manifest(paths: &PvPaths, default_track: &str) -> Result<()> {
    write_sensitive_file(
        &paths.downloads().join("manifest.json"),
        &test_manifest(default_track),
    )?;

    Ok(())
}

fn seed_local_ca(paths: &PvPaths) -> Result<GeneratedLocalCa> {
    let local_ca = platform::generate_local_ca()?;

    write_sensitive_file(&paths.ca_certificate(), &local_ca.certificate_pem)?;
    write_sensitive_file(&paths.ca_private_key(), &local_ca.private_key_pem)?;

    Ok(local_ca)
}

fn write_project_certificate_with_remaining_days(
    paths: &PvPaths,
    project: &ProjectRecord,
    local_ca: &GeneratedLocalCa,
    remaining_days: i64,
) -> Result<()> {
    let ca_key_pair = KeyPair::from_pem(&local_ca.private_key_pem)?;
    let issuer = Issuer::from_ca_cert_pem(&local_ca.certificate_pem, ca_key_pair)?;
    let primary_hostname = served_project_hostname(project)?;
    let mut params = CertificateParams::new(vec![primary_hostname.to_string()])?;
    let now = OffsetDateTime::now_utc();
    params.not_before = now - Duration::days(1);
    params.not_after = now + Duration::days(remaining_days);
    params
        .distinguished_name
        .push(DnType::CommonName, primary_hostname);
    params.use_authority_key_identifier_extension = true;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let key_pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?;
    let certificate = params.signed_by(&key_pair, &issuer)?;

    write_sensitive_file(
        &paths.project_tls_certificate(&project.id),
        &format!("{}{}", certificate.pem(), local_ca.certificate_pem),
    )?;
    write_sensitive_file(
        &paths.project_tls_private_key(&project.id),
        &key_pair.serialize_pem(),
    )?;

    Ok(())
}

fn served_project_hostname(project: &ProjectRecord) -> Result<&str> {
    project
        .primary_hostname
        .as_deref()
        .ok_or_else(|| anyhow!("expected Project `{}` to have a hostname", project.slug))
}

fn test_manifest(default_track: &str) -> String {
    json!({
        "schema_version": 1,
        "minimum_pv_version": "0.1.0",
        "resources": [
            {
                "name": "postgres",
                "default_track": default_track,
                "tracks": [
                    {
                        "name": "8.0",
                        "artifacts": [
                            {
                                "artifact_version": "8.0.42-pv1",
                                "upstream_version": "8.0.42",
                                "pv_build_revision": "pv1",
                                "platform": "darwin-arm64",
                                "url": "https://artifacts.example.test/postgres-8.0.42-pv1-darwin-arm64.tar.gz",
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
                                "artifact_version": "8.4.5-pv1",
                                "upstream_version": "8.4.5",
                                "pv_build_revision": "pv1",
                                "platform": "darwin-arm64",
                                "url": "https://artifacts.example.test/postgres-8.4.5-pv1-darwin-arm64.tar.gz",
                                "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                                "size": 12345,
                                "published_at": "2026-05-27T14:30:00Z"
                            }
                        ]
                    }
                ]
            },
            {
                "name": "php",
                "default_track": default_track,
                "tracks": [
                    {
                        "name": "8.0",
                        "artifacts": [
                            {
                                "artifact_version": "8.0.30-pv1",
                                "upstream_version": "8.0.30",
                                "pv_build_revision": "pv1",
                                "platform": "darwin-arm64",
                                "url": "https://artifacts.example.test/php-8.0.30-pv1-darwin-arm64.tar.gz",
                                "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
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
                                "sha256": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
                                "size": 12345,
                                "published_at": "2026-05-27T14:30:00Z"
                            }
                        ]
                    }
                ]
            }
        ]
    })
    .to_string()
}

fn env_context(entries: &[(&str, &str)]) -> EnvContextValues {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect::<BTreeMap<_, _>>()
}

fn read_project_config(project: &ProjectRecord) -> Result<String> {
    state::fs::read_to_string(&project.config_path).map_err(Into::into)
}

fn read_dotenv(project: &ProjectRecord) -> Result<String> {
    state::fs::read_to_string(&project.path.join(".env")).map_err(Into::into)
}

fn read_project_tls_files(paths: &PvPaths, project: &ProjectRecord) -> Result<(String, String)> {
    let certificate_pem = state::fs::read_to_string(&paths.project_tls_certificate(&project.id))?;
    let private_key_pem = state::fs::read_to_string(&paths.project_tls_private_key(&project.id))?;

    Ok((certificate_pem, private_key_pem))
}

fn assert_project_certificate_matches(
    certificate_pem: &str,
    private_key_pem: &str,
    primary_hostname: &str,
    local_ca: &GeneratedLocalCa,
) {
    assert!(
        platform::project_certificate_matches(
            certificate_pem,
            private_key_pem,
            primary_hostname,
            &local_ca.certificate_pem,
        ),
        "Project certificate should match the primary hostname and current local CA"
    );
}

fn certificate_pem_block_count(content: &str) -> usize {
    content.matches("-----BEGIN CERTIFICATE-----").count()
}

fn read_optional_dotenv(project: &ProjectRecord) -> Result<Option<String>> {
    match state::fs::read_to_string(&project.path.join(".env")) {
        Ok(content) => Ok(Some(content)),
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "daemon Project env tests set fixture permissions directly"
)]
fn set_file_mode(path: &Utf8Path, mode: u32) -> Result<()> {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "daemon Project env tests inspect fixture permissions directly"
)]
fn mode_string(path: &Utf8Path) -> Result<String> {
    let mode = std::fs::metadata(path)?.permissions().mode() & 0o777;

    Ok(format!("{mode:o}"))
}

fn latest_job(database: &Database, scope: &str) -> Result<JobRecord> {
    database
        .recent_jobs()?
        .into_iter()
        .find(|job| job.scope == scope)
        .ok_or_else(|| anyhow!("missing job for scope {scope}"))
}

fn assert_with_normalized_timestamps(
    name: &'static str,
    snapshot: impl std::fmt::Debug,
) -> Result<()> {
    let mut settings = Settings::clone_current();
    settings.add_filter(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", "<timestamp>");
    settings.add_filter(r"project:[a-z0-9]{10}", "project:<project_id>");
    settings.add_filter(r"projects/[a-z0-9]{10}/", "projects/<project_id>/");
    settings.add_filter(
        r#"project_id: "[a-z0-9]{10}""#,
        r#"project_id: "<project_id>""#,
    );
    settings.add_filter(
        "id: \"[a-z0-9]{10}\",\n        slug:",
        "id: \"<project_id>\",\n        slug:",
    );
    settings.add_filter(r"Project `[a-z0-9]{10}`", "Project `<project_id>`");
    settings.bind(|| {
        assert_debug_snapshot!(name, snapshot);
        Ok::<(), anyhow::Error>(())
    })
}

fn assert_with_normalized_timestamps_and_tempdir(
    name: &'static str,
    snapshot: impl std::fmt::Debug,
    tempdir: &Utf8Path,
) -> Result<()> {
    let mut settings = Settings::clone_current();
    settings.add_filter(tempdir.as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.add_filter(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", "<timestamp>");
    settings.add_filter(r"project:[a-z0-9]{10}", "project:<project_id>");
    settings.add_filter(r"projects/[a-z0-9]{10}/", "projects/<project_id>/");
    settings.add_filter(
        r#"project_id: "[a-z0-9]{10}""#,
        r#"project_id: "<project_id>""#,
    );
    settings.add_filter(
        "id: \"[a-z0-9]{10}\",\n        slug:",
        "id: \"<project_id>\",\n        slug:",
    );
    settings.add_filter(r"Project `[a-z0-9]{10}`", "Project `<project_id>`");
    settings.bind(|| {
        assert_debug_snapshot!(name, snapshot);
        Ok::<(), anyhow::Error>(())
    })
}
