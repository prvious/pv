use std::collections::BTreeMap;
use std::io;

use anyhow::{Result, anyhow};
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::{Settings, assert_debug_snapshot};
use serde_json::{Value, json};
use state::{
    Database, EnvContextValues, JobRecord, LinkProjectInput, ProjectManagedResourceInput,
    ProjectRecord, PvPaths, ResourceAllocationInput, StateError,
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
async fn latest_resource_track_fails_before_state_or_dotenv_writes() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        "mysql:\n  version: latest\n  env:\n    DB_HOST: \"${host}\"\n",
    )?;

    let lines = run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "latest_resource_track_fails_before_state_or_dotenv_writes",
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

    daemon.shutdown().await?;

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

fn seed_mysql_context(paths: &PvPaths, project: &ProjectRecord) -> Result<()> {
    let mut database = Database::open(paths)?;

    database.record_managed_resource_track_env_context(
        "mysql",
        "8.0",
        &env_context(&[
            ("host", "127.0.0.1"),
            ("password", "secret"),
            ("port", "3306"),
            ("username", "root"),
        ]),
    )?;
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
        "app-db",
        &env_context(&[("database", "acme_test_app_db")]),
    )?;

    Ok(())
}

fn env_context(entries: &[(&str, &str)]) -> EnvContextValues {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect::<BTreeMap<_, _>>()
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
