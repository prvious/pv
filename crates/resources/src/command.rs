use std::collections::BTreeSet;

use camino::{Utf8Path, Utf8PathBuf};
use state::{
    Database, ManagedResourceDesiredState, ManagedResourceTrackRecord, PvPaths, StateError,
};
use thiserror::Error;

use crate::http::ResourceHttpClient;
use crate::registry;
use crate::runtime::{composer_adapter, frankenphp_adapter, php_adapter};
use crate::{
    ArtifactDownloader, ArtifactInstaller, ArtifactManifest, ArtifactManifestCache,
    ArtifactManifestSource, ArtifactVersion, ManifestArtifact, ResourceAdapter, ResourceName,
    ResourcesError, TargetPlatform, TrackName, TrackSelector,
};

pub type ManagedResourceCommandResult<T> = std::result::Result<T, ManagedResourceCommandError>;

#[derive(Debug, Error)]
pub enum ManagedResourceCommandError {
    #[error(transparent)]
    Resources(#[from] ResourcesError),

    #[error(transparent)]
    State(#[from] StateError),

    #[error("Managed Resource `{resource}` track `{track}` is not installed")]
    TrackNotInstalled { resource: String, track: String },

    #[error(
        "Managed Resource `{resource}` track `{track}` is used by {usage_count} linked project(s); use --force to remove it anyway"
    )]
    TrackInUse {
        resource: String,
        track: String,
        usage_count: i64,
    },
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
    revoked_latest: Option<ManagedResourceRevokedLatest>,
    downloaded_from_cache: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedResourceRevokedLatest {
    artifact_version: ArtifactVersion,
    reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedResourceUpdate {
    installs: Vec<ManagedResourceInstall>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhpPairInstall {
    php: ManagedResourceInstall,
    frankenphp: ManagedResourceInstall,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhpPairUpdate {
    installs: Vec<ManagedResourceInstall>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhpPairRemovalIntent {
    php: ManagedResourceRemovalIntent,
    frankenphp: ManagedResourceRemovalIntent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedResourceRemovalIntent {
    resource_name: ResourceName,
    track: TrackName,
    prune: bool,
    force: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ManagedResourceUninstallOptions {
    prune: bool,
    force: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedResourceTrack {
    resource_name: ResourceName,
    track: TrackName,
    installed_version: ArtifactVersion,
    current_artifact_path: Utf8PathBuf,
    usage_count: i64,
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
        let manifest = refresh.manifest();
        let track = manifest
            .resolve_track(adapter.resource_name(), selector)?
            .clone();

        self.install_track(adapter, track, manifest, refresh.source(), client)
    }

    pub fn install_php_pair(
        &self,
        selector: TrackSelector,
        client: &impl ResourceHttpClient,
    ) -> ManagedResourceCommandResult<PhpPairInstall> {
        let php = php_adapter()?;
        let frankenphp = frankenphp_adapter()?;
        registry::resolve_canonical(php.resource_name().as_str())?;
        registry::resolve_canonical(frankenphp.resource_name().as_str())?;

        let refresh = ArtifactManifestCache::new(self.paths.downloads())
            .refresh(&self.manifest_url, client)?;
        let manifest = refresh.manifest();
        let track = manifest
            .resolve_track(php.resource_name(), selector)?
            .clone();

        let php = self.install_track(&php, track.clone(), manifest, refresh.source(), client)?;
        let frankenphp =
            self.install_track(&frankenphp, track, manifest, refresh.source(), client)?;

        Ok(PhpPairInstall { php, frankenphp })
    }

    fn install_track(
        &self,
        adapter: &impl ResourceAdapter,
        track: TrackName,
        manifest: &ArtifactManifest,
        manifest_source: &ArtifactManifestSource,
        client: &impl ResourceHttpClient,
    ) -> ManagedResourceCommandResult<ManagedResourceInstall> {
        let selection =
            manifest.select_latest(adapter.resource_name(), &track, self.target_platform)?;
        let revoked_latest = selection
            .revoked_latest()
            .map(revoked_fallback_from_artifact);
        let artifact = selection.artifact().clone();
        let mut database = Database::open(&self.paths)?;
        database.record_managed_resource_track_desired(
            adapter.resource_name().as_str(),
            track.as_str(),
            ManagedResourceDesiredState::Installed,
        )?;

        let installer = ArtifactInstaller::new(self.paths.resources());
        let (install, downloaded_from_cache) = if let Some(existing_install) =
            installer.install_existing_release(adapter, &track, &artifact)?
        {
            (existing_install, false)
        } else {
            let download =
                ArtifactDownloader::new(self.paths.downloads()).download(&artifact, client)?;
            (
                installer.install(adapter, &track, &artifact, download.path())?,
                download.is_from_cache(),
            )
        };
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
            manifest_source: manifest_source.clone(),
            revoked_latest,
            downloaded_from_cache,
        })
    }

    pub fn update(
        &self,
        adapter: &impl ResourceAdapter,
        client: &impl ResourceHttpClient,
    ) -> ManagedResourceCommandResult<ManagedResourceUpdate> {
        registry::resolve_canonical(adapter.resource_name().as_str())?;

        let installed_tracks = self.list(Some(adapter.resource_name()))?;
        let mut installs = Vec::new();
        if installed_tracks.is_empty() {
            return Ok(ManagedResourceUpdate { installs });
        }

        let refresh = ArtifactManifestCache::new(self.paths.downloads())
            .refresh_latest(&self.manifest_url, client)?;

        for record in installed_tracks {
            installs.push(self.install_track(
                adapter,
                record.track,
                refresh.manifest(),
                refresh.source(),
                client,
            )?);
        }

        Ok(ManagedResourceUpdate { installs })
    }

    pub fn update_php_pairs(
        &self,
        client: &impl ResourceHttpClient,
    ) -> ManagedResourceCommandResult<PhpPairUpdate> {
        let php = php_adapter()?;
        let frankenphp = frankenphp_adapter()?;
        let mut tracks = BTreeSet::new();

        for record in self.list(Some(php.resource_name()))? {
            tracks.insert(record.track().clone());
        }
        for record in self.list(Some(frankenphp.resource_name()))? {
            tracks.insert(record.track().clone());
        }

        let mut installs = Vec::new();
        if tracks.is_empty() {
            return Ok(PhpPairUpdate { installs });
        }

        let refresh = ArtifactManifestCache::new(self.paths.downloads())
            .refresh_latest(&self.manifest_url, client)?;

        for track in tracks {
            installs.push(self.install_track(
                &php,
                track.clone(),
                refresh.manifest(),
                refresh.source(),
                client,
            )?);
            installs.push(self.install_track(
                &frankenphp,
                track,
                refresh.manifest(),
                refresh.source(),
                client,
            )?);
        }

        Ok(PhpPairUpdate { installs })
    }

    pub fn install_composer(
        &self,
        client: &impl ResourceHttpClient,
    ) -> ManagedResourceCommandResult<ManagedResourceInstall> {
        self.install(
            &composer_adapter()?,
            TrackSelector::Track(composer_track()?),
            client,
        )
    }

    pub fn update_composer(
        &self,
        client: &impl ResourceHttpClient,
    ) -> ManagedResourceCommandResult<ManagedResourceUpdate> {
        self.update(&composer_adapter()?, client)
    }

    pub fn uninstall(
        &self,
        resource_name: &ResourceName,
        track: &TrackName,
        options: ManagedResourceUninstallOptions,
    ) -> ManagedResourceCommandResult<ManagedResourceRemovalIntent> {
        registry::resolve_canonical(resource_name.as_str())?;
        if TrackSelector::is_reserved_alias(track.as_str()) {
            return Err(ResourcesError::ReservedTrackName {
                name: track.as_str().to_string(),
            }
            .into());
        }

        let mut database = Database::open(&self.paths)?;
        let installed_track = database
            .managed_resource_tracks()?
            .into_iter()
            .find(|record| {
                record.resource_name == resource_name.as_str() && record.track == track.as_str()
            })
            .filter(|record| {
                record.desired_state == ManagedResourceDesiredState::Installed
                    && record.installed_version.is_some()
                    && record.current_artifact_path.is_some()
            })
            .ok_or_else(|| ManagedResourceCommandError::TrackNotInstalled {
                resource: resource_name.as_str().to_string(),
                track: track.as_str().to_string(),
            })?;
        if installed_track.usage_count > 0 && !options.force {
            return Err(ManagedResourceCommandError::TrackInUse {
                resource: resource_name.as_str().to_string(),
                track: track.as_str().to_string(),
                usage_count: installed_track.usage_count,
            });
        }

        // Uninstall records intent. Daemon reconciliation owns runtime stops,
        // artifact removal, mutable data pruning, and installed metadata cleanup.
        database.record_managed_resource_track_removal_intent(
            resource_name.as_str(),
            track.as_str(),
            options.prune,
            options.force,
        )?;

        Ok(ManagedResourceRemovalIntent {
            resource_name: resource_name.clone(),
            track: track.clone(),
            prune: options.prune,
            force: options.force,
        })
    }

    pub fn uninstall_php_pair(
        &self,
        track: &TrackName,
        options: ManagedResourceUninstallOptions,
    ) -> ManagedResourceCommandResult<PhpPairRemovalIntent> {
        let php = php_adapter()?;
        let frankenphp = frankenphp_adapter()?;
        let php = self.uninstall(php.resource_name(), track, options)?;
        let frankenphp = self.uninstall(frankenphp.resource_name(), track, options)?;

        Ok(PhpPairRemovalIntent { php, frankenphp })
    }

    pub fn uninstall_composer(
        &self,
        options: ManagedResourceUninstallOptions,
    ) -> ManagedResourceCommandResult<ManagedResourceRemovalIntent> {
        let composer = composer_adapter()?;
        let track = composer_track()?;

        self.uninstall(composer.resource_name(), &track, options)
    }

    pub fn list(
        &self,
        resource_name: Option<&ResourceName>,
    ) -> ManagedResourceCommandResult<Vec<ManagedResourceTrack>> {
        if let Some(resource_name) = resource_name {
            registry::resolve_canonical(resource_name.as_str())?;
        }

        let database = Database::open(&self.paths)?;
        let records = database.managed_resource_tracks()?;
        let mut filtered = Vec::new();

        for record in records {
            if let Some(resource_name) = resource_name
                && record.resource_name != resource_name.as_str()
            {
                continue;
            }
            let Some(track) = ManagedResourceTrack::from_state_record(record)? else {
                continue;
            };
            filtered.push(track);
        }

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

    pub fn revoked_latest(&self) -> Option<&ManagedResourceRevokedLatest> {
        self.revoked_latest.as_ref()
    }
}

impl ManagedResourceRevokedLatest {
    pub fn artifact_version(&self) -> &ArtifactVersion {
        &self.artifact_version
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl ManagedResourceUpdate {
    pub fn installs(&self) -> &[ManagedResourceInstall] {
        &self.installs
    }
}

impl PhpPairInstall {
    pub fn php(&self) -> &ManagedResourceInstall {
        &self.php
    }

    pub fn frankenphp(&self) -> &ManagedResourceInstall {
        &self.frankenphp
    }
}

impl PhpPairUpdate {
    pub fn installs(&self) -> &[ManagedResourceInstall] {
        &self.installs
    }
}

impl PhpPairRemovalIntent {
    pub fn php(&self) -> &ManagedResourceRemovalIntent {
        &self.php
    }

    pub fn frankenphp(&self) -> &ManagedResourceRemovalIntent {
        &self.frankenphp
    }
}

impl ManagedResourceRemovalIntent {
    pub fn resource_name(&self) -> &ResourceName {
        &self.resource_name
    }

    pub fn track(&self) -> &TrackName {
        &self.track
    }

    pub fn prune(&self) -> bool {
        self.prune
    }

    pub fn force(&self) -> bool {
        self.force
    }
}

impl ManagedResourceUninstallOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn prune(mut self, prune: bool) -> Self {
        self.prune = prune;
        self
    }

    pub fn force(mut self, force: bool) -> Self {
        self.force = force;
        self
    }

    pub fn should_prune(self) -> bool {
        self.prune
    }

    pub fn should_force(self) -> bool {
        self.force
    }
}

impl ManagedResourceTrack {
    fn from_state_record(
        record: ManagedResourceTrackRecord,
    ) -> ManagedResourceCommandResult<Option<Self>> {
        if record.desired_state != ManagedResourceDesiredState::Installed {
            return Ok(None);
        }
        let (Some(installed_version), Some(current_artifact_path)) =
            (record.installed_version, record.current_artifact_path)
        else {
            return Ok(None);
        };

        Ok(Some(Self {
            resource_name: ResourceName::new(record.resource_name)?,
            track: TrackName::new(record.track)?,
            installed_version: ArtifactVersion::new(installed_version)?,
            current_artifact_path,
            usage_count: record.usage_count,
        }))
    }

    pub fn resource_name(&self) -> &ResourceName {
        &self.resource_name
    }

    pub fn track(&self) -> &TrackName {
        &self.track
    }

    pub fn installed_version(&self) -> &ArtifactVersion {
        &self.installed_version
    }

    pub fn current_artifact_path(&self) -> &Utf8Path {
        &self.current_artifact_path
    }

    pub fn usage_count(&self) -> i64 {
        self.usage_count
    }
}

fn revoked_fallback_from_artifact(artifact: &ManifestArtifact) -> ManagedResourceRevokedLatest {
    ManagedResourceRevokedLatest {
        artifact_version: artifact.artifact_version().clone(),
        reason: artifact
            .revocation_state()
            .reason()
            .unwrap_or_default()
            .to_string(),
    }
}

fn composer_track() -> ManagedResourceCommandResult<TrackName> {
    Ok(TrackName::new("2")?)
}
