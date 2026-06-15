use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::OsString;
use std::io;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use cli::{Environment, run_with_environment};
use insta::{Settings, assert_debug_snapshot};
use resources::{ResourceHttpClient, ResourcesError, TargetPlatform};
use state::{
    Database, ManagedResourceTrackRecord, PortRequest, PvPaths, RuntimeObservedStatus,
    RuntimeSubject,
};

const MANIFEST_URL: &str = "https://artifacts.example.test/manifest.json";
const RESOURCE_NAME: &str = "postgres";
const DEFAULT_TRACK: &str = "16";
const OLD_VERSION: &str = "16.3-pv1";
const NEW_VERSION: &str = "16.4-pv1";
const EXECUTABLE_PATH: &str = "bin/postgres";

#[derive(Debug)]
struct TestEnvironment {
    home: PathBuf,
    current_dir: PathBuf,
    client: ScriptedClient,
}

impl TestEnvironment {
    fn new(home: &Utf8Path, current_dir: &Utf8Path, client: ScriptedClient) -> Self {
        Self {
            home: home.as_std_path().to_path_buf(),
            current_dir: current_dir.as_std_path().to_path_buf(),
            client,
        }
    }

    fn text_request_count(&self) -> usize {
        self.client.text_request_count()
    }

    fn byte_request_count(&self) -> usize {
        self.client.byte_request_count()
    }
}

impl Environment for TestEnvironment {
    fn var_os(&self, _key: &str) -> Option<OsString> {
        None
    }

    fn home_dir(&self) -> Option<PathBuf> {
        Some(self.home.clone())
    }

    fn current_dir(&self) -> io::Result<PathBuf> {
        Ok(self.current_dir.clone())
    }

    fn current_exe(&self) -> io::Result<PathBuf> {
        Ok(PathBuf::from("/bin/pv"))
    }

    fn stdin_is_terminal(&self) -> bool {
        false
    }

    fn read_line(&self) -> io::Result<String> {
        Ok(String::new())
    }

    fn open_url(&self, _url: &str) -> io::Result<()> {
        Ok(())
    }

    fn artifact_manifest_url(&self) -> Option<String> {
        Some(MANIFEST_URL.to_string())
    }

    fn resource_http_client(&self) -> Option<&dyn ResourceHttpClient> {
        Some(&self.client)
    }

    fn target_platform(&self) -> Option<TargetPlatform> {
        Some(TargetPlatform::DarwinArm64)
    }
}

#[test]
fn postgres_install_uses_manifest_default_and_installs_without_network_download()
-> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let artifact = fixture_artifact(NEW_VERSION);
    prepare_existing_release(&home, DEFAULT_TRACK, &artifact)?;
    let environment = TestEnvironment::new(
        &home,
        &current_dir,
        ScriptedClient::new().with_text(&resource_manifest(DEFAULT_TRACK, &[&artifact])),
    );

    let output = run_pv(&["postgres:install"], &environment)?;
    let database = Database::open(&pv_paths(&home))?;
    let records = managed_resource_records(&database)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_resource_snapshot(
        "postgres_install_uses_manifest_default_and_installs_without_network_download",
        tempdir.path(),
        &(
            output,
            resource_record_snapshots(&records, tempdir.path())?,
            environment.text_request_count(),
            environment.byte_request_count(),
        ),
    );

    Ok(())
}

#[test]
fn pg_alias_install_records_canonical_postgres_resource() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let artifact = fixture_artifact(NEW_VERSION);
    prepare_existing_release(&home, DEFAULT_TRACK, &artifact)?;
    let environment = TestEnvironment::new(
        &home,
        &current_dir,
        ScriptedClient::new().with_text(&resource_manifest(DEFAULT_TRACK, &[&artifact])),
    );

    let output = run_pv(&["pg:install", DEFAULT_TRACK], &environment)?;
    let database = Database::open(&pv_paths(&home))?;
    let records = managed_resource_records(&database)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_resource_snapshot(
        "pg_alias_install_records_canonical_postgres_resource",
        tempdir.path(),
        &(output, resource_record_snapshots(&records, tempdir.path())?),
    );

    Ok(())
}

#[test]
fn postgres_update_updates_installed_tracks() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let old_artifact = fixture_artifact(OLD_VERSION);
    let new_artifact = fixture_artifact(NEW_VERSION);
    record_installed_resource(&home, DEFAULT_TRACK, &old_artifact)?;
    prepare_existing_release(&home, DEFAULT_TRACK, &new_artifact)?;
    let environment = TestEnvironment::new(
        &home,
        &current_dir,
        ScriptedClient::new().with_text(&resource_manifest(DEFAULT_TRACK, &[&new_artifact])),
    );

    let output = run_pv(&["pg:update"], &environment)?;
    let database = Database::open(&pv_paths(&home))?;
    let records = managed_resource_records(&database)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_resource_snapshot(
        "postgres_update_updates_installed_tracks",
        tempdir.path(),
        &(
            output,
            resource_record_snapshots(&records, tempdir.path())?,
            environment.text_request_count(),
            environment.byte_request_count(),
        ),
    );

    Ok(())
}

#[test]
fn postgres_list_reports_running_state_ports_and_usage() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = pv_paths(&home);
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let artifact = fixture_artifact(NEW_VERSION);
    record_installed_resource(&home, DEFAULT_TRACK, &artifact)?;
    seed_running_resource(&paths, DEFAULT_TRACK, "tcp", 5432)?;
    let environment = TestEnvironment::new(&home, &current_dir, ScriptedClient::new());

    let output = run_pv(&["pg:list"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_resource_snapshot(
        "postgres_list_reports_running_state_ports_and_usage",
        tempdir.path(),
        &output,
    );

    Ok(())
}

#[test]
fn postgres_uninstall_force_prune_queues_removal_intent() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let artifact = fixture_artifact(NEW_VERSION);
    record_installed_resource(&home, DEFAULT_TRACK, &artifact)?;
    let environment = TestEnvironment::new(&home, &current_dir, ScriptedClient::new());

    let output = run_pv(
        &["pg:uninstall", DEFAULT_TRACK, "--force", "--prune"],
        &environment,
    )?;
    let database = Database::open(&pv_paths(&home))?;
    let records = managed_resource_records(&database)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_resource_snapshot(
        "postgres_uninstall_force_prune_queues_removal_intent",
        tempdir.path(),
        &(output, resource_record_snapshots(&records, tempdir.path())?),
    );

    Ok(())
}

#[derive(Debug)]
#[expect(
    dead_code,
    reason = "snapshot-only structure is read through derived Debug"
)]
struct RunOutput {
    exit_code: ExitCode,
    stdout: String,
    stderr: String,
}

fn run_pv(args: &[&str], environment: &impl Environment) -> anyhow::Result<RunOutput> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let args = std::iter::once("pv").chain(args.iter().copied());
    let exit_code = run_with_environment(args, environment, &mut stdout, &mut stderr)?;

    Ok(RunOutput {
        exit_code,
        stdout: String::from_utf8(stdout)?,
        stderr: String::from_utf8(stderr)?,
    })
}

fn managed_resource_records(
    database: &Database,
) -> anyhow::Result<Vec<ManagedResourceTrackRecord>> {
    Ok(database
        .managed_resource_tracks()?
        .into_iter()
        .filter(|record| record.resource_name == RESOURCE_NAME)
        .collect())
}

#[derive(Debug)]
#[expect(
    dead_code,
    reason = "snapshot-only structure is read through derived Debug"
)]
struct ResourceRecordSnapshot {
    resource_name: String,
    track: String,
    desired_state: String,
    installed_version: Option<String>,
    current_artifact_path: Option<String>,
    usage_count: i64,
    removal_prune: bool,
    removal_force: bool,
}

fn resource_record_snapshots(
    records: &[ManagedResourceTrackRecord],
    root: &Utf8Path,
) -> anyhow::Result<Vec<ResourceRecordSnapshot>> {
    records
        .iter()
        .map(|record| resource_record_snapshot(record, root))
        .collect()
}

fn resource_record_snapshot(
    record: &ManagedResourceTrackRecord,
    root: &Utf8Path,
) -> anyhow::Result<ResourceRecordSnapshot> {
    Ok(ResourceRecordSnapshot {
        resource_name: record.resource_name.clone(),
        track: record.track.clone(),
        desired_state: format!("{:?}", record.desired_state),
        installed_version: record.installed_version.clone(),
        current_artifact_path: record
            .current_artifact_path
            .as_ref()
            .map(|path| path.strip_prefix(root).map(Utf8Path::to_string))
            .transpose()?,
        usage_count: record.usage_count,
        removal_prune: record.removal_prune,
        removal_force: record.removal_force,
    })
}

#[derive(Debug)]
struct FixtureArtifact {
    version: String,
    platform: String,
}

fn fixture_artifact(version: &str) -> FixtureArtifact {
    FixtureArtifact {
        version: version.to_string(),
        platform: TargetPlatform::DarwinArm64.as_str().to_string(),
    }
}

fn prepare_existing_release(
    home: &Utf8Path,
    track: &str,
    artifact: &FixtureArtifact,
) -> anyhow::Result<()> {
    let release = release_path(home, track, artifact);
    let executable = release.join(EXECUTABLE_PATH);
    let parent = executable
        .parent()
        .ok_or_else(|| anyhow::anyhow!("fixture executable has no parent: {executable}"))?;
    create_dir(parent)?;
    write_file(&executable, "fixture executable\n")?;
    write_file(&release.join("bin/initdb"), "fixture initdb\n")?;
    write_file(
        &release.join("share/postgresql/postgres.bki"),
        "fixture bki\n",
    )
}

fn record_installed_resource(
    home: &Utf8Path,
    track: &str,
    artifact: &FixtureArtifact,
) -> anyhow::Result<()> {
    prepare_existing_release(home, track, artifact)?;
    let release = release_path(home, track, artifact);
    let mut database = Database::open(&pv_paths(home))?;
    database.record_managed_resource_track_installed(
        RESOURCE_NAME,
        track,
        &artifact.version,
        &release,
    )?;

    Ok(())
}

fn release_path(home: &Utf8Path, track: &str, artifact: &FixtureArtifact) -> Utf8PathBuf {
    pv_paths(home)
        .resources()
        .join(RESOURCE_NAME)
        .join(track)
        .join("releases")
        .join(&artifact.version)
}

fn seed_running_resource(
    paths: &PvPaths,
    track: &str,
    port_name: &str,
    port: u16,
) -> anyhow::Result<()> {
    let mut database = Database::open(paths)?;
    database.assign_port(
        PortRequest::resource_port(RESOURCE_NAME, track, port_name, port, port, port),
        |_| true,
    )?;
    database.record_runtime_observed_snapshot(
        RuntimeSubject::Resource {
            name: RESOURCE_NAME.to_string(),
            track: track.to_string(),
        },
        RuntimeObservedStatus::Running,
        Some("Managed Resource runtime is ready"),
    )?;

    Ok(())
}

fn resource_manifest(default_track: &str, artifacts: &[&FixtureArtifact]) -> String {
    let artifacts = artifacts
        .iter()
        .map(|artifact| {
            format!(
                r#"{{
              "artifact_version": "{}",
              "upstream_version": "{}",
              "pv_build_revision": "1",
              "platform": "{}",
              "url": "https://artifacts.example.test/{}-{}-{}.tar.gz",
              "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
              "size": 0,
              "published_at": "2026-05-26T13:30:00Z"
            }}"#,
                artifact.version,
                artifact.version.trim_end_matches("-pv1"),
                artifact.platform,
                RESOURCE_NAME,
                artifact.version,
                artifact.platform,
            )
        })
        .collect::<Vec<_>>()
        .join(",");

    format!(
        r#"
{{
  "schema_version": 1,
  "minimum_pv_version": "0.1.0",
  "resources": [
    {{
      "name": "{RESOURCE_NAME}",
      "default_track": "{default_track}",
      "tracks": [
        {{
          "name": "{default_track}",
          "artifacts": [
            {artifacts}
          ]
        }}
      ]
    }}
  ]
}}
"#
    )
}

#[derive(Debug)]
struct ScriptedClient {
    text_responses: RefCell<VecDeque<Result<String, ResourcesError>>>,
    text_request_count: Cell<usize>,
    byte_request_count: Cell<usize>,
}

impl ScriptedClient {
    fn new() -> Self {
        Self {
            text_responses: RefCell::new(VecDeque::new()),
            text_request_count: Cell::new(0),
            byte_request_count: Cell::new(0),
        }
    }

    fn with_text(self, text: &str) -> Self {
        self.text_responses
            .borrow_mut()
            .push_back(Ok(text.to_string()));
        self
    }

    fn text_request_count(&self) -> usize {
        self.text_request_count.get()
    }

    fn byte_request_count(&self) -> usize {
        self.byte_request_count.get()
    }
}

impl ResourceHttpClient for ScriptedClient {
    fn get_text(&self, url: &str) -> resources::Result<String> {
        self.text_request_count
            .set(self.text_request_count.get() + 1);
        self.text_responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| {
                Err(ResourcesError::HttpRequestFailed {
                    url: url.to_string(),
                    reason: "no scripted text response".to_string(),
                })
            })
    }

    fn download(&self, url: &str, writer: &mut dyn Write) -> resources::Result<()> {
        let _writer = writer;
        self.byte_request_count
            .set(self.byte_request_count.get() + 1);
        Err(ResourcesError::HttpRequestFailed {
            url: url.to_string(),
            reason: "no scripted byte response".to_string(),
        })
    }
}

fn pv_paths(home: &Utf8Path) -> PvPaths {
    PvPaths::for_home(home.to_path_buf())
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI Postgres tests create fixture directories"
)]
fn create_dir(path: &Utf8Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(path)?;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI Postgres tests write fixture files"
)]
fn write_file(path: &Utf8Path, contents: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)?;

    Ok(())
}

fn assert_resource_snapshot(
    name: &'static str,
    tempdir: &Utf8Path,
    snapshot: &impl std::fmt::Debug,
) {
    let mut settings = Settings::clone_current();
    settings.add_filter(tempdir.as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!(name, snapshot);
    });
}
