use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use flate2::Compression;
use flate2::write::GzEncoder;
use insta::assert_debug_snapshot;
use resources::{
    ArtifactInstall, ArtifactInstaller, ArtifactManifest, ManifestArtifact, ResourceAdapter,
    ResourceName, ResourcesError, TargetPlatform, TrackName,
};
use tar::{Builder, Header};

#[test]
fn artifact_installer_unpacks_single_root_archive_and_updates_current_pointer() -> Result<()> {
    let tempdir = tempdir()?;
    let archive_path = tempdir.path().join("redis.tar.gz");
    write_fixture_archive(
        &archive_path,
        &[(
            "redis-7.2.5-pv1/bin/redis-server",
            b"redis executable" as &[u8],
        )],
    )?;
    let installer = ArtifactInstaller::new(tempdir.path().join("resources"));
    let adapter = RequiredPathAdapter::new("redis", &["bin/redis-server"])?;
    let artifact = redis_artifact()?;
    let track = TrackName::new("7.2")?;

    let install = installer.install(&adapter, &track, &artifact, &archive_path)?;

    assert_debug_snapshot!(install_summary(&install, tempdir.path())?);

    Ok(())
}

#[test]
fn artifact_installer_keeps_current_release_when_new_archive_fails_validation() -> Result<()> {
    let tempdir = tempdir()?;
    let first_archive_path = tempdir.path().join("redis-7.2.5.tar.gz");
    let broken_archive_path = tempdir.path().join("redis-7.2.6.tar.gz");
    write_fixture_archive(
        &first_archive_path,
        &[(
            "redis-7.2.5-pv1/bin/redis-server",
            b"redis executable" as &[u8],
        )],
    )?;
    write_fixture_archive(
        &broken_archive_path,
        &[(
            "redis-7.2.6-pv1/README.md",
            b"missing redis-server" as &[u8],
        )],
    )?;
    let installer = ArtifactInstaller::new(tempdir.path().join("resources"));
    let adapter = RequiredPathAdapter::new("redis", &["bin/redis-server"])?;
    let track = TrackName::new("7.2")?;
    let first_artifact = redis_artifact()?;
    let broken_artifact = redis_artifact_with_version("7.2.6-pv1")?;

    let first_install =
        installer.install(&adapter, &track, &first_artifact, &first_archive_path)?;
    let failed_install =
        installer.install(&adapter, &track, &broken_artifact, &broken_archive_path);

    assert_debug_snapshot!((
        failed_install,
        read_link(first_install.current_path())?,
        first_install.release_path().exists(),
        first_install
            .release_path()
            .parent()
            .map(sorted_file_names)
            .transpose()?,
    ));

    Ok(())
}

#[test]
fn artifact_installer_retains_current_and_previous_releases() -> Result<()> {
    let tempdir = tempdir()?;
    let installer = ArtifactInstaller::new(tempdir.path().join("resources"));
    let adapter = RequiredPathAdapter::new("redis", &["bin/redis-server"])?;
    let track = TrackName::new("7.2")?;

    for version in ["7.2.5-pv1", "7.2.6-pv1", "7.2.7-pv1"] {
        let archive_path = tempdir.path().join(format!("{version}.tar.gz"));
        write_fixture_archive(
            &archive_path,
            &[(
                &format!("{version}/bin/redis-server"),
                b"redis executable" as &[u8],
            )],
        )?;
        let artifact = redis_artifact_with_version(version)?;
        installer.install(&adapter, &track, &artifact, &archive_path)?;
    }

    let releases_dir = tempdir.path().join("resources/redis/7.2/releases");
    let current_path = tempdir.path().join("resources/redis/7.2/current");

    assert_eq!(
        sorted_file_names(&releases_dir)?,
        vec!["7.2.6-pv1", "7.2.7-pv1"]
    );
    assert_eq!(read_link(&current_path)?, "releases/7.2.7-pv1");

    Ok(())
}

struct RequiredPathAdapter {
    resource_name: ResourceName,
    required_paths: Vec<Utf8PathBuf>,
}

impl RequiredPathAdapter {
    fn new(resource_name: &str, required_paths: &[&str]) -> Result<Self> {
        Ok(Self {
            resource_name: ResourceName::new(resource_name)?,
            required_paths: required_paths.iter().map(Utf8PathBuf::from).collect(),
        })
    }
}

impl ResourceAdapter for RequiredPathAdapter {
    fn resource_name(&self) -> &ResourceName {
        &self.resource_name
    }

    fn validate_installation(&self, root: &Utf8Path) -> resources::Result<()> {
        for required_path in &self.required_paths {
            let candidate = root.join(required_path);
            if !candidate.exists() {
                return Err(ResourcesError::InvalidArtifactLayout {
                    resource: self.resource_name.as_str().to_string(),
                    reason: format!("missing required path `{required_path}`"),
                });
            }
        }

        Ok(())
    }
}

fn redis_artifact() -> Result<ManifestArtifact> {
    redis_artifact_with_version("7.2.5-pv1")
}

fn redis_artifact_with_version(version: &str) -> Result<ManifestArtifact> {
    let manifest = VALID_MANIFEST.replace("7.2.5-pv1", version);
    let parsed = ArtifactManifest::parse(&manifest)?;
    let resource = ResourceName::new("redis")?;
    let track = TrackName::new("7.2")?;
    let selected = parsed.select_latest(&resource, &track, TargetPlatform::new("darwin-arm64")?)?;

    Ok(selected.artifact().clone())
}

fn install_summary(install: &ArtifactInstall, root: &Utf8Path) -> Result<(String, String, String)> {
    Ok((
        relative_path(root, install.release_path())?,
        relative_path(root, install.current_path())?,
        read_link(install.current_path())?,
    ))
}

fn relative_path(root: &Utf8Path, path: &Utf8Path) -> Result<String> {
    Ok(path.strip_prefix(root)?.to_string())
}

fn sorted_file_names(path: &Utf8Path) -> Result<Vec<String>> {
    let mut file_names = path
        .read_dir_utf8()?
        .map(|entry| {
            entry
                .map(|entry| entry.file_name().to_string())
                .map_err(anyhow::Error::from)
        })
        .collect::<Result<Vec<_>>>()?;
    file_names.sort();

    Ok(file_names)
}

#[expect(
    clippy::disallowed_types,
    reason = "resource install tests create fixture archives directly"
)]
fn write_fixture_archive(path: &Utf8Path, entries: &[(&str, &[u8])]) -> Result<()> {
    let file = std::fs::File::create(path)?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut builder = Builder::new(encoder);

    for (path, content) in entries {
        let mut header = Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder.append_data(&mut header, path, *content)?;
    }

    let encoder = builder.into_inner()?;
    encoder.finish()?;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "resource install tests inspect current symlink targets directly"
)]
fn read_link(path: &Utf8Path) -> Result<String> {
    Ok(std::fs::read_link(path)?.to_string_lossy().into_owned())
}

const VALID_MANIFEST: &str = r#"
{
  "schema_version": 1,
  "minimum_pv_version": "0.1.0",
  "resources": [
    {
      "name": "redis",
      "default_track": "7.2",
      "tracks": [
        {
          "name": "7.2",
          "artifacts": [
            {
              "artifact_version": "7.2.5-pv1",
              "upstream_version": "7.2.5",
              "pv_build_revision": "1",
              "platform": "darwin-arm64",
              "url": "https://artifacts.example.test/redis-7.2.5-pv1-darwin-arm64.tar.gz",
              "sha256": "87698b18df0047a6404165a79250f5728ecc25b65fed27077ed9dff23e1232a9",
              "size": 22,
              "published_at": "2026-05-26T14:30:00Z"
            }
          ]
        }
      ]
    }
  ]
}
"#;
