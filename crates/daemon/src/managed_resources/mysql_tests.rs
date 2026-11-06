use std::collections::BTreeMap;

use anyhow::{Result, bail};
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::{Settings, assert_debug_snapshot};
use pv_fake::Persona;
use serde_json::{Value, json};
use state::{Database, LinkProjectInput, ProjectRecord, PvPaths};

use super::{ManagedResourceRuntimeAdapter, tests::ManagedResourceFixtureGuard};

const MYSQL_TRACK: &str = "8.0";
const MYSQL_ARTIFACT_VERSION: &str = "8.0.35-pv1";

#[test]
fn mysql_runtime_arguments_disable_x_protocol() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let adapter = super::mysql::MysqlRuntimeAdapter::new();
    let context = super::ManagedResourceRuntimeContext {
        resource_name: "mysql".to_string(),
        track: MYSQL_TRACK.to_string(),
        artifact_path: tempdir.path().join("mysql-artifact"),
        data_dir: paths.resource_data_dir("mysql", MYSQL_TRACK),
        ports: BTreeMap::from([("mysql".to_string(), 3307)]),
        env: BTreeMap::new(),
        postgres_preload_libraries: Vec::new(),
    };

    let spec =
        super::ManagedResourceRuntimeAdapter::build_process_spec(&adapter, &paths, &context)?;

    assert_eq!(
        spec.arguments.first().map(String::as_str),
        Some("--no-defaults")
    );
    assert!(
        spec.arguments
            .iter()
            .any(|argument| argument == "--mysqlx=0"),
        "MySQL runtime args must disable X Protocol: {:#?}",
        spec.arguments
    );
    assert_mysql_snapshot(
        tempdir.path(),
        "mysql_runtime_arguments_disable_x_protocol",
        spec.arguments,
    )?;

    Ok(())
}

#[tokio::test]
async fn mysql_allocation_cleanup_drops_only_the_named_database() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let admin = super::mysql::RecordingMysqlAdmin::default();
    let adapter = super::mysql::MysqlRuntimeAdapter::with_recording_admin(admin.clone())?;
    let context = super::ManagedResourceRuntimeContext {
        resource_name: "mysql".to_string(),
        track: MYSQL_TRACK.to_string(),
        artifact_path: tempdir.path().join("mysql-artifact"),
        data_dir: paths.resource_data_dir("mysql", MYSQL_TRACK),
        ports: BTreeMap::from([("mysql".to_string(), 3307)]),
        env: BTreeMap::from([
            ("password".to_string(), "secret".to_string()),
            ("username".to_string(), "pv_root".to_string()),
        ]),
        postgres_preload_libraries: Vec::new(),
    };

    adapter
        .delete_allocation(&context, &context.env, "acme_app_db")
        .await?;

    assert_debug_snapshot!(admin.operations().await?);

    Ok(())
}

#[tokio::test]
async fn mysql_reconciliation_creates_database_allocation_and_renders_env() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project_with_mysql_database_env(&paths, &tempdir.path().join("project"))?;
    let admin = super::mysql::RecordingMysqlAdmin::default();
    let catalog = super::mysql::mysql_runtime_catalog_with_recording_admin(
        resources::default_artifact_manifest_url(),
        admin.clone(),
    )?;
    let mut runtimes = ManagedResourceFixtureGuard::new(&paths)?;
    seed_mysql_fixture_artifact(&paths, MYSQL_TRACK)?;

    run_project_reconciliation(&paths, &project, &catalog).await?;
    let snapshot = {
        let database = Database::open(&paths)?;

        (
            admin.operations().await?,
            read_dotenv(&project)?,
            database.managed_resource_track("mysql", MYSQL_TRACK)?,
            database.assigned_ports()?,
            database.resource_allocations(&project.id, "mysql")?,
            database.runtime_observed_states()?,
            read_runtime_metadata(&paths, MYSQL_TRACK)?,
            mysql_system_database_initialized(&paths, MYSQL_TRACK)?,
        )
    };

    assert_mysql_snapshot(
        tempdir.path(),
        "mysql_reconciliation_creates_database_allocation_and_renders_env",
        snapshot,
    )?;
    runtimes.cleanup().await?;

    Ok(())
}

#[tokio::test]
async fn mysql_reconciliation_reuses_admin_env_and_ready_allocation() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project_with_mysql_database_env(&paths, &tempdir.path().join("project"))?;
    let admin = super::mysql::RecordingMysqlAdmin::default();
    let catalog = super::mysql::mysql_runtime_catalog_with_recording_admin(
        resources::default_artifact_manifest_url(),
        admin,
    )?;
    let mut runtimes = ManagedResourceFixtureGuard::new(&paths)?;
    seed_mysql_fixture_artifact(&paths, MYSQL_TRACK)?;

    run_project_reconciliation(&paths, &project, &catalog).await?;
    let first = mysql_track_and_allocation(&paths, &project)?;

    run_project_reconciliation(&paths, &project, &catalog).await?;
    let second = mysql_track_and_allocation(&paths, &project)?;

    assert_eq!(first.0.env, second.0.env);
    assert_eq!(first.1.generated_name, second.1.generated_name);
    assert_eq!(first.1.env, second.1.env);
    assert_eq!(second.1.status, state::ResourceAllocationStatus::Ready);
    runtimes.cleanup().await?;

    Ok(())
}

async fn run_project_reconciliation(
    paths: &PvPaths,
    project: &ProjectRecord,
    catalog: &super::ManagedResourceRuntimeCatalog,
) -> Result<()> {
    crate::project_env::reconcile_project_env_with_catalog(paths, &project.id, catalog).await?;

    Ok(())
}

fn mysql_track_and_allocation(
    paths: &PvPaths,
    project: &ProjectRecord,
) -> Result<(
    state::ManagedResourceTrackRecord,
    state::ResourceAllocationRecord,
)> {
    let database = Database::open(paths)?;
    let track = database.managed_resource_track("mysql", MYSQL_TRACK)?;
    let allocations = database.resource_allocations(&project.id, "mysql")?;
    let [allocation] = allocations.as_slice() else {
        bail!("expected one MySQL allocation, got {allocations:#?}");
    };

    Ok((track, allocation.clone()))
}

fn link_project_with_mysql_database_env(
    paths: &PvPaths,
    project_path: &Utf8Path,
) -> Result<ProjectRecord> {
    link_project(
        paths,
        project_path,
        "acme.test",
        r#"env:
  APP_URL: "${url}"
mysql:
  version: "8.0"
  env:
    DB_HOST: "${host}"
    DB_PORT: "${port}"
    DB_USERNAME: "${username}"
    DB_PASSWORD: "${password}"
  allocations:
    app-db:
      env:
        DB_DATABASE: "${database}"
        DATABASE_URL: "${url}"
"#,
    )
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
    })?;

    Ok(result.project)
}

fn read_dotenv(project: &ProjectRecord) -> Result<String> {
    state::fs::read_to_string(&project.path.join(".env")).map_err(Into::into)
}

fn read_runtime_metadata(paths: &PvPaths, track: &str) -> Result<Value> {
    let content = state::fs::read_to_string(&paths.resource_runtime_metadata("mysql", track))?;
    let mut metadata: Value = serde_json::from_str(&content)?;
    if let Some(monitor_instance) = metadata.get_mut("monitor_instance") {
        *monitor_instance = json!("<monitor-instance>");
    }

    Ok(metadata)
}

fn mysql_system_database_initialized(paths: &PvPaths, track: &str) -> Result<bool> {
    path_exists(&paths.resource_data_dir("mysql", track).join("mysql"))
}

fn seed_mysql_fixture_artifact(paths: &PvPaths, track: &str) -> Result<()> {
    let release_path = paths
        .resources()
        .join("mysql")
        .join(track)
        .join(format!("releases/{MYSQL_ARTIFACT_VERSION}"));
    pv_fake::install(&release_path.join("bin/mysqld"), Persona::Mysqld)?;
    let mut database = Database::open(paths)?;
    database.record_managed_resource_track_installed(
        "mysql",
        track,
        MYSQL_ARTIFACT_VERSION,
        &release_path,
    )?;

    Ok(())
}

fn assert_mysql_snapshot(
    tempdir: &Utf8Path,
    name: &'static str,
    snapshot: impl std::fmt::Debug,
) -> Result<()> {
    let mut settings = Settings::clone_current();

    settings.add_filter(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", "<timestamp>");
    settings.add_filter(&regex_literal(tempdir.as_str()), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.add_filter(
        r#"project_id: "[a-z0-9]{10}""#,
        r#"project_id: "<project_id>""#,
    );
    settings.add_filter(r#"id: "allocation_[0-9]{6}""#, r#"id: "<allocation_id>""#);
    settings.add_filter(r"Number\(\d+\)", "Number(<pid>)");
    settings.add_filter(r"127\.0\.0\.1:\d+", "127.0.0.1:<mysql_port>");
    settings.add_filter(r"DB_PORT=\d+", "DB_PORT=<mysql_port>");
    settings.add_filter(r#""port": "\d+""#, r#""port": "<mysql_port>""#);
    settings.add_filter(r#"String\("\d+"\)"#, r#"String("<mysql_port>")"#);
    settings.add_filter(r#""--port","\d+""#, r#""--port","<mysql_port>""#);
    settings.add_filter(r"port: \d+", "port: <mysql_port>");
    settings.add_filter(r"[0-9a-f]{32}", "<generated_mysql_password>");
    settings.bind(|| {
        assert_debug_snapshot!(name, snapshot);
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "daemon MySQL tests assert fixture data directory initialization directly"
)]
fn path_exists(path: &Utf8Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
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
