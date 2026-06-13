use std::collections::BTreeSet;

use camino::{Utf8Path, Utf8PathBuf};
use state::{
    Database, ManagedResourceDesiredState, ManagedResourceTrackInstallInput,
    ManagedResourceTrackRecord, ManagedResourceTrackRemovalInput, PvPaths, StateError,
};
use thiserror::Error;

use crate::http::ResourceHttpClient;
use crate::registry;
use crate::runtime::{composer_adapter, frankenphp_adapter, php_adapter};
use crate::{
    ArtifactDownloader, ArtifactInstall, ArtifactInstaller, ArtifactManifest,
    ArtifactManifestCache, ArtifactManifestSource, ArtifactVersion, ManifestArtifact,
    ResourceAdapter, ResourceName, ResourcesError, TargetPlatform, TrackName, TrackSelector,
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

    #[error(
        "Managed Resource operation failed with `{original_error}`, and rollback also failed: {rollback_error}"
    )]
    RollbackFailed {
        original_error: Box<ManagedResourceCommandError>,
        rollback_error: ResourcesError,
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
    artifact_install: ArtifactInstall,
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
pub struct ManagedResourceUpdateCheck {
    tracks: Vec<ManagedResourceUpdateCheckTrack>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedResourceUpdateCheckTrack {
    status: ManagedResourceUpdateStatus,
    resource_name: ResourceName,
    track: TrackName,
    current_artifact_version: ArtifactVersion,
    current_artifact_path: Utf8PathBuf,
    latest_artifact_version: Option<ArtifactVersion>,
    current_revocation: Option<ManagedResourceUpdateRevocation>,
    latest_revocation: Option<ManagedResourceUpdateRevocation>,
    blocked_by: Option<ManagedResourceUpdateBlocker>,
    reason: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedResourceUpdateStatus {
    Current,
    UpdateAvailable,
    Blocked,
    Revoked,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedResourceUpdateRevocation {
    artifact_version: ArtifactVersion,
    reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedResourceUpdateBlocker {
    minimum_pv_version: String,
    current_pv_version: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhpPairInstall {
    php: ManagedResourceInstall,
    frankenphp: ManagedResourceInstall,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComposerWithPhpPairInstall {
    php_pair: PhpPairInstall,
    composer: ManagedResourceInstall,
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
        adapter: &(impl ResourceAdapter + ?Sized),
        selector: TrackSelector,
        client: &(impl ResourceHttpClient + ?Sized),
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
        client: &(impl ResourceHttpClient + ?Sized),
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
        self.validate_install_selection(&php, &track, manifest)?;
        self.validate_install_selection(&frankenphp, &track, manifest)?;

        let install = self.prepare_php_pair_install(
            &php,
            &frankenphp,
            track,
            manifest,
            refresh.source(),
            client,
        )?;
        if let Err(error) = self.record_php_pair_install(&install) {
            return Err(self.rollback_php_pair_after_error(&install, error));
        }

        Ok(install)
    }

    fn validate_install_selection(
        &self,
        adapter: &(impl ResourceAdapter + ?Sized),
        track: &TrackName,
        manifest: &ArtifactManifest,
    ) -> ManagedResourceCommandResult<()> {
        manifest.select_latest(adapter.resource_name(), track, self.target_platform)?;

        Ok(())
    }

    fn install_track(
        &self,
        adapter: &(impl ResourceAdapter + ?Sized),
        track: TrackName,
        manifest: &ArtifactManifest,
        manifest_source: &ArtifactManifestSource,
        client: &(impl ResourceHttpClient + ?Sized),
    ) -> ManagedResourceCommandResult<ManagedResourceInstall> {
        let selection =
            manifest.select_latest(adapter.resource_name(), &track, self.target_platform)?;
        let revoked_latest = selection
            .revoked_latest()
            .map(revoked_fallback_from_artifact);
        let mut database = Database::open(&self.paths)?;
        database.record_managed_resource_track_desired(
            adapter.resource_name().as_str(),
            track.as_str(),
            ManagedResourceDesiredState::Installed,
        )?;

        let artifact = selection.artifact().clone();
        let install = self.install_selected_artifact(
            adapter,
            track,
            artifact,
            revoked_latest,
            manifest_source,
            client,
        )?;
        database.record_managed_resource_track_installed(
            adapter.resource_name().as_str(),
            install.track.as_str(),
            install.artifact_version.as_str(),
            &install.current_artifact_path,
        )?;

        Ok(install)
    }

    fn prepare_track_install(
        &self,
        adapter: &(impl ResourceAdapter + ?Sized),
        track: TrackName,
        manifest: &ArtifactManifest,
        manifest_source: &ArtifactManifestSource,
        client: &(impl ResourceHttpClient + ?Sized),
    ) -> ManagedResourceCommandResult<ManagedResourceInstall> {
        let selection =
            manifest.select_latest(adapter.resource_name(), &track, self.target_platform)?;
        let revoked_latest = selection
            .revoked_latest()
            .map(revoked_fallback_from_artifact);
        let artifact = selection.artifact().clone();

        self.install_selected_artifact(
            adapter,
            track,
            artifact,
            revoked_latest,
            manifest_source,
            client,
        )
    }

    fn install_selected_artifact(
        &self,
        adapter: &(impl ResourceAdapter + ?Sized),
        track: TrackName,
        artifact: ManifestArtifact,
        revoked_latest: Option<ManagedResourceRevokedLatest>,
        manifest_source: &ArtifactManifestSource,
        client: &(impl ResourceHttpClient + ?Sized),
    ) -> ManagedResourceCommandResult<ManagedResourceInstall> {
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

        let current_artifact_path = install.release_path().to_path_buf();

        Ok(ManagedResourceInstall {
            resource_name: adapter.resource_name().clone(),
            track,
            artifact_version: artifact.artifact_version().clone(),
            current_artifact_path,
            manifest_source: manifest_source.clone(),
            revoked_latest,
            downloaded_from_cache,
            artifact_install: install,
        })
    }

    fn prepare_php_pair_install(
        &self,
        php: &impl ResourceAdapter,
        frankenphp: &impl ResourceAdapter,
        track: TrackName,
        manifest: &ArtifactManifest,
        manifest_source: &ArtifactManifestSource,
        client: &(impl ResourceHttpClient + ?Sized),
    ) -> ManagedResourceCommandResult<PhpPairInstall> {
        let php =
            self.prepare_track_install(php, track.clone(), manifest, manifest_source, client)?;
        let frankenphp = match self.prepare_track_install(
            frankenphp,
            track,
            manifest,
            manifest_source,
            client,
        ) {
            Ok(install) => install,
            Err(error) => return Err(self.rollback_after_error(&[&php], error)),
        };

        Ok(PhpPairInstall { php, frankenphp })
    }

    fn rollback_php_pair_install(
        &self,
        install: &PhpPairInstall,
    ) -> ManagedResourceCommandResult<()> {
        self.rollback_prepared_installs(&[install.frankenphp(), install.php()])
    }

    fn rollback_prepared_installs(
        &self,
        installs: &[&ManagedResourceInstall],
    ) -> ManagedResourceCommandResult<()> {
        let installer = ArtifactInstaller::new(self.paths.resources());
        let mut first_error = None;

        for install in installs {
            if let Err(error) = installer.rollback(&install.artifact_install)
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }

        if let Some(error) = first_error {
            return Err(error.into());
        }

        Ok(())
    }

    fn rollback_after_error(
        &self,
        installs: &[&ManagedResourceInstall],
        original_error: ManagedResourceCommandError,
    ) -> ManagedResourceCommandError {
        match self.rollback_prepared_installs(installs) {
            Ok(()) => original_error,
            Err(ManagedResourceCommandError::Resources(rollback_error)) => {
                ManagedResourceCommandError::RollbackFailed {
                    original_error: Box::new(original_error),
                    rollback_error,
                }
            }
            Err(error) => error,
        }
    }

    fn rollback_php_pair_after_error(
        &self,
        install: &PhpPairInstall,
        original_error: ManagedResourceCommandError,
    ) -> ManagedResourceCommandError {
        match self.rollback_php_pair_install(install) {
            Ok(()) => original_error,
            Err(ManagedResourceCommandError::Resources(rollback_error)) => {
                ManagedResourceCommandError::RollbackFailed {
                    original_error: Box::new(original_error),
                    rollback_error,
                }
            }
            Err(error) => error,
        }
    }

    fn record_php_pair_install(
        &self,
        install: &PhpPairInstall,
    ) -> ManagedResourceCommandResult<()> {
        let mut database = Database::open(&self.paths)?;
        database.record_managed_resource_tracks_desired_and_installed(&[
            ManagedResourceTrackInstallInput {
                resource_name: install.php.resource_name.as_str(),
                track: install.php.track.as_str(),
                installed_version: install.php.artifact_version.as_str(),
                current_artifact_path: &install.php.current_artifact_path,
            },
            ManagedResourceTrackInstallInput {
                resource_name: install.frankenphp.resource_name.as_str(),
                track: install.frankenphp.track.as_str(),
                installed_version: install.frankenphp.artifact_version.as_str(),
                current_artifact_path: &install.frankenphp.current_artifact_path,
            },
        ])?;

        Ok(())
    }

    fn record_composer_with_php_pair_install(
        &self,
        php_pair: &PhpPairInstall,
        composer: &ManagedResourceInstall,
    ) -> ManagedResourceCommandResult<()> {
        let mut database = Database::open(&self.paths)?;
        database.record_managed_resource_tracks_desired_and_installed(&[
            ManagedResourceTrackInstallInput {
                resource_name: php_pair.php.resource_name.as_str(),
                track: php_pair.php.track.as_str(),
                installed_version: php_pair.php.artifact_version.as_str(),
                current_artifact_path: &php_pair.php.current_artifact_path,
            },
            ManagedResourceTrackInstallInput {
                resource_name: php_pair.frankenphp.resource_name.as_str(),
                track: php_pair.frankenphp.track.as_str(),
                installed_version: php_pair.frankenphp.artifact_version.as_str(),
                current_artifact_path: &php_pair.frankenphp.current_artifact_path,
            },
            ManagedResourceTrackInstallInput {
                resource_name: composer.resource_name.as_str(),
                track: composer.track.as_str(),
                installed_version: composer.artifact_version.as_str(),
                current_artifact_path: &composer.current_artifact_path,
            },
        ])?;

        Ok(())
    }

    pub fn update(
        &self,
        adapter: &(impl ResourceAdapter + ?Sized),
        client: &(impl ResourceHttpClient + ?Sized),
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
        client: &(impl ResourceHttpClient + ?Sized),
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

        for track in &tracks {
            self.validate_install_selection(&php, track, refresh.manifest())?;
            self.validate_install_selection(&frankenphp, track, refresh.manifest())?;
        }

        for track in tracks {
            let install = self.prepare_php_pair_install(
                &php,
                &frankenphp,
                track,
                refresh.manifest(),
                refresh.source(),
                client,
            )?;
            if let Err(error) = self.record_php_pair_install(&install) {
                return Err(self.rollback_php_pair_after_error(&install, error));
            }
            installs.push(install.php);
            installs.push(install.frankenphp);
        }

        Ok(PhpPairUpdate { installs })
    }

    pub fn install_composer(
        &self,
        client: &(impl ResourceHttpClient + ?Sized),
    ) -> ManagedResourceCommandResult<ManagedResourceInstall> {
        self.install(
            &composer_adapter()?,
            TrackSelector::Track(composer_track()?),
            client,
        )
    }

    pub fn install_composer_with_php_pair(
        &self,
        php_selector: TrackSelector,
        client: &(impl ResourceHttpClient + ?Sized),
    ) -> ManagedResourceCommandResult<ComposerWithPhpPairInstall> {
        let php = php_adapter()?;
        let frankenphp = frankenphp_adapter()?;
        let composer = composer_adapter()?;
        registry::resolve_canonical(php.resource_name().as_str())?;
        registry::resolve_canonical(frankenphp.resource_name().as_str())?;
        registry::resolve_canonical(composer.resource_name().as_str())?;

        let refresh = ArtifactManifestCache::new(self.paths.downloads())
            .refresh(&self.manifest_url, client)?;
        let manifest = refresh.manifest();
        let php_track = manifest
            .resolve_track(php.resource_name(), php_selector)?
            .clone();
        let composer_track = composer_track()?;
        self.validate_install_selection(&php, &php_track, manifest)?;
        self.validate_install_selection(&frankenphp, &php_track, manifest)?;
        self.validate_install_selection(&composer, &composer_track, manifest)?;

        let php_pair = self.prepare_php_pair_install(
            &php,
            &frankenphp,
            php_track,
            manifest,
            refresh.source(),
            client,
        )?;
        let composer = match self.prepare_track_install(
            &composer,
            composer_track,
            manifest,
            refresh.source(),
            client,
        ) {
            Ok(install) => install,
            Err(error) => return Err(self.rollback_php_pair_after_error(&php_pair, error)),
        };
        if let Err(error) = self.record_composer_with_php_pair_install(&php_pair, &composer) {
            let error = self.rollback_after_error(&[&composer], error);

            return Err(self.rollback_php_pair_after_error(&php_pair, error));
        }

        Ok(ComposerWithPhpPairInstall { php_pair, composer })
    }

    pub fn update_composer(
        &self,
        client: &(impl ResourceHttpClient + ?Sized),
    ) -> ManagedResourceCommandResult<ManagedResourceUpdate> {
        let composer = composer_adapter()?;
        let track = composer_track()?;
        let installed_tracks = self.list(Some(composer.resource_name()))?;
        let mut installs = Vec::new();
        if !installed_tracks
            .iter()
            .any(|record| record.track() == &track)
        {
            return Ok(ManagedResourceUpdate { installs });
        }

        let refresh = ArtifactManifestCache::new(self.paths.downloads())
            .refresh_latest(&self.manifest_url, client)?;
        installs.push(self.install_track(
            &composer,
            track,
            refresh.manifest(),
            refresh.source(),
            client,
        )?);

        Ok(ManagedResourceUpdate { installs })
    }

    pub fn update_all_installed(
        &self,
        backing_adapters: &[&dyn ResourceAdapter],
        client: &(impl ResourceHttpClient + ?Sized),
    ) -> ManagedResourceCommandResult<ManagedResourceUpdate> {
        for adapter in backing_adapters {
            registry::resolve_canonical(adapter.resource_name().as_str())?;
        }

        let refresh = ArtifactManifestCache::new(self.paths.downloads())
            .refresh_latest(&self.manifest_url, client)?;
        let manifest = refresh.manifest();
        let installed_tracks = self.list(None)?;
        let mut installs = Vec::new();

        self.update_installed_php_pairs(
            &installed_tracks,
            manifest,
            refresh.source(),
            client,
            &mut installs,
        )?;
        self.update_installed_composer(
            &installed_tracks,
            manifest,
            refresh.source(),
            client,
            &mut installs,
        )?;
        self.update_installed_backing_resources(
            &installed_tracks,
            backing_adapters,
            manifest,
            refresh.source(),
            client,
            &mut installs,
        )?;

        Ok(ManagedResourceUpdate { installs })
    }

    fn update_installed_php_pairs(
        &self,
        installed_tracks: &[ManagedResourceTrack],
        manifest: &ArtifactManifest,
        manifest_source: &ArtifactManifestSource,
        client: &(impl ResourceHttpClient + ?Sized),
        installs: &mut Vec<ManagedResourceInstall>,
    ) -> ManagedResourceCommandResult<()> {
        let php = php_adapter()?;
        let frankenphp = frankenphp_adapter()?;
        let mut tracks = BTreeSet::new();

        collect_installed_tracks(installed_tracks, php.resource_name(), &mut tracks);
        collect_installed_tracks(installed_tracks, frankenphp.resource_name(), &mut tracks);

        for track in &tracks {
            self.validate_install_selection(&php, track, manifest)?;
            self.validate_install_selection(&frankenphp, track, manifest)?;
        }

        for track in tracks {
            let php_installed = find_installed_track(installed_tracks, php.resource_name(), &track);
            let frankenphp_installed =
                find_installed_track(installed_tracks, frankenphp.resource_name(), &track);
            if !self.track_needs_update(&php, &track, php_installed, manifest)?
                && !self.track_needs_update(&frankenphp, &track, frankenphp_installed, manifest)?
            {
                continue;
            }

            let install = self.prepare_php_pair_install(
                &php,
                &frankenphp,
                track,
                manifest,
                manifest_source,
                client,
            )?;
            if let Err(error) = self.record_php_pair_install(&install) {
                return Err(self.rollback_php_pair_after_error(&install, error));
            }
            installs.push(install.php);
            installs.push(install.frankenphp);
        }

        Ok(())
    }

    fn update_installed_composer(
        &self,
        installed_tracks: &[ManagedResourceTrack],
        manifest: &ArtifactManifest,
        manifest_source: &ArtifactManifestSource,
        client: &(impl ResourceHttpClient + ?Sized),
        installs: &mut Vec<ManagedResourceInstall>,
    ) -> ManagedResourceCommandResult<()> {
        let composer = composer_adapter()?;
        let track = composer_track()?;
        let Some(installed) =
            find_installed_track(installed_tracks, composer.resource_name(), &track)
        else {
            return Ok(());
        };
        if !self.track_needs_update(&composer, &track, Some(installed), manifest)? {
            return Ok(());
        }

        installs.push(self.install_track(&composer, track, manifest, manifest_source, client)?);

        Ok(())
    }

    fn update_installed_backing_resources(
        &self,
        installed_tracks: &[ManagedResourceTrack],
        backing_adapters: &[&dyn ResourceAdapter],
        manifest: &ArtifactManifest,
        manifest_source: &ArtifactManifestSource,
        client: &(impl ResourceHttpClient + ?Sized),
        installs: &mut Vec<ManagedResourceInstall>,
    ) -> ManagedResourceCommandResult<()> {
        for adapter in backing_adapters {
            for installed in installed_tracks
                .iter()
                .filter(|track| track.resource_name() == adapter.resource_name())
            {
                if !self.track_needs_update(
                    *adapter,
                    installed.track(),
                    Some(installed),
                    manifest,
                )? {
                    continue;
                }
                installs.push(self.install_track(
                    *adapter,
                    installed.track().clone(),
                    manifest,
                    manifest_source,
                    client,
                )?);
            }
        }

        Ok(())
    }

    fn track_needs_update(
        &self,
        adapter: &(impl ResourceAdapter + ?Sized),
        track: &TrackName,
        installed: Option<&ManagedResourceTrack>,
        manifest: &ArtifactManifest,
    ) -> ManagedResourceCommandResult<bool> {
        let selection =
            manifest.select_latest(adapter.resource_name(), track, self.target_platform)?;
        let latest_artifact = selection.artifact();
        let Some(installed) = installed else {
            return Ok(true);
        };

        if latest_artifact.artifact_version() != installed.installed_version() {
            return Ok(true);
        }

        let current_artifact = manifest.select_artifact(
            adapter.resource_name(),
            track,
            installed.installed_version(),
            self.target_platform,
        )?;

        Ok(current_artifact.is_some_and(|artifact| artifact.revocation_state().is_revoked()))
    }

    pub fn check_updates(
        &self,
        client: &(impl ResourceHttpClient + ?Sized),
    ) -> ManagedResourceCommandResult<ManagedResourceUpdateCheck> {
        let installed_tracks = self.list(None)?;
        let refresh = ArtifactManifestCache::new(self.paths.downloads())
            .refresh_latest(&self.manifest_url, client);
        let refresh = match refresh {
            Ok(refresh) => refresh,
            Err(ResourcesError::RequiresNewerPv {
                minimum_pv_version,
                current_pv_version,
            }) => {
                return Ok(ManagedResourceUpdateCheck {
                    tracks: installed_tracks
                        .into_iter()
                        .map(|track| {
                            ManagedResourceUpdateCheckTrack::blocked(
                                track,
                                ManagedResourceUpdateBlocker {
                                    minimum_pv_version: minimum_pv_version.clone(),
                                    current_pv_version: current_pv_version.clone(),
                                },
                            )
                        })
                        .collect(),
                });
            }
            Err(error) => return Err(error.into()),
        };
        let tracks = installed_tracks
            .into_iter()
            .map(|track| {
                check_installed_track_update(track, refresh.manifest(), self.target_platform)
            })
            .collect();

        Ok(ManagedResourceUpdateCheck { tracks })
    }

    pub fn uninstall(
        &self,
        resource_name: &ResourceName,
        track: &TrackName,
        options: ManagedResourceUninstallOptions,
    ) -> ManagedResourceCommandResult<ManagedResourceRemovalIntent> {
        validate_uninstall_request(resource_name, track)?;
        let mut database = Database::open(&self.paths)?;
        let records = database.managed_resource_tracks()?;
        validate_uninstall_eligibility(&records, resource_name, track, options)?;

        record_removal_intent(&mut database, resource_name, track, options)
    }

    pub fn uninstall_php_pair(
        &self,
        track: &TrackName,
        options: ManagedResourceUninstallOptions,
    ) -> ManagedResourceCommandResult<PhpPairRemovalIntent> {
        let php = php_adapter()?;
        let frankenphp = frankenphp_adapter()?;
        validate_uninstall_request(php.resource_name(), track)?;
        validate_uninstall_request(frankenphp.resource_name(), track)?;

        let mut database = Database::open(&self.paths)?;
        let records = database.managed_resource_tracks()?;
        validate_uninstall_eligibility(&records, php.resource_name(), track, options)?;
        validate_uninstall_eligibility(&records, frankenphp.resource_name(), track, options)?;

        database.record_managed_resource_tracks_removal_intent(&[
            ManagedResourceTrackRemovalInput {
                resource_name: php.resource_name().as_str(),
                track: track.as_str(),
                prune: options.prune,
                force: options.force,
            },
            ManagedResourceTrackRemovalInput {
                resource_name: frankenphp.resource_name().as_str(),
                track: track.as_str(),
                prune: options.prune,
                force: options.force,
            },
        ])?;
        let php = ManagedResourceRemovalIntent {
            resource_name: php.resource_name().clone(),
            track: track.clone(),
            prune: options.prune,
            force: options.force,
        };
        let frankenphp = ManagedResourceRemovalIntent {
            resource_name: frankenphp.resource_name().clone(),
            track: track.clone(),
            prune: options.prune,
            force: options.force,
        };

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

impl ManagedResourceUpdateCheck {
    pub fn tracks(&self) -> &[ManagedResourceUpdateCheckTrack] {
        &self.tracks
    }
}

impl ManagedResourceUpdateCheckTrack {
    fn blocked(track: ManagedResourceTrack, blocked_by: ManagedResourceUpdateBlocker) -> Self {
        Self {
            status: ManagedResourceUpdateStatus::Blocked,
            resource_name: track.resource_name,
            track: track.track,
            current_artifact_version: track.installed_version,
            current_artifact_path: track.current_artifact_path,
            latest_artifact_version: None,
            current_revocation: None,
            latest_revocation: None,
            blocked_by: Some(blocked_by),
            reason: None,
        }
    }

    fn unavailable(track: ManagedResourceTrack, reason: String) -> Self {
        Self {
            status: ManagedResourceUpdateStatus::Unavailable,
            resource_name: track.resource_name,
            track: track.track,
            current_artifact_version: track.installed_version,
            current_artifact_path: track.current_artifact_path,
            latest_artifact_version: None,
            current_revocation: None,
            latest_revocation: None,
            blocked_by: None,
            reason: Some(reason),
        }
    }

    pub fn status(&self) -> ManagedResourceUpdateStatus {
        self.status
    }

    pub fn resource_name(&self) -> &ResourceName {
        &self.resource_name
    }

    pub fn track(&self) -> &TrackName {
        &self.track
    }

    pub fn current_artifact_version(&self) -> &ArtifactVersion {
        &self.current_artifact_version
    }

    pub fn current_artifact_path(&self) -> &Utf8Path {
        &self.current_artifact_path
    }

    pub fn latest_artifact_version(&self) -> Option<&ArtifactVersion> {
        self.latest_artifact_version.as_ref()
    }

    pub fn current_revocation(&self) -> Option<&ManagedResourceUpdateRevocation> {
        self.current_revocation.as_ref()
    }

    pub fn latest_revocation(&self) -> Option<&ManagedResourceUpdateRevocation> {
        self.latest_revocation.as_ref()
    }

    pub fn blocked_by(&self) -> Option<&ManagedResourceUpdateBlocker> {
        self.blocked_by.as_ref()
    }

    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }
}

impl ManagedResourceUpdateRevocation {
    pub fn artifact_version(&self) -> &ArtifactVersion {
        &self.artifact_version
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl ManagedResourceUpdateBlocker {
    pub fn minimum_pv_version(&self) -> &str {
        &self.minimum_pv_version
    }

    pub fn current_pv_version(&self) -> &str {
        &self.current_pv_version
    }
}

impl std::fmt::Display for ManagedResourceUpdateBlocker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "requires PV {}, current PV {}",
            self.minimum_pv_version, self.current_pv_version
        )
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

impl ComposerWithPhpPairInstall {
    pub fn php_pair(&self) -> &PhpPairInstall {
        &self.php_pair
    }

    pub fn composer(&self) -> &ManagedResourceInstall {
        &self.composer
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

fn validate_uninstall_request(
    resource_name: &ResourceName,
    track: &TrackName,
) -> ManagedResourceCommandResult<()> {
    registry::resolve_canonical(resource_name.as_str())?;
    if TrackSelector::is_reserved_alias(track.as_str()) {
        return Err(ResourcesError::ReservedTrackName {
            name: track.as_str().to_string(),
        }
        .into());
    }

    Ok(())
}

fn collect_installed_tracks(
    installed_tracks: &[ManagedResourceTrack],
    resource_name: &ResourceName,
    tracks: &mut BTreeSet<TrackName>,
) {
    for installed in installed_tracks
        .iter()
        .filter(|track| track.resource_name() == resource_name)
    {
        tracks.insert(installed.track().clone());
    }
}

fn find_installed_track<'a>(
    installed_tracks: &'a [ManagedResourceTrack],
    resource_name: &ResourceName,
    track: &TrackName,
) -> Option<&'a ManagedResourceTrack> {
    installed_tracks
        .iter()
        .find(|installed| installed.resource_name() == resource_name && installed.track() == track)
}

fn validate_uninstall_eligibility(
    records: &[ManagedResourceTrackRecord],
    resource_name: &ResourceName,
    track: &TrackName,
    options: ManagedResourceUninstallOptions,
) -> ManagedResourceCommandResult<()> {
    let Some(installed_track) = records
        .iter()
        .find(|record| {
            record.resource_name == resource_name.as_str() && record.track == track.as_str()
        })
        .filter(|record| {
            record.desired_state == ManagedResourceDesiredState::Installed
                && record.installed_version.is_some()
                && record.current_artifact_path.is_some()
        })
    else {
        return Err(ManagedResourceCommandError::TrackNotInstalled {
            resource: resource_name.as_str().to_string(),
            track: track.as_str().to_string(),
        });
    };

    if installed_track.usage_count > 0 && !options.force {
        return Err(ManagedResourceCommandError::TrackInUse {
            resource: resource_name.as_str().to_string(),
            track: track.as_str().to_string(),
            usage_count: installed_track.usage_count,
        });
    }

    Ok(())
}

fn record_removal_intent(
    database: &mut Database,
    resource_name: &ResourceName,
    track: &TrackName,
    options: ManagedResourceUninstallOptions,
) -> ManagedResourceCommandResult<ManagedResourceRemovalIntent> {
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

fn check_installed_track_update(
    track: ManagedResourceTrack,
    manifest: &ArtifactManifest,
    target_platform: TargetPlatform,
) -> ManagedResourceUpdateCheckTrack {
    let current_artifact = manifest.select_artifact(
        track.resource_name(),
        track.track(),
        track.installed_version(),
        target_platform,
    );
    let current_revocation = match current_artifact {
        Ok(Some(artifact)) => update_revocation_from_current_artifact(artifact),
        Ok(None) => None,
        Err(error) => {
            return ManagedResourceUpdateCheckTrack::unavailable(
                track,
                format!("artifact lookup failed: {error}"),
            );
        }
    };

    let selection =
        match manifest.select_latest(track.resource_name(), track.track(), target_platform) {
            Ok(selection) => selection,
            Err(error) => {
                if current_revocation.is_some() {
                    return ManagedResourceUpdateCheckTrack {
                        status: ManagedResourceUpdateStatus::Revoked,
                        resource_name: track.resource_name,
                        track: track.track,
                        current_artifact_version: track.installed_version,
                        current_artifact_path: track.current_artifact_path,
                        latest_artifact_version: None,
                        current_revocation,
                        latest_revocation: None,
                        blocked_by: None,
                        reason: None,
                    };
                }
                return ManagedResourceUpdateCheckTrack::unavailable(track, error.to_string());
            }
        };
    let latest_artifact = selection.artifact();
    let latest_revocation = selection
        .revoked_latest()
        .map(update_revocation_from_artifact);
    let status = if current_revocation.is_some() {
        ManagedResourceUpdateStatus::Revoked
    } else if latest_artifact.artifact_version() != track.installed_version() {
        ManagedResourceUpdateStatus::UpdateAvailable
    } else {
        ManagedResourceUpdateStatus::Current
    };

    ManagedResourceUpdateCheckTrack {
        status,
        resource_name: track.resource_name,
        track: track.track,
        current_artifact_version: track.installed_version,
        current_artifact_path: track.current_artifact_path,
        latest_artifact_version: Some(latest_artifact.artifact_version().clone()),
        current_revocation,
        latest_revocation,
        blocked_by: None,
        reason: None,
    }
}

fn update_revocation_from_current_artifact(
    artifact: &ManifestArtifact,
) -> Option<ManagedResourceUpdateRevocation> {
    if !artifact.revocation_state().is_revoked() {
        return None;
    }

    Some(ManagedResourceUpdateRevocation {
        artifact_version: artifact.artifact_version().clone(),
        reason: artifact
            .revocation_state()
            .reason()
            .unwrap_or_default()
            .to_string(),
    })
}

fn update_revocation_from_artifact(artifact: &ManifestArtifact) -> ManagedResourceUpdateRevocation {
    ManagedResourceUpdateRevocation {
        artifact_version: artifact.artifact_version().clone(),
        reason: artifact
            .revocation_state()
            .reason()
            .unwrap_or_default()
            .to_string(),
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
