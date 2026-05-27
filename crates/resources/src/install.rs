use std::path::{Component, Path};
use std::sync::atomic::{AtomicU64, Ordering};

use camino::{Utf8Path, Utf8PathBuf};
use flate2::read::GzDecoder;
use tar::Archive;

use crate::fs;
use crate::{ArtifactVersion, ManifestArtifact, ResourceName, ResourcesError, Result, TrackName};

static INSTALL_COUNTER: AtomicU64 = AtomicU64::new(0);

pub trait ResourceAdapter {
    fn resource_name(&self) -> &ResourceName;
    fn validate_installation(&self, root: &Utf8Path) -> Result<()>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactInstaller {
    resources_dir: Utf8PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactInstall {
    resource_name: ResourceName,
    track: TrackName,
    artifact_version: ArtifactVersion,
    release_path: Utf8PathBuf,
    current_path: Utf8PathBuf,
}

impl ArtifactInstaller {
    pub fn new(resources_dir: impl Into<Utf8PathBuf>) -> Self {
        Self {
            resources_dir: resources_dir.into(),
        }
    }

    pub fn install(
        &self,
        adapter: &impl ResourceAdapter,
        track: &TrackName,
        artifact: &ManifestArtifact,
        archive_path: &Utf8Path,
    ) -> Result<ArtifactInstall> {
        let track_dir = self
            .resources_dir
            .join(adapter.resource_name().as_str())
            .join(track.as_str());
        let releases_dir = track_dir.join("releases");
        let release_path = releases_dir.join(artifact.artifact_version().as_str());
        let current_path = track_dir.join("current");
        let previous_release = current_release_name(&current_path)?;

        if fs::path_exists(&release_path) {
            adapter.validate_installation(&release_path)?;
            update_current_pointer(&track_dir, artifact.artifact_version())?;
            prune_old_releases(
                &releases_dir,
                artifact.artifact_version(),
                previous_release.as_deref(),
            )?;

            return Ok(ArtifactInstall::new(
                adapter.resource_name().clone(),
                track.clone(),
                artifact.artifact_version().clone(),
                release_path,
                current_path,
            ));
        }

        fs::create_dir_all(&releases_dir)?;
        let staging_dir = staging_dir(&track_dir, artifact.artifact_version());

        let result = unpack_validate_and_promote(
            archive_path,
            &staging_dir,
            &release_path,
            adapter,
            artifact.artifact_version(),
        );
        if let Err(error) = result {
            if let Err(_cleanup_error) = fs::remove_dir_all_if_exists(&staging_dir) {}

            return Err(error);
        }

        update_current_pointer(&track_dir, artifact.artifact_version())?;
        prune_old_releases(
            &releases_dir,
            artifact.artifact_version(),
            previous_release.as_deref(),
        )?;
        fs::remove_dir_all_if_exists(&staging_dir)?;

        Ok(ArtifactInstall::new(
            adapter.resource_name().clone(),
            track.clone(),
            artifact.artifact_version().clone(),
            release_path,
            current_path,
        ))
    }
}

impl ArtifactInstall {
    fn new(
        resource_name: ResourceName,
        track: TrackName,
        artifact_version: ArtifactVersion,
        release_path: Utf8PathBuf,
        current_path: Utf8PathBuf,
    ) -> Self {
        Self {
            resource_name,
            track,
            artifact_version,
            release_path,
            current_path,
        }
    }

    pub fn resource_name(&self) -> &ResourceName {
        &self.resource_name
    }

    pub fn track(&self) -> &TrackName {
        &self.track
    }

    pub fn artifact_version(&self) -> &ArtifactVersion {
        &self.artifact_version
    }

    pub fn release_path(&self) -> &Utf8Path {
        &self.release_path
    }

    pub fn current_path(&self) -> &Utf8Path {
        &self.current_path
    }
}

fn unpack_validate_and_promote(
    archive_path: &Utf8Path,
    staging_dir: &Utf8Path,
    release_path: &Utf8Path,
    adapter: &impl ResourceAdapter,
    artifact_version: &ArtifactVersion,
) -> Result<()> {
    fs::remove_dir_all_if_exists(staging_dir)?;
    fs::create_dir_all(staging_dir)?;
    let root = unpack_single_root_archive(archive_path, staging_dir)?;
    adapter.validate_installation(&root)?;
    fs::rename(&root, release_path)?;

    if let Some(parent) = release_path.parent() {
        fs::sync_directory(parent)?;
    }

    if !fs::path_exists(release_path) {
        return Err(ResourcesError::InvalidArtifactLayout {
            resource: adapter.resource_name().as_str().to_string(),
            reason: format!("artifact `{artifact_version}` was not installed"),
        });
    }

    Ok(())
}

fn unpack_single_root_archive(
    archive_path: &Utf8Path,
    staging_dir: &Utf8Path,
) -> Result<Utf8PathBuf> {
    let file = open_archive_file(archive_path)?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);
    let mut root_name: Option<String> = None;
    let entries = archive
        .entries()
        .map_err(|source| invalid_archive(archive_path, source))?;

    for entry in entries {
        let mut entry = entry.map_err(|source| invalid_archive(archive_path, source))?;
        let entry_path = entry
            .path()
            .map_err(|source| invalid_archive(archive_path, source))?;
        let relative_path = clean_archive_path(archive_path, &entry_path)?;
        let Some(first_component) = relative_path.components().next() else {
            continue;
        };

        match root_name.as_deref() {
            Some(root) if root != first_component.as_str() => {
                return Err(ResourcesError::InvalidArtifactArchive {
                    path: archive_path.to_string(),
                    reason: "archive must contain exactly one top-level directory".to_string(),
                });
            }
            None => root_name = Some(first_component.to_string()),
            _ => {}
        }

        let output_path = staging_dir.join(&relative_path);
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }

        entry
            .unpack(&output_path)
            .map_err(|source| invalid_archive(archive_path, source))?;
    }

    let Some(root_name) = root_name else {
        return Err(ResourcesError::InvalidArtifactArchive {
            path: archive_path.to_string(),
            reason: "archive is empty".to_string(),
        });
    };

    Ok(staging_dir.join(root_name))
}

fn clean_archive_path(archive_path: &Utf8Path, path: &Path) -> Result<Utf8PathBuf> {
    let mut clean = Utf8PathBuf::new();

    for component in path.components() {
        match component {
            Component::Normal(value) => {
                let Some(value) = value.to_str() else {
                    return Err(ResourcesError::InvalidArtifactArchive {
                        path: archive_path.to_string(),
                        reason: "archive entry path is not valid UTF-8".to_string(),
                    });
                };
                clean.push(value);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ResourcesError::InvalidArtifactArchive {
                    path: archive_path.to_string(),
                    reason: format!("archive entry `{}` is not relative", path.display()),
                });
            }
        }
    }

    if clean.as_str().is_empty() {
        return Err(ResourcesError::InvalidArtifactArchive {
            path: archive_path.to_string(),
            reason: "archive entry path is empty".to_string(),
        });
    }

    Ok(clean)
}

fn staging_dir(track_dir: &Utf8Path, artifact_version: &ArtifactVersion) -> Utf8PathBuf {
    let process_id = std::process::id();
    let counter = INSTALL_COUNTER.fetch_add(1, Ordering::Relaxed);

    track_dir.join(format!(
        ".installing-{}-{process_id}-{counter}",
        artifact_version.as_str()
    ))
}

fn update_current_pointer(track_dir: &Utf8Path, artifact_version: &ArtifactVersion) -> Result<()> {
    let current_path = track_dir.join("current");
    let temporary_path = track_dir.join(format!(
        "current.{}.{}.tmp",
        std::process::id(),
        INSTALL_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));

    fs::remove_file_if_exists(&temporary_path)?;
    symlink_dir(
        Utf8Path::new(&format!("releases/{}", artifact_version.as_str())),
        &temporary_path,
    )?;
    fs::rename(&temporary_path, &current_path)?;
    fs::sync_directory(track_dir)?;

    Ok(())
}

fn current_release_name(current_path: &Utf8Path) -> Result<Option<String>> {
    if !fs::path_exists(current_path) {
        return Ok(None);
    }

    let target = read_link(current_path)?;
    let Some(version) = target.strip_prefix("releases/") else {
        return Ok(None);
    };
    if version.contains('/') || version.is_empty() {
        return Ok(None);
    }

    Ok(Some(version.to_string()))
}

fn prune_old_releases(
    releases_dir: &Utf8Path,
    current: &ArtifactVersion,
    previous: Option<&str>,
) -> Result<()> {
    for release in release_entries(releases_dir)? {
        if release.name == current.as_str() || Some(release.name.as_str()) == previous {
            continue;
        }

        fs::remove_dir_all_if_exists(&release.path)?;
    }

    fs::sync_directory(releases_dir)?;

    Ok(())
}

struct ReleaseEntry {
    name: String,
    path: Utf8PathBuf,
}

#[expect(
    clippy::disallowed_methods,
    reason = "resource installer owns release directory pruning"
)]
fn release_entries(path: &Utf8Path) -> Result<Vec<ReleaseEntry>> {
    let mut entries = Vec::new();

    for entry in std::fs::read_dir(path).map_err(|source| ResourcesError::Filesystem {
        path: path.to_string(),
        reason: source.to_string(),
    })? {
        let entry = entry.map_err(|source| ResourcesError::Filesystem {
            path: path.to_string(),
            reason: source.to_string(),
        })?;
        if !entry
            .file_type()
            .map_err(|source| ResourcesError::Filesystem {
                path: path.to_string(),
                reason: source.to_string(),
            })?
            .is_dir()
        {
            continue;
        }

        let name = entry.file_name().to_string_lossy().into_owned();
        let path = Utf8PathBuf::from_path_buf(entry.path()).map_err(|path| {
            ResourcesError::Filesystem {
                path: path.to_string_lossy().into_owned(),
                reason: "release path is not valid UTF-8".to_string(),
            }
        })?;
        entries.push(ReleaseEntry { name, path });
    }

    Ok(entries)
}

#[expect(
    clippy::disallowed_types,
    reason = "resource installer owns archive file handles"
)]
fn open_archive_file(path: &Utf8Path) -> Result<std::fs::File> {
    std::fs::File::open(path).map_err(|source| ResourcesError::Filesystem {
        path: path.to_string(),
        reason: source.to_string(),
    })
}

fn invalid_archive(path: &Utf8Path, source: std::io::Error) -> ResourcesError {
    ResourcesError::InvalidArtifactArchive {
        path: path.to_string(),
        reason: source.to_string(),
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "resource installer owns current symlink inspection"
)]
fn read_link(path: &Utf8Path) -> Result<String> {
    std::fs::read_link(path)
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(|source| ResourcesError::Filesystem {
            path: path.to_string(),
            reason: source.to_string(),
        })
}

#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "resource installer owns current symlink updates"
)]
fn symlink_dir(target: &Utf8Path, link: &Utf8Path) -> Result<()> {
    std::os::unix::fs::symlink(target, link).map_err(|source| ResourcesError::Filesystem {
        path: link.to_string(),
        reason: source.to_string(),
    })
}

#[cfg(not(unix))]
fn symlink_dir(_target: &Utf8Path, link: &Utf8Path) -> Result<()> {
    Err(ResourcesError::Filesystem {
        path: link.to_string(),
        reason: "PV artifact installs require Unix symlinks".to_string(),
    })
}
