use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::OsString;
use std::io;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use cli::{Environment, run_with_environment};
use resources::{ResourceHttpClient, ResourcesError, TargetPlatform};
use state::{
    Database, ManagedResourceTrackRecord, PortRequest, PvPaths, RuntimeObservedStatus,
    RuntimeSubject,
};

const MANIFEST_URL: &str = "https://artifacts.example.test/manifest.json";

#[derive(Clone, Copy)]
pub(crate) struct ResourceCliSpec {
    pub(crate) resource_name: &'static str,
    pub(crate) executable_path: &'static str,
    pub(crate) support_files: &'static [(&'static str, &'static str)],
}

#[derive(Debug)]
pub(crate) struct TestEnvironment {
    home: PathBuf,
    current_dir: PathBuf,
    client: ScriptedClient,
    pub(crate) target_platform_resolution_fails: bool,
}

impl TestEnvironment {
    pub(crate) fn new(home: &Utf8Path, current_dir: &Utf8Path, client: ScriptedClient) -> Self {
        Self {
            home: home.as_std_path().to_path_buf(),
            current_dir: current_dir.as_std_path().to_path_buf(),
            client,
            target_platform_resolution_fails: false,
        }
    }

    pub(crate) fn text_request_count(&self) -> usize {
        self.client.text_request_count()
    }

    pub(crate) fn byte_request_count(&self) -> usize {
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

    fn resolve_target_platform(&self) -> resources::Result<TargetPlatform> {
        if self.target_platform_resolution_fails {
            return Err(ResourcesError::UnsupportedPlatform {
                platform: "linux-aarch64".to_string(),
            });
        }

        Ok(TargetPlatform::DarwinArm64)
    }
}

#[derive(Debug)]
#[expect(
    dead_code,
    reason = "snapshot-only structure is read through derived Debug"
)]
pub(crate) struct RunOutput {
    pub(crate) exit_code: ExitCode,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

pub(crate) fn run_pv(args: &[&str], environment: &impl Environment) -> anyhow::Result<RunOutput> {
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

pub(crate) fn managed_resource_records(
    database: &Database,
    spec: ResourceCliSpec,
) -> anyhow::Result<Vec<ManagedResourceTrackRecord>> {
    Ok(database
        .managed_resource_tracks()?
        .into_iter()
        .filter(|record| record.resource_name == spec.resource_name)
        .collect())
}

#[derive(Debug)]
#[expect(
    dead_code,
    reason = "snapshot-only structure is read through derived Debug"
)]
pub(crate) struct ResourceRecordSnapshot {
    resource_name: String,
    track: String,
    desired_state: String,
    installed_version: Option<String>,
    current_artifact_path: Option<String>,
    usage_count: i64,
    removal_prune: bool,
    removal_force: bool,
}

pub(crate) fn resource_record_snapshots(
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
pub(crate) struct FixtureArtifact {
    version: String,
    platform: String,
}

pub(crate) fn fixture_artifact(version: &str) -> FixtureArtifact {
    FixtureArtifact {
        version: version.to_string(),
        platform: TargetPlatform::DarwinArm64.as_str().to_string(),
    }
}

pub(crate) fn prepare_existing_release(
    home: &Utf8Path,
    track: &str,
    artifact: &FixtureArtifact,
    spec: ResourceCliSpec,
) -> anyhow::Result<()> {
    let release = release_path(home, track, artifact, spec);
    let executable = release.join(spec.executable_path);
    let parent = executable
        .parent()
        .ok_or_else(|| anyhow::anyhow!("fixture executable has no parent: {executable}"))?;
    create_dir(parent)?;
    write_file(&executable, "fixture executable\n")?;
    for (relative_path, contents) in spec.support_files {
        write_file(&release.join(relative_path), contents)?;
    }

    Ok(())
}

pub(crate) fn record_installed_resource(
    home: &Utf8Path,
    track: &str,
    artifact: &FixtureArtifact,
    spec: ResourceCliSpec,
) -> anyhow::Result<()> {
    prepare_existing_release(home, track, artifact, spec)?;
    let release = release_path(home, track, artifact, spec);
    let mut database = Database::open(&pv_paths(home))?;
    database.record_managed_resource_track_installed(
        spec.resource_name,
        track,
        &artifact.version,
        &release,
    )?;

    Ok(())
}

fn release_path(
    home: &Utf8Path,
    track: &str,
    artifact: &FixtureArtifact,
    spec: ResourceCliSpec,
) -> Utf8PathBuf {
    pv_paths(home)
        .resources()
        .join(spec.resource_name)
        .join(track)
        .join("releases")
        .join(&artifact.version)
}

pub(crate) fn seed_running_resource(
    paths: &PvPaths,
    track: &str,
    port_name: &str,
    port: u16,
    spec: ResourceCliSpec,
) -> anyhow::Result<()> {
    let mut database = Database::open(paths)?;
    database.assign_port(
        PortRequest::resource_port(spec.resource_name, track, port_name, port, port, port),
        |_| true,
    )?;
    database.record_runtime_observed_snapshot(
        RuntimeSubject::Resource {
            name: spec.resource_name.to_string(),
            track: track.to_string(),
        },
        RuntimeObservedStatus::Running,
        Some("Managed Resource runtime is ready"),
    )?;

    Ok(())
}

pub(crate) fn resource_manifest(
    default_track: &str,
    artifacts: &[&FixtureArtifact],
    spec: ResourceCliSpec,
) -> String {
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
                spec.resource_name,
                artifact.version,
                artifact.platform,
            )
        })
        .collect::<Vec<_>>()
        .join(",");

    let resource_name = spec.resource_name;
    format!(
        r#"
{{
  "schema_version": 1,
  "minimum_pv_version": "0.1.0",
  "resources": [
    {{
      "name": "{resource_name}",
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
pub(crate) struct ScriptedClient {
    text_responses: RefCell<VecDeque<Result<String, ResourcesError>>>,
    text_request_count: Cell<usize>,
    byte_request_count: Cell<usize>,
}

impl ScriptedClient {
    pub(crate) fn new() -> Self {
        Self {
            text_responses: RefCell::new(VecDeque::new()),
            text_request_count: Cell::new(0),
            byte_request_count: Cell::new(0),
        }
    }

    pub(crate) fn with_text(self, text: &str) -> Self {
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

pub(crate) fn pv_paths(home: &Utf8Path) -> PvPaths {
    PvPaths::for_home(home.to_path_buf())
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI resource tests create fixture directories"
)]
pub(crate) fn create_dir(path: &Utf8Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(path)?;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI resource tests write fixture files"
)]
fn write_file(path: &Utf8Path, contents: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)?;

    Ok(())
}
