use camino::{Utf8Path, Utf8PathBuf};
use state::{Database, ManagedResourceDesiredState, ManagedResourceTrackRecord, PvPaths};
use thiserror::Error;

use crate::http::ResourceHttpClient;
use crate::registry;
use crate::{
    ArtifactDownloader, ArtifactManifestCache, ArtifactManifestSource, ArtifactVersion,
    ResourceAdapter, ResourceName, ResourcesError, TargetPlatform, TrackName, TrackSelector,
};

pub type ManagedResourceCommandResult<T> = std::result::Result<T, ManagedResourceCommandError>;

#[derive(Debug, Error)]
pub enum ManagedResourceCommandError {
    #[error(transparent)]
    Resources(#[from] ResourcesError),

    #[error(transparent)]
    State(#[from] state::StateError),
}

#[derive(Clone, Debug)]
pub struct ManagedResourceCommands {
    paths: PvPaths,
    manifest_url: String,
    target_platform: TargetPlatform,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedResourceInstall {
    resource_name: ResourceName,
    track: TrackName,
    artifact_version: ArtifactVersion,
    current_artifact_path: Utf8PathBuf,
    manifest_source: ArtifactManifestSource,
    downloaded_from_cache: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedResourceUpdate {
    installs: Vec<ManagedResourceInstall>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedResourceUninstall {
    record: ManagedResourceTrackRecord,
}

impl ManagedResourceCommands {
    pub fn new(
        paths: PvPaths,
        manifest_url: impl Into<String>,
        target_platform: TargetPlatform,
    ) -> Self {
        Self {
            paths,
            manifest_url: manifest_url.into(),
            target_platform,
        }
    }

    pub fn install(
        &self,
        adapter: &impl ResourceAdapter,
        selector: TrackSelector,
        client: &impl ResourceHttpClient,
    ) -> ManagedResourceCommandResult<ManagedResourceInstall> {
        registry::resolve_canonical(adapter.resource_name().as_str())?;

        let refresh = ArtifactManifestCache::new(self.paths.downloads())
            .refresh(&self.manifest_url, client)?;
        let manifest_source = refresh.source().clone();
        let manifest = refresh.manifest();
        let track = manifest
            .resolve_track(adapter.resource_name(), selector)?
            .clone();
        let artifact = manifest
            .select_latest(adapter.resource_name(), &track, self.target_platform)?
            .artifact()
            .clone();

        let mut database = Database::open(&self.paths)?;
        database.record_managed_resource_track_desired(
            adapter.resource_name().as_str(),
            track.as_str(),
            ManagedResourceDesiredState::Installed,
        )?;

        let download =
            ArtifactDownloader::new(self.paths.downloads()).download(&artifact, client)?;
        let install = crate::ArtifactInstaller::new(self.paths.resources()).install(
            adapter,
            &track,
            &artifact,
            download.path(),
        )?;
        database.record_managed_resource_track_installed(
            adapter.resource_name().as_str(),
            track.as_str(),
            artifact.artifact_version().as_str(),
            install.release_path(),
        )?;

        Ok(ManagedResourceInstall {
            resource_name: adapter.resource_name().clone(),
            track,
            artifact_version: artifact.artifact_version().clone(),
            current_artifact_path: install.release_path().to_path_buf(),
            manifest_source,
            downloaded_from_cache: download.is_from_cache(),
        })
    }

    pub fn update(
        &self,
        adapter: &impl ResourceAdapter,
        client: &impl ResourceHttpClient,
    ) -> ManagedResourceCommandResult<ManagedResourceUpdate> {
        registry::resolve_canonical(adapter.resource_name().as_str())?;

        let installed_tracks = self
            .list(Some(adapter.resource_name()))?
            .into_iter()
            .filter(|record| {
                record.desired_state == ManagedResourceDesiredState::Installed
                    && record.installed_version.is_some()
            })
            .collect::<Vec<_>>();
        let mut installs = Vec::new();

        for record in installed_tracks {
            let track = TrackName::new(record.track)?;
            installs.push(self.install(adapter, TrackSelector::Track(track), client)?);
        }

        Ok(ManagedResourceUpdate { installs })
    }

    pub fn uninstall(
        &self,
        resource_name: &ResourceName,
        track: &TrackName,
    ) -> ManagedResourceCommandResult<ManagedResourceUninstall> {
        registry::resolve_canonical(resource_name.as_str())?;

        let mut database = Database::open(&self.paths)?;
        let record = database.record_managed_resource_track_desired(
            resource_name.as_str(),
            track.as_str(),
            ManagedResourceDesiredState::Removed,
        )?;

        Ok(ManagedResourceUninstall { record })
    }

    pub fn list(
        &self,
        resource_name: Option<&ResourceName>,
    ) -> ManagedResourceCommandResult<Vec<ManagedResourceTrackRecord>> {
        if let Some(resource_name) = resource_name {
            registry::resolve_canonical(resource_name.as_str())?;
        }

        let database = Database::open(&self.paths)?;
        let records = database.managed_resource_tracks()?;
        let filtered = records
            .into_iter()
            .filter(|record| match resource_name {
                Some(resource_name) => record.resource_name == resource_name.as_str(),
                None => true,
            })
            .collect();

        Ok(filtered)
    }
}

impl ManagedResourceInstall {
    pub fn resource_name(&self) -> &ResourceName {
        &self.resource_name
    }

    pub fn track(&self) -> &TrackName {
        &self.track
    }

    pub fn artifact_version(&self) -> &ArtifactVersion {
        &self.artifact_version
    }

    pub fn current_artifact_path(&self) -> &Utf8Path {
        &self.current_artifact_path
    }

    pub fn manifest_source(&self) -> &ArtifactManifestSource {
        &self.manifest_source
    }

    pub fn downloaded_from_cache(&self) -> bool {
        self.downloaded_from_cache
    }
}

impl ManagedResourceUpdate {
    pub fn installs(&self) -> &[ManagedResourceInstall] {
        &self.installs
    }
}

impl ManagedResourceUninstall {
    pub fn record(&self) -> &ManagedResourceTrackRecord {
        &self.record
    }
}
