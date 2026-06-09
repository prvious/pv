use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;

use anyhow::{Result, bail};
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::{Settings, assert_debug_snapshot};
use resources::RuntimeArtifactAdapter;
use state::{Database, LinkProjectInput, ProjectRecord, PvPaths};

use super::ManagedResourceRuntimeAdapter;

const MYSQL_TRACK: &str = "8.0";
const MYSQL_ARTIFACT_VERSION: &str = "8.0.35-pv1";
const MYSQL_ARCHIVE_FILE_NAME: &str = "mysql-8.0.35-pv1-any.tar.gz";
const OFFLINE_TEST_MANIFEST_URL: &str = "https://127.0.0.1:9/manifest.json";

#[test]
fn mysql_runtime_port_prefers_3306() -> Result<()> {
    let adapter = super::mysql::MysqlRuntimeAdapter::new();
    let [port] = adapter.port_specs() else {
        bail!("expected one MySQL port spec");
    };

    assert_eq!(port.name, "mysql");
    assert_eq!(port.preferred_port, 3306);

    Ok(())
}

#[test]
fn mysql_runtime_uses_resources_artifact_adapter() -> Result<()> {
    let adapter = super::mysql::MysqlRuntimeAdapter::new();
    let artifact_adapter: RuntimeArtifactAdapter = adapter.artifact_adapter()?;

    assert_eq!(artifact_adapter, resources::mysql_adapter()?);

    Ok(())
}

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
async fn mysql_reconciliation_creates_database_allocation_and_renders_env() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project_with_mysql_database_env(&paths, &tempdir.path().join("project"))?;
    let admin = super::mysql::RecordingMysqlAdmin::default();
    let catalog = super::mysql::mysql_runtime_catalog_with_recording_admin(
        super::DEFAULT_MANIFEST_URL,
        admin.clone(),
    )?;
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
    stop_mysql_runtime(&paths, &project, &catalog).await?;

    Ok(())
}

#[tokio::test]
async fn mysql_project_demand_installs_missing_fixture_track_before_start() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project_with_mysql_database_env(&paths, &tempdir.path().join("project"))?;
    let admin = super::mysql::RecordingMysqlAdmin::default();
    let catalog = super::mysql::mysql_runtime_catalog_with_recording_admin(
        OFFLINE_TEST_MANIFEST_URL,
        admin.clone(),
    )?;
    seed_mysql_cached_fixture(&paths, tempdir.path())?;

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
        "mysql_project_demand_installs_missing_fixture_track_before_start",
        snapshot,
    )?;
    stop_mysql_runtime(&paths, &project, &catalog).await?;

    Ok(())
}

#[tokio::test]
async fn mysql_reconciliation_reuses_admin_env_and_ready_allocation() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project_with_mysql_database_env(&paths, &tempdir.path().join("project"))?;
    let admin = super::mysql::RecordingMysqlAdmin::default();
    let catalog = super::mysql::mysql_runtime_catalog_with_recording_admin(
        super::DEFAULT_MANIFEST_URL,
        admin,
    )?;
    seed_mysql_fixture_artifact(&paths, MYSQL_TRACK)?;

    run_project_reconciliation(&paths, &project, &catalog).await?;
    let first = mysql_track_and_allocation(&paths, &project)?;

    run_project_reconciliation(&paths, &project, &catalog).await?;
    let second = mysql_track_and_allocation(&paths, &project)?;

    assert_eq!(first.0.env, second.0.env);
    assert_eq!(first.1.generated_name, second.1.generated_name);
    assert_eq!(first.1.env, second.1.env);
    assert_eq!(second.1.status, state::ResourceAllocationStatus::Ready);
    stop_mysql_runtime(&paths, &project, &catalog).await?;

    Ok(())
}

async fn run_project_reconciliation(
    paths: &PvPaths,
    project: &ProjectRecord,
    catalog: &super::ManagedResourceRuntimeCatalog,
) -> Result<()> {
    let mut database = Database::open(paths)?;

    crate::project_env::reconcile_project_env_with_catalog(
        paths,
        &mut database,
        &project.id,
        catalog,
    )
    .await?;

    Ok(())
}

async fn stop_mysql_runtime(
    paths: &PvPaths,
    project: &ProjectRecord,
    catalog: &super::ManagedResourceRuntimeCatalog,
) -> Result<()> {
    write_project_config(
        project,
        r#"env:
  APP_URL: "${project_url}"
"#,
    )?;

    let _result = run_project_reconciliation(paths, project, catalog).await;

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
  APP_URL: "${project_url}"
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

fn read_runtime_metadata(paths: &PvPaths, track: &str) -> Result<serde_json::Value> {
    let content = state::fs::read_to_string(&paths.resource_runtime_metadata("mysql", track))?;

    serde_json::from_str(&content).map_err(Into::into)
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
    let executable = release_path.join("bin/mysqld");

    state::fs::write_sensitive_file(&executable, mysql_fixture_script())?;
    set_executable(&executable)?;
    let mut database = Database::open(paths)?;
    database.record_managed_resource_track_installed(
        "mysql",
        track,
        MYSQL_ARTIFACT_VERSION,
        &release_path,
    )?;

    Ok(())
}

fn seed_mysql_cached_fixture(paths: &PvPaths, tempdir: &Utf8Path) -> Result<()> {
    let archive_path = tempdir.join(MYSQL_ARCHIVE_FILE_NAME);

    create_mysql_archive(tempdir, &archive_path)?;
    let sha256 = sha256_file(&archive_path)?;
    let cache_path = paths
        .downloads()
        .join(format!("{sha256}-{MYSQL_ARCHIVE_FILE_NAME}"));

    copy_file(&archive_path, &cache_path)?;
    let size = file_size(&cache_path)?;
    let manifest = mysql_manifest(&sha256, size);

    state::fs::write_sensitive_file(&paths.downloads().join("manifest.json"), &manifest)?;

    Ok(())
}

fn create_mysql_archive(tempdir: &Utf8Path, archive_path: &Utf8Path) -> Result<()> {
    let archive_parent = tempdir.join("archive-root");
    let root_name = format!("mysql-{MYSQL_ARTIFACT_VERSION}");
    let root = archive_parent.join(&root_name);
    let executable = root.join("bin/mysqld");

    state::fs::write_sensitive_file(&executable, mysql_fixture_script())?;
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

fn mysql_manifest(sha256: &str, size: u64) -> String {
    let dummy_sha256 = "0000000000000000000000000000000000000000000000000000000000000000";

    format!(
        r#"{{
  "schema_version": 1,
  "minimum_pv_version": "0.1.0",
  "resources": [
    {{
      "name": "mysql",
      "default_track": "{MYSQL_TRACK}",
      "tracks": [
        {{
          "name": "{MYSQL_TRACK}",
          "artifacts": [
            {{
              "artifact_version": "{MYSQL_ARTIFACT_VERSION}",
              "upstream_version": "8.0.35",
              "pv_build_revision": "1",
              "platform": "any",
              "url": "https://artifacts.example.test/{MYSQL_ARCHIVE_FILE_NAME}",
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

fn mysql_fixture_script() -> &'static str {
    r#"#!/bin/sh
set -eu

datadir=""
port=""
first_arg="${1:-}"

while [ "$#" -gt 0 ]; do
  case "$1" in
    --initialize-insecure)
      initialize=1
      shift
      ;;
    --datadir)
      datadir="$2"
      shift 2
      ;;
    --basedir)
      shift 2
      ;;
    --port)
      port="$2"
      shift 2
      ;;
    --bind-address|--socket)
      shift 2
      ;;
    --no-defaults)
      shift
      ;;
    *)
      shift
      ;;
  esac
done

if [ "${initialize:-0}" = "1" ]; then
  if [ "${first_arg:-}" != "--no-defaults" ]; then
    echo "mysqld initialization must start with --no-defaults" >&2
    exit 64
  fi
  mkdir -p "$datadir/mysql"
  exit 0
fi

python3 - "$port" <<'PY'
import signal
import socketserver
import sys

class Handler(socketserver.BaseRequestHandler):
    def handle(self):
        self.request.recv(1024)

class TcpServer(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True

def stop(_signum, _frame):
    sys.exit(0)

signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)

server = TcpServer(("127.0.0.1", int(sys.argv[1])), Handler)
server.serve_forever()
PY
"#
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
    reason = "daemon MySQL tests shell out to build archive fixtures without extra dev-dependencies"
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
    reason = "daemon MySQL tests seed cached artifact fixtures directly"
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
    reason = "daemon MySQL tests read fixture archive metadata for manifest size"
)]
fn file_size(path: &Utf8Path) -> Result<u64> {
    Ok(std::fs::metadata(path)?.len())
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

#[expect(
    clippy::disallowed_methods,
    reason = "daemon MySQL tests set fixture executable bits directly"
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
