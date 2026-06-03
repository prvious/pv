use std::collections::BTreeMap;
use std::io;
use std::os::unix::fs::PermissionsExt;

use anyhow::{Result, anyhow};
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::{Settings, assert_debug_snapshot};
use serde_json::{Value, json};
use state::{
    Database, EnvContextValues, JobRecord, LinkProjectInput, PortOwner,
    ProjectManagedResourceInput, ProjectRecord, PvPaths, ResourceAllocationInput, StateError,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

#[tokio::test]
async fn root_only_env_rendering_writes_dotenv_and_records_rendered_state() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        "env:\n  APP_URL: \"${project_url}\"\n  APP_NAME: acme\n",
    )?;

    let lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "root_only_env_rendering_writes_dotenv_and_records_rendered_state",
        (
            lines,
            read_dotenv(&project)?,
            database.project_env_observed_state(&project.id)?,
            database.recent_jobs()?,
        ),
    )?;

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
        r#"mysql:
  version: "8.0"
  allocations:
    app-db:
      env:
        DATABASE_URL: "mysql://${username}:${password}@${host}:${port}/${database}"
        DB_DATABASE: "${database}"
        DB_HOST: "${host}"
        DB_PORT: "${port}"
        DB_USERNAME: "${username}"
"#,
    )?;
    seed_mysql_context(&paths, &project)?;

    let lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "seeded_resource_and_allocation_contexts_render_dotenv",
        (
            lines,
            read_optional_dotenv(&project)?,
            database.project_managed_resources(&project.id)?,
            database.resource_allocations(&project.id, "mysql")?,
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
        r#"mysql:
  version: "8.0"
  allocations:
    app-db:
      env:
        DB_DATABASE: "${database}"
"#,
    )?;
    seed_mysql_context(&paths, &project)?;
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
            database.resource_allocations(&project.id, "mysql")?,
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
        "mysql:\n  version: \"8.0\"\n  env:\n    DB_HOST: \"${host}\"\n",
    )?;
    state::fs::write_sensitive_file(&project.path.join(".env"), "USER_VALUE=kept\n")?;

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
        "env:\n  APP_URL: \"${project_url}\"\nmysql:\n  version: \"8.0\"\n",
    )?;
    state::fs::write_sensitive_file(&project.path.join(".env"), "USER_VALUE=kept\n")?;

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
        r#"mysql:
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
            database.resource_allocations(&project.id, "mysql")?,
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
    state::fs::write_sensitive_file(
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
mysql:
  version: "8.0"
  env:
    DB_HOST: "${host}"
"#,
    )?;
    state::fs::write_sensitive_file(
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
            database.resource_allocations(&project.id, "mysql")?,
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
    state::fs::write_sensitive_file(&project.path.join(".env"), "APP_URL=https://user.test\n")?;

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
        r#"mysql:
  version: "8.0"
  allocations:
    analytics:
      env:
        DATABASE_URL: "mysql://${database}"
    app:
      env:
        DATABASE_URL: "mysql://${database}"
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
            database.resource_allocations(&project.id, "mysql")?,
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
            r#"mysql:
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
            database.resource_allocations(&project.id, "mysql")?,
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
mysql:
  version: "8.0"
  allocations:
    app-db:
      env:
        DB_DATABASE: "${database}"
"#,
    )?;
    seed_mysql_context(&paths, &project)?;
    let initial_lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;
    let managed_resources_before = database.project_managed_resources(&project.id)?;
    let allocations_before = database.resource_allocations(&project.id, "mysql")?;
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
    let allocations_after = database.resource_allocations(&project.id, "mysql")?;
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
mysql:
  version: "8.0"
  allocations:
    app-db:
      env:
        DB_DATABASE: "${database}"
"#,
    )?;
    seed_mysql_context(&paths, &project)?;
    let initial_lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;
    let managed_resources_before = database.project_managed_resources(&project.id)?;
    let allocations_before = database.resource_allocations(&project.id, "mysql")?;
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
    let allocations_after = database.resource_allocations(&project.id, "mysql")?;
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
    state::fs::write_sensitive_file(&project.path.join(".env"), "USER_VALUE=kept\n")?;

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
        r#"mysql:
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
            database.resource_allocations(&project.id, "mysql")?,
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
    state::fs::write_sensitive_file(
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
        "mysql:\n  version: latest\n  env:\n    DB_HOST: \"${host}\"\n",
    )?;
    seed_manifest(&paths, "8.0")?;
    seed_mysql_resource_context(&paths)?;

    let lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "latest_resource_track_resolves_default_track_before_state_and_dotenv_writes",
        (
            lines,
            read_project_config(&project)?,
            read_optional_dotenv(&project)?,
            database.project_managed_resources(&project.id)?,
            database.resource_allocations(&project.id, "mysql")?,
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
        "mysql:\n  version: latest\n  env:\n    DB_HOST: \"${host}\"\n    DB_PORT: \"${port}\"\n",
    )?;
    seed_manifest(&paths, "8.0")?;
    seed_mysql_resource_context(&paths)?;
    let initial_lines = run_project_reconciliation(&paths, &project).await?;

    seed_manifest(&paths, "8.4")?;
    seed_mysql_resource_context_for_track(&paths, "8.4", "3406")?;
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
        "mysql:\n  env:\n    DB_HOST: \"${host}\"\n",
    )?;
    seed_manifest(&paths, "8.0")?;
    seed_mysql_resource_context(&paths)?;

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
        "mysql:\n  env:\n    DB_HOST: \"${host}\"\n    DB_PORT: \"${port}\"\n",
    )?;
    seed_manifest(&paths, "8.0")?;
    seed_mysql_resource_context(&paths)?;
    let initial_lines = run_project_reconciliation(&paths, &project).await?;

    seed_manifest(&paths, "8.4")?;
    seed_mysql_resource_context_for_track(&paths, "8.4", "3406")?;
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
        "mysql:\n",
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
            database.resource_allocations(&project.id, "mysql")?,
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
    let daemon = daemon::RunningDaemon::start(paths.clone()).await?;
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

    let shutdown_result = daemon.shutdown().await;
    let mut database = Database::open(paths)?;
    database.release_port(PortOwner::Dns)?;
    shutdown_result?;

    Ok(lines)
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

fn seed_mysql_context(paths: &PvPaths, project: &ProjectRecord) -> Result<()> {
    let mut database = Database::open(paths)?;

    seed_mysql_resource_context_in_database(&mut database)?;
    database.replace_project_managed_resources(
        &project.id,
        &[ProjectManagedResourceInput {
            resource_name: "mysql".to_string(),
            track: "8.0".to_string(),
        }],
    )?;
    database.replace_project_resource_allocations(
        &project.id,
        "mysql",
        "8.0",
        &[ResourceAllocationInput {
            allocation_name: "app-db".to_string(),
            generated_name: "acme_test_app_db".to_string(),
        }],
    )?;
    database.mark_resource_allocation_ready(
        &project.id,
        "mysql",
        "8.0",
        "app-db",
        &env_context(&[("database", "acme_test_app_db")]),
    )?;

    Ok(())
}

fn seed_mysql_resource_context(paths: &PvPaths) -> Result<()> {
    seed_mysql_resource_context_for_track(paths, "8.0", "3306")
}

fn seed_mysql_resource_context_for_track(paths: &PvPaths, track: &str, port: &str) -> Result<()> {
    let mut database = Database::open(paths)?;
    seed_mysql_resource_context_for_track_in_database(&mut database, track, port)
}

fn seed_mysql_resource_context_in_database(database: &mut Database) -> Result<()> {
    seed_mysql_resource_context_for_track_in_database(database, "8.0", "3306")
}

fn seed_mysql_resource_context_for_track_in_database(
    database: &mut Database,
    track: &str,
    port: &str,
) -> Result<()> {
    database.record_managed_resource_track_env_context(
        "mysql",
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
    state::fs::write_sensitive_file(
        &paths.downloads().join("manifest.json"),
        &test_manifest(default_track),
    )?;

    Ok(())
}

fn test_manifest(default_track: &str) -> String {
    json!({
        "schema_version": 1,
        "minimum_pv_version": "0.1.0",
        "resources": [
            {
                "name": "mysql",
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
                                "url": "https://artifacts.example.test/mysql-8.0.42-pv1-darwin-arm64.tar.gz",
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
                                "url": "https://artifacts.example.test/mysql-8.4.5-pv1-darwin-arm64.tar.gz",
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
    settings.add_filter(
        r#"project_id: "[a-z0-9]{10}""#,
        r#"project_id: "<project_id>""#,
    );
    settings.add_filter(r"Project `[a-z0-9]{10}`", "Project `<project_id>`");
    settings.bind(|| {
        assert_debug_snapshot!(name, snapshot);
        Ok::<(), anyhow::Error>(())
    })
}
