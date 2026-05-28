use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::Write;

use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use flate2::Compression;
use flate2::write::GzEncoder;
use insta::assert_debug_snapshot;
use resources::{
    ManagedResourceCommands, ResourceAdapter, ResourceHttpClient, ResourceName, ResourcesError,
    TargetPlatform, TrackSelector,
};
use sha2::{Digest, Sha256};
use state::PvPaths;
use tar::{Builder, Header};

#[test]
fn managed_resource_commands_install_update_list_and_uninstall_fake_adapter() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let commands =
        ManagedResourceCommands::new(paths.clone(), MANIFEST_URL, TargetPlatform::DarwinArm64);
    let adapter = FakeAdapter::new("redis", &["bin/pv-fake-resource"])?;
    let first_artifact = fixture_artifact("7.2.5-pv1", "first")?;
    let second_artifact = fixture_artifact("7.2.6-pv1", "second")?;
    let first_manifest = manifest_with_artifacts(&[&first_artifact]);
    let second_manifest = manifest_with_artifacts(&[&first_artifact, &second_artifact]);
    let client = ScriptedClient::new()
        .with_text(&first_manifest)
        .with_bytes(first_artifact.bytes())
        .with_text(&second_manifest)
        .with_bytes(second_artifact.bytes());

    let installed = commands.install(&adapter, TrackSelector::Latest, &client)?;
    let updated = commands.update(&adapter, &client)?;
    let listed_after_update = commands.list(Some(adapter.resource_name()))?;
    let uninstalled = commands.uninstall(adapter.resource_name(), updated.installs()[0].track())?;
    let listed_after_uninstall = commands.list(Some(adapter.resource_name()))?;

    assert_debug_snapshot!((
        installed.summary(tempdir.path()),
        updated.summary(tempdir.path()),
        track_records_summary(&listed_after_update, tempdir.path())?,
        track_record_summary(uninstalled.record(), tempdir.path())?,
        track_records_summary(&listed_after_uninstall, tempdir.path())?,
    ));

    Ok(())
}

#[test]
fn managed_resource_commands_keep_installed_state_when_update_validation_fails() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let commands =
        ManagedResourceCommands::new(paths.clone(), MANIFEST_URL, TargetPlatform::DarwinArm64);
    let adapter = FakeAdapter::new("redis", &["bin/pv-fake-resource"])?;
    let first_artifact = fixture_artifact("7.2.5-pv1", "first")?;
    let broken_artifact = fixture_artifact_with_entries("7.2.6-pv1", &[("README.md", "broken")])?;
    let first_manifest = manifest_with_artifacts(&[&first_artifact]);
    let broken_manifest = manifest_with_artifacts(&[&first_artifact, &broken_artifact]);
    let client = ScriptedClient::new()
        .with_text(&first_manifest)
        .with_bytes(first_artifact.bytes())
        .with_text(&broken_manifest)
        .with_bytes(broken_artifact.bytes());

    let installed = commands.install(&adapter, TrackSelector::Latest, &client)?;
    let failed_update = commands.update(&adapter, &client);
    let listed_after_failure = commands.list(Some(adapter.resource_name()))?;

    assert_debug_snapshot!((
        installed.summary(tempdir.path()),
        failed_update,
        track_records_summary(&listed_after_failure, tempdir.path())?,
    ));

    Ok(())
}

struct FakeAdapter {
    resource_name: ResourceName,
    required_paths: Vec<Utf8PathBuf>,
}

impl FakeAdapter {
    fn new(resource_name: &str, required_paths: &[&str]) -> Result<Self> {
        Ok(Self {
            resource_name: ResourceName::new(resource_name)?,
            required_paths: required_paths.iter().map(Utf8PathBuf::from).collect(),
        })
    }
}

impl ResourceAdapter for FakeAdapter {
    fn resource_name(&self) -> &ResourceName {
        &self.resource_name
    }

    fn validate_installation(&self, root: &Utf8Path) -> resources::Result<()> {
        for required_path in &self.required_paths {
            if !root.join(required_path).exists() {
                return Err(ResourcesError::InvalidArtifactLayout {
                    resource: self.resource_name.as_str().to_string(),
                    reason: format!("missing required path `{required_path}`"),
                });
            }
        }

        Ok(())
    }
}

#[derive(Clone, Debug)]
struct FixtureArtifact {
    version: String,
    bytes: Vec<u8>,
    sha256: String,
}

impl FixtureArtifact {
    fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Debug)]
#[expect(
    dead_code,
    reason = "snapshot-only structure is read through derived Debug"
)]
struct InstallSnapshot {
    resource_name: String,
    track: String,
    artifact_version: String,
    current_artifact_path: String,
    downloaded_from_cache: bool,
}

#[derive(Debug)]
#[expect(
    dead_code,
    reason = "snapshot-only structure is read through derived Debug"
)]
struct TrackRecordSnapshot {
    resource_name: String,
    track: String,
    desired_state: String,
    installed_version: Option<String>,
    current_artifact_path: Option<String>,
}

trait InstallSummary {
    fn summary(&self, root: &Utf8Path) -> Result<Vec<InstallSnapshot>>;
}

impl InstallSummary for resources::ManagedResourceInstall {
    fn summary(&self, root: &Utf8Path) -> Result<Vec<InstallSnapshot>> {
        Ok(vec![install_summary(self, root)?])
    }
}

impl InstallSummary for resources::ManagedResourceUpdate {
    fn summary(&self, root: &Utf8Path) -> Result<Vec<InstallSnapshot>> {
        self.installs()
            .iter()
            .map(|install| install_summary(install, root))
            .collect()
    }
}

fn install_summary(
    install: &resources::ManagedResourceInstall,
    root: &Utf8Path,
) -> Result<InstallSnapshot> {
    Ok(InstallSnapshot {
        resource_name: install.resource_name().as_str().to_string(),
        track: install.track().as_str().to_string(),
        artifact_version: install.artifact_version().as_str().to_string(),
        current_artifact_path: install
            .current_artifact_path()
            .strip_prefix(root)?
            .to_string(),
        downloaded_from_cache: install.downloaded_from_cache(),
    })
}

fn track_records_summary(
    records: &[state::ManagedResourceTrackRecord],
    root: &Utf8Path,
) -> Result<Vec<TrackRecordSnapshot>> {
    records
        .iter()
        .map(|record| track_record_summary(record, root))
        .collect()
}

fn track_record_summary(
    record: &state::ManagedResourceTrackRecord,
    root: &Utf8Path,
) -> Result<TrackRecordSnapshot> {
    Ok(TrackRecordSnapshot {
        resource_name: record.resource_name.clone(),
        track: record.track.clone(),
        desired_state: format!("{:?}", record.desired_state),
        installed_version: record.installed_version.clone(),
        current_artifact_path: record
            .current_artifact_path
            .as_ref()
            .map(|path| path.strip_prefix(root).map(Utf8Path::to_string))
            .transpose()?,
    })
}

#[derive(Debug)]
struct ScriptedClient {
    text_responses: RefCell<VecDeque<Result<String, ResourcesError>>>,
    byte_responses: RefCell<VecDeque<Result<Vec<u8>, ResourcesError>>>,
}

impl ScriptedClient {
    fn new() -> Self {
        Self {
            text_responses: RefCell::new(VecDeque::new()),
            byte_responses: RefCell::new(VecDeque::new()),
        }
    }

    fn with_text(self, text: &str) -> Self {
        self.text_responses
            .borrow_mut()
            .push_back(Ok(text.to_string()));
        self
    }

    fn with_bytes(self, bytes: &[u8]) -> Self {
        self.byte_responses
            .borrow_mut()
            .push_back(Ok(bytes.to_vec()));
        self
    }
}

impl ResourceHttpClient for ScriptedClient {
    fn get_text(&self, url: &str) -> resources::Result<String> {
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
        let bytes = self
            .byte_responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| {
                Err(ResourcesError::HttpRequestFailed {
                    url: url.to_string(),
                    reason: "no scripted byte response".to_string(),
                })
            })?;
        writer
            .write_all(&bytes)
            .map_err(|source| ResourcesError::HttpRequestFailed {
                url: url.to_string(),
                reason: source.to_string(),
            })
    }
}

fn fixture_artifact(version: &str, marker: &str) -> Result<FixtureArtifact> {
    fixture_artifact_with_entries(
        version,
        &[("bin/pv-fake-resource", &format!("fake resource {marker}"))],
    )
}

fn fixture_artifact_with_entries(
    version: &str,
    entries: &[(&str, &str)],
) -> Result<FixtureArtifact> {
    let root = format!("redis-{version}-darwin-arm64");
    let bytes = fixture_archive_bytes(&root, entries)?;
    let sha256 = sha256_hex(&bytes);

    Ok(FixtureArtifact {
        version: version.to_string(),
        bytes,
        sha256,
    })
}

fn fixture_archive_bytes(root: &str, entries: &[(&str, &str)]) -> Result<Vec<u8>> {
    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut builder = Builder::new(encoder);

    for (path, content) in entries {
        let path = format!("{root}/{path}");
        let mut header = Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder.append_data(&mut header, path, content.as_bytes())?;
    }

    let encoder = builder.into_inner()?;
    Ok(encoder.finish()?)
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);

    for byte in digest {
        hex.push(HEX[(byte >> 4) as usize] as char);
        hex.push(HEX[(byte & 0x0f) as usize] as char);
    }

    hex
}

fn manifest_with_artifacts(artifacts: &[&FixtureArtifact]) -> String {
    let artifacts = artifacts
        .iter()
        .map(|artifact| {
            format!(
                r#"{{
              "artifact_version": "{}",
              "upstream_version": "{}",
              "pv_build_revision": "1",
              "platform": "darwin-arm64",
              "url": "https://artifacts.example.test/redis-{}-darwin-arm64.tar.gz",
              "sha256": "{}",
              "size": {},
              "published_at": "{}"
            }}"#,
                artifact.version,
                artifact.version.trim_end_matches("-pv1"),
                artifact.version,
                artifact.sha256,
                artifact.bytes.len(),
                published_at_for(&artifact.version),
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
      "name": "redis",
      "default_track": "7.2",
      "tracks": [
        {{
          "name": "7.2",
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

fn published_at_for(version: &str) -> &'static str {
    match version {
        "7.2.5-pv1" => "2026-05-26T14:30:00Z",
        "7.2.6-pv1" => "2026-05-27T14:30:00Z",
        _ => "2026-05-28T14:30:00Z",
    }
}

const MANIFEST_URL: &str = "https://artifacts.example.test/manifest.json";
