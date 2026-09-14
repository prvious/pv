use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use camino::{Utf8Path, Utf8PathBuf};
use state::{
    Database, ManagedResourceDesiredState, ManagedResourceTrackInstallInput,
    ManagedResourceTrackRecord, ManagedResourceTrackRemovalInput, PvPaths, StateError,
};
use thiserror::Error;

use crate::http::ResourceHttpClient;
use crate::install::validate_artifact_matches_request;
use crate::registry;
use crate::runtime::{composer_adapter, frankenphp_adapter, php_adapter};
use crate::{
    ArtifactDownload, ArtifactDownloader, ArtifactInstall, ArtifactInstaller, ArtifactManifest,
    ArtifactManifestCache, ArtifactManifestRefresh, ArtifactManifestSource, ArtifactVersion,
    DownloadProgress, ManifestArtifact, NoDownloadProgress, ResourceAdapter, ResourceName,
    ResourceOperation, ResourceOperationEvent, ResourceOperationOutcome, ResourcesError,
    TargetPlatform, TrackName, TrackSelector,
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

    #[error("{source}")]
    PartialUpdate {
        #[source]
        source: Box<ManagedResourceCommandError>,
        update: ManagedResourceUpdate,
    },

    #[error("Managed Resource updates failed: {}", format_update_failures(.failures))]
    UpdateFailures {
        failures: Vec<ManagedResourceUpdateFailure>,
    },
}

#[derive(Clone, Debug)]
pub struct ManagedResourceCommands {
    paths: PvPaths,
    manifest_url: String,
    target_platform: TargetPlatform,
}

struct ArtifactInstallContext<'context, Client, Progress>
where
    Client: ResourceHttpClient + ?Sized,
    Progress: DownloadProgress,
{
    manifest_source: &'context ArtifactManifestSource,
    client: &'context Client,
    progress: &'context Progress,
}

impl<Client, Progress> Clone for ArtifactInstallContext<'_, Client, Progress>
where
    Client: ResourceHttpClient + ?Sized,
    Progress: DownloadProgress,
{
    fn clone(&self) -> Self {
        *self
    }
}

impl<Client, Progress> Copy for ArtifactInstallContext<'_, Client, Progress>
where
    Client: ResourceHttpClient + ?Sized,
    Progress: DownloadProgress,
{
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

#[derive(Clone, Debug)]
pub struct ManagedResourceInstallArtifact {
    artifact: ManifestArtifact,
    revoked_latest: Option<ManagedResourceRevokedLatest>,
    download_required: bool,
    manifest_source: ArtifactManifestSource,
    target_platform: TargetPlatform,
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

#[derive(Clone)]
enum PrefetchedUpdate<'adapter> {
    PhpPair {
        php: ManagedResourceInstallArtifact,
        frankenphp: Box<ManagedResourceInstallArtifact>,
    },
    Artifact {
        adapter: &'adapter dyn ResourceAdapter,
        resolved: ManagedResourceInstallArtifact,
    },
}

impl PrefetchedUpdate<'_> {
    fn label(&self) -> String {
        match self {
            Self::PhpPair { php, .. } => format!("php/frankenphp {}", php.artifact().track()),
            Self::Artifact { resolved, .. } => format!(
                "{} {}",
                resolved.artifact().resource_name(),
                resolved.artifact().track()
            ),
        }
    }
}

#[derive(Debug)]
pub struct ManagedResourceUpdateFailure {
    label: String,
    error: Box<ManagedResourceCommandError>,
}

impl ManagedResourceUpdateFailure {
    fn new(label: String, error: ManagedResourceCommandError) -> Self {
        Self {
            label,
            error: Box::new(error),
        }
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn error(&self) -> &ManagedResourceCommandError {
        &self.error
    }
}

type ArtifactDownloadKey = (String, String, String);

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

    fn refresh_manifest(
        &self,
        client: &(impl ResourceHttpClient + ?Sized),
        progress: &impl DownloadProgress,
        latest_only: bool,
    ) -> ManagedResourceCommandResult<ArtifactManifestRefresh> {
        self.refresh_manifest_result(client, progress, latest_only)
            .map_err(Into::into)
    }

    fn refresh_manifest_result(
        &self,
        client: &(impl ResourceHttpClient + ?Sized),
        progress: &impl DownloadProgress,
        latest_only: bool,
    ) -> crate::Result<ArtifactManifestRefresh> {
        let started_at = Instant::now();
        let cache = ArtifactManifestCache::new(self.paths.downloads());
        let result = if latest_only {
            cache.refresh_latest(&self.manifest_url, client)
        } else {
            cache.refresh(&self.manifest_url, client)
        };
        let outcome = match &result {
            Ok(refresh) => match refresh.source() {
                ArtifactManifestSource::Latest => ResourceOperationOutcome::Succeeded,
                ArtifactManifestSource::Cached { reason } => {
                    ResourceOperationOutcome::Fallback { reason }
                }
            },
            Err(_error) => ResourceOperationOutcome::Failed,
        };
        report_resource_operation(progress, ResourceOperation::Manifest, started_at, outcome);

        result
    }

    pub fn manifest_snapshot_with_progress(
        &self,
        client: &(impl ResourceHttpClient + ?Sized),
        progress: &impl DownloadProgress,
    ) -> crate::Result<ArtifactManifestRefresh> {
        self.refresh_manifest_result(client, progress, false)
    }

    pub fn latest_manifest_snapshot_with_progress(
        &self,
        client: &(impl ResourceHttpClient + ?Sized),
        progress: &impl DownloadProgress,
    ) -> crate::Result<ArtifactManifestRefresh> {
        self.refresh_manifest_result(client, progress, true)
    }

    pub fn resolve_install_artifact(
        &self,
        adapter: &(impl ResourceAdapter + ?Sized),
        track: TrackName,
        refresh: &ArtifactManifestRefresh,
    ) -> ManagedResourceCommandResult<ManagedResourceInstallArtifact> {
        registry::resolve_canonical(adapter.resource_name().as_str())?;
        let selection = refresh.manifest().select_latest(
            adapter.resource_name(),
            &track,
            self.target_platform,
        )?;
        let download_required = !ArtifactInstaller::new(self.paths.resources())
            .has_valid_existing_release(adapter, &track, selection.artifact())?;

        Ok(ManagedResourceInstallArtifact {
            artifact: selection.artifact().clone(),
            revoked_latest: selection
                .revoked_latest()
                .map(revoked_fallback_from_artifact),
            download_required,
            manifest_source: refresh.source().clone(),
            target_platform: self.target_platform,
        })
    }

    pub fn install_resolved_artifact_with_progress(
        &self,
        adapter: &(impl ResourceAdapter + ?Sized),
        resolved: ManagedResourceInstallArtifact,
        download: Option<&ArtifactDownload>,
        progress: &impl DownloadProgress,
    ) -> ManagedResourceCommandResult<ManagedResourceInstall> {
        validate_prefetched_install(self, adapter, &resolved, download)?;
        let mut database = Database::open(&self.paths)?;
        database.record_managed_resource_track_desired(
            adapter.resource_name().as_str(),
            resolved.artifact.track().as_str(),
            ManagedResourceDesiredState::Installed,
        )?;
        let install = self.install_prefetched_artifact(adapter, resolved, download, progress)?;
        if let Err(error) = database.record_managed_resource_track_installed(
            adapter.resource_name().as_str(),
            install.track.as_str(),
            install.artifact_version.as_str(),
            &install.current_artifact_path,
        ) {
            return Err(self.rollback_after_error(&[&install], error.into()));
        }

        Ok(install)
    }

    pub fn install_resolved_php_pair_with_progress(
        &self,
        php_resolved: ManagedResourceInstallArtifact,
        php_download: Option<&ArtifactDownload>,
        frankenphp_resolved: ManagedResourceInstallArtifact,
        frankenphp_download: Option<&ArtifactDownload>,
        progress: &impl DownloadProgress,
    ) -> ManagedResourceCommandResult<PhpPairInstall> {
        let php_adapter = php_adapter()?;
        let frankenphp_adapter = frankenphp_adapter()?;
        validate_prefetched_install(self, &php_adapter, &php_resolved, php_download)?;
        validate_prefetched_install(
            self,
            &frankenphp_adapter,
            &frankenphp_resolved,
            frankenphp_download,
        )?;
        if php_resolved.artifact.track() != frankenphp_resolved.artifact.track() {
            return Err(ResourcesError::InvalidArtifactLayout {
                resource: frankenphp_adapter.resource_name().as_str().to_string(),
                reason: format!(
                    "PHP pair tracks differ: php uses `{}`, frankenphp uses `{}`",
                    php_resolved.artifact.track(),
                    frankenphp_resolved.artifact.track()
                ),
            }
            .into());
        }
        let php =
            self.install_prefetched_artifact(&php_adapter, php_resolved, php_download, progress)?;
        let frankenphp = match self.install_prefetched_artifact(
            &frankenphp_adapter,
            frankenphp_resolved,
            frankenphp_download,
            progress,
        ) {
            Ok(install) => install,
            Err(error) => return Err(self.rollback_after_error(&[&php], error)),
        };
        let install = PhpPairInstall { php, frankenphp };
        if let Err(error) = self.record_php_pair_install(&install) {
            return Err(self.rollback_php_pair_after_error(&install, error));
        }

        Ok(install)
    }

    pub fn install(
        &self,
        adapter: &(impl ResourceAdapter + ?Sized),
        selector: TrackSelector,
        client: &(impl ResourceHttpClient + ?Sized),
    ) -> ManagedResourceCommandResult<ManagedResourceInstall> {
        self.install_with_progress(adapter, selector, client, &NoDownloadProgress)
    }

    pub fn install_with_progress(
        &self,
        adapter: &(impl ResourceAdapter + ?Sized),
        selector: TrackSelector,
        client: &(impl ResourceHttpClient + ?Sized),
        progress: &impl DownloadProgress,
    ) -> ManagedResourceCommandResult<ManagedResourceInstall> {
        registry::resolve_canonical(adapter.resource_name().as_str())?;

        let refresh = self.refresh_manifest(client, progress, false)?;
        let manifest = refresh.manifest();
        let track = manifest
            .resolve_track(adapter.resource_name(), selector)?
            .clone();

        self.install_track(adapter, track, manifest, refresh.source(), client, progress)
    }

    pub fn install_php_pair(
        &self,
        selector: TrackSelector,
        client: &(impl ResourceHttpClient + ?Sized),
    ) -> ManagedResourceCommandResult<PhpPairInstall> {
        self.install_php_pair_with_progress(selector, client, &NoDownloadProgress)
    }

    pub fn install_php_pair_with_progress(
        &self,
        selector: TrackSelector,
        client: &(impl ResourceHttpClient + ?Sized),
        progress: &impl DownloadProgress,
    ) -> ManagedResourceCommandResult<PhpPairInstall> {
        let php = php_adapter()?;
        let frankenphp = frankenphp_adapter()?;
        registry::resolve_canonical(php.resource_name().as_str())?;
        registry::resolve_canonical(frankenphp.resource_name().as_str())?;

        let refresh = self.refresh_manifest(client, progress, false)?;
        let manifest = refresh.manifest();
        let track = manifest
            .resolve_track(php.resource_name(), selector)?
            .clone();
        self.validate_install_selection(&php, &track, manifest)?;
        self.validate_install_selection(&frankenphp, &track, manifest)?;

        let context = ArtifactInstallContext {
            manifest_source: refresh.source(),
            client,
            progress,
        };
        let install = self.prepare_php_pair_install_with_progress(
            &php,
            &frankenphp,
            track,
            manifest,
            context,
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
        progress: &impl DownloadProgress,
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
        let context = ArtifactInstallContext {
            manifest_source,
            client,
            progress,
        };
        let install =
            self.install_selected_artifact(adapter, track, artifact, revoked_latest, context)?;
        if let Err(error) = database.record_managed_resource_track_installed(
            adapter.resource_name().as_str(),
            install.track.as_str(),
            install.artifact_version.as_str(),
            &install.current_artifact_path,
        ) {
            return Err(self.rollback_after_error(&[&install], error.into()));
        }

        Ok(install)
    }

    fn prepare_track_install_with_progress(
        &self,
        adapter: &(impl ResourceAdapter + ?Sized),
        track: TrackName,
        manifest: &ArtifactManifest,
        manifest_source: &ArtifactManifestSource,
        client: &(impl ResourceHttpClient + ?Sized),
        progress: &impl DownloadProgress,
    ) -> ManagedResourceCommandResult<ManagedResourceInstall> {
        let selection =
            manifest.select_latest(adapter.resource_name(), &track, self.target_platform)?;
        let revoked_latest = selection
            .revoked_latest()
            .map(revoked_fallback_from_artifact);
        let artifact = selection.artifact().clone();
        let context = ArtifactInstallContext {
            manifest_source,
            client,
            progress,
        };

        self.install_selected_artifact(adapter, track, artifact, revoked_latest, context)
    }

    fn install_selected_artifact<Client, Progress>(
        &self,
        adapter: &(impl ResourceAdapter + ?Sized),
        track: TrackName,
        artifact: ManifestArtifact,
        revoked_latest: Option<ManagedResourceRevokedLatest>,
        context: ArtifactInstallContext<'_, Client, Progress>,
    ) -> ManagedResourceCommandResult<ManagedResourceInstall>
    where
        Client: ResourceHttpClient + ?Sized,
        Progress: DownloadProgress,
    {
        let installer = ArtifactInstaller::new(self.paths.resources());
        let existing_started_at = Instant::now();
        let existing_install = installer.install_existing_release(adapter, &track, &artifact);
        let (install, downloaded_from_cache) = match existing_install {
            Ok(Some(existing_install)) => {
                report_resource_operation(
                    context.progress,
                    ResourceOperation::Install(&artifact),
                    existing_started_at,
                    ResourceOperationOutcome::Succeeded,
                );
                (existing_install, false)
            }
            Ok(None) => {
                let download = ArtifactDownloader::new(self.paths.downloads())
                    .download_with_progress(&artifact, context.client, context.progress)?;
                let install_started_at = Instant::now();
                let install =
                    installer.install(adapter, &track, &artifact, download.install_path());
                report_resource_operation(
                    context.progress,
                    ResourceOperation::Install(&artifact),
                    install_started_at,
                    ResourceOperationOutcome::from_succeeded(install.is_ok()),
                );

                (install?, download.is_from_cache())
            }
            Err(error) => {
                report_resource_operation(
                    context.progress,
                    ResourceOperation::Install(&artifact),
                    existing_started_at,
                    ResourceOperationOutcome::Failed,
                );

                return Err(error.into());
            }
        };

        let current_artifact_path = install.release_path().to_path_buf();

        Ok(ManagedResourceInstall {
            resource_name: adapter.resource_name().clone(),
            track,
            artifact_version: artifact.artifact_version().clone(),
            current_artifact_path,
            manifest_source: context.manifest_source.clone(),
            revoked_latest,
            downloaded_from_cache,
            artifact_install: install,
        })
    }

    fn install_prefetched_artifact(
        &self,
        adapter: &(impl ResourceAdapter + ?Sized),
        resolved: ManagedResourceInstallArtifact,
        download: Option<&ArtifactDownload>,
        progress: &impl DownloadProgress,
    ) -> ManagedResourceCommandResult<ManagedResourceInstall> {
        let ManagedResourceInstallArtifact {
            artifact,
            revoked_latest,
            manifest_source,
            ..
        } = resolved;
        let track = artifact.track().clone();
        let started_at = Instant::now();
        let installer = ArtifactInstaller::new(self.paths.resources());
        let install = match installer.install_existing_release(adapter, &track, &artifact) {
            Ok(Some(install)) => Ok((install, false)),
            Ok(None) => {
                let download = download.ok_or_else(|| ResourcesError::MissingArtifactDownload {
                    resource: artifact.resource_name().as_str().to_string(),
                    artifact_version: artifact.artifact_version().as_str().to_string(),
                })?;
                installer
                    .install(adapter, &track, &artifact, download.install_path())
                    .map(|install| (install, download.is_from_cache()))
            }
            Err(error) => Err(error),
        };
        report_resource_operation(
            progress,
            ResourceOperation::Install(&artifact),
            started_at,
            ResourceOperationOutcome::from_succeeded(install.is_ok()),
        );
        let (install, downloaded_from_cache) = install?;
        let current_artifact_path = install.release_path().to_path_buf();

        Ok(ManagedResourceInstall {
            resource_name: adapter.resource_name().clone(),
            track,
            artifact_version: artifact.artifact_version().clone(),
            current_artifact_path,
            manifest_source,
            revoked_latest,
            downloaded_from_cache,
            artifact_install: install,
        })
    }

    fn prepare_php_pair_install_with_progress<Client, Progress>(
        &self,
        php: &impl ResourceAdapter,
        frankenphp: &impl ResourceAdapter,
        track: TrackName,
        manifest: &ArtifactManifest,
        context: ArtifactInstallContext<'_, Client, Progress>,
    ) -> ManagedResourceCommandResult<PhpPairInstall>
    where
        Client: ResourceHttpClient + ?Sized,
        Progress: DownloadProgress,
    {
        let php = self.prepare_track_install_with_progress(
            php,
            track.clone(),
            manifest,
            context.manifest_source,
            context.client,
            context.progress,
        )?;
        let frankenphp = match self.prepare_track_install_with_progress(
            frankenphp,
            track,
            manifest,
            context.manifest_source,
            context.client,
            context.progress,
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

    fn ensure_php_pair_defaults(
        &self,
        install: &PhpPairInstall,
    ) -> ManagedResourceCommandResult<()> {
        crate::php_defaults::ensure_php_track_defaults(&self.paths, install.php.track.as_str())?;

        Ok(())
    }

    fn record_php_pair_install(
        &self,
        install: &PhpPairInstall,
    ) -> ManagedResourceCommandResult<()> {
        self.ensure_php_pair_defaults(install)?;

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
        self.ensure_php_pair_defaults(php_pair)?;

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
        self.update_with_progress(adapter, client, &NoDownloadProgress)
    }

    pub fn update_with_progress(
        &self,
        adapter: &(impl ResourceAdapter + ?Sized),
        client: &(impl ResourceHttpClient + ?Sized),
        progress: &impl DownloadProgress,
    ) -> ManagedResourceCommandResult<ManagedResourceUpdate> {
        registry::resolve_canonical(adapter.resource_name().as_str())?;

        let installed_tracks = self.list(Some(adapter.resource_name()))?;
        let mut installs = Vec::new();
        if installed_tracks.is_empty() {
            return Ok(ManagedResourceUpdate { installs });
        }
        for record in &installed_tracks {
            self.validate_installed_track(record)?;
        }

        let refresh = self.refresh_manifest(client, progress, true)?;

        for record in installed_tracks {
            installs.push(self.install_track(
                adapter,
                record.track,
                refresh.manifest(),
                refresh.source(),
                client,
                progress,
            )?);
        }

        Ok(ManagedResourceUpdate { installs })
    }

    pub fn update_php_pairs(
        &self,
        client: &(impl ResourceHttpClient + ?Sized),
    ) -> ManagedResourceCommandResult<PhpPairUpdate> {
        self.update_php_pairs_with_progress(client, &NoDownloadProgress)
    }

    pub fn update_php_pairs_with_progress(
        &self,
        client: &(impl ResourceHttpClient + ?Sized),
        progress: &impl DownloadProgress,
    ) -> ManagedResourceCommandResult<PhpPairUpdate> {
        let php = php_adapter()?;
        let frankenphp = frankenphp_adapter()?;
        let mut tracks = BTreeSet::new();

        for record in self.list(Some(php.resource_name()))? {
            self.validate_installed_track(&record)?;
            tracks.insert(record.track().clone());
        }
        for record in self.list(Some(frankenphp.resource_name()))? {
            self.validate_installed_track(&record)?;
            tracks.insert(record.track().clone());
        }

        let mut installs = Vec::new();
        if tracks.is_empty() {
            return Ok(PhpPairUpdate { installs });
        }

        let refresh = self.refresh_manifest(client, progress, true)?;

        for track in &tracks {
            self.validate_install_selection(&php, track, refresh.manifest())?;
            self.validate_install_selection(&frankenphp, track, refresh.manifest())?;
        }

        for track in tracks {
            let context = ArtifactInstallContext {
                manifest_source: refresh.source(),
                client,
                progress,
            };
            let install = self.prepare_php_pair_install_with_progress(
                &php,
                &frankenphp,
                track,
                refresh.manifest(),
                context,
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
        self.install_with_progress(
            &composer_adapter()?,
            TrackSelector::Track(composer_track()?),
            client,
            &NoDownloadProgress,
        )
    }

    pub fn install_composer_with_progress(
        &self,
        client: &(impl ResourceHttpClient + ?Sized),
        progress: &impl DownloadProgress,
    ) -> ManagedResourceCommandResult<ManagedResourceInstall> {
        self.install_with_progress(
            &composer_adapter()?,
            TrackSelector::Track(composer_track()?),
            client,
            progress,
        )
    }

    pub fn install_composer_with_php_pair(
        &self,
        php_selector: TrackSelector,
        client: &(impl ResourceHttpClient + ?Sized),
    ) -> ManagedResourceCommandResult<ComposerWithPhpPairInstall> {
        self.install_composer_with_php_pair_and_progress(php_selector, client, &NoDownloadProgress)
    }

    pub fn install_composer_with_php_pair_and_progress(
        &self,
        php_selector: TrackSelector,
        client: &(impl ResourceHttpClient + ?Sized),
        progress: &impl DownloadProgress,
    ) -> ManagedResourceCommandResult<ComposerWithPhpPairInstall> {
        let php = php_adapter()?;
        let frankenphp = frankenphp_adapter()?;
        let composer = composer_adapter()?;
        registry::resolve_canonical(php.resource_name().as_str())?;
        registry::resolve_canonical(frankenphp.resource_name().as_str())?;
        registry::resolve_canonical(composer.resource_name().as_str())?;

        let refresh = self.refresh_manifest(client, progress, false)?;
        let manifest = refresh.manifest();
        let php_track = manifest
            .resolve_track(php.resource_name(), php_selector)?
            .clone();
        let composer_track = composer_track()?;
        self.validate_install_selection(&php, &php_track, manifest)?;
        self.validate_install_selection(&frankenphp, &php_track, manifest)?;
        self.validate_install_selection(&composer, &composer_track, manifest)?;

        let context = ArtifactInstallContext {
            manifest_source: refresh.source(),
            client,
            progress,
        };
        let php_pair = self.prepare_php_pair_install_with_progress(
            &php,
            &frankenphp,
            php_track,
            manifest,
            context,
        )?;
        let composer = match self.prepare_track_install_with_progress(
            &composer,
            composer_track,
            manifest,
            refresh.source(),
            client,
            progress,
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
        self.update_composer_with_progress(client, &NoDownloadProgress)
    }

    pub fn update_composer_with_progress(
        &self,
        client: &(impl ResourceHttpClient + ?Sized),
        progress: &impl DownloadProgress,
    ) -> ManagedResourceCommandResult<ManagedResourceUpdate> {
        let composer = composer_adapter()?;
        let track = composer_track()?;
        let installed_tracks = self.list(Some(composer.resource_name()))?;
        let mut installs = Vec::new();
        let Some(installed) = installed_tracks
            .iter()
            .find(|record| record.track() == &track)
        else {
            return Ok(ManagedResourceUpdate { installs });
        };
        self.validate_installed_track(installed)?;

        let refresh = self.refresh_manifest(client, progress, true)?;
        installs.push(self.install_track(
            &composer,
            track,
            refresh.manifest(),
            refresh.source(),
            client,
            progress,
        )?);

        Ok(ManagedResourceUpdate { installs })
    }

    pub fn update_all_installed(
        &self,
        resource_adapters: &[&dyn ResourceAdapter],
        client: &(impl ResourceHttpClient + ?Sized),
    ) -> ManagedResourceCommandResult<ManagedResourceUpdate> {
        self.update_all_installed_with_progress(resource_adapters, client, &NoDownloadProgress)
    }

    pub fn update_all_installed_with_progress(
        &self,
        resource_adapters: &[&dyn ResourceAdapter],
        client: &(impl ResourceHttpClient + ?Sized),
        progress: &impl DownloadProgress,
    ) -> ManagedResourceCommandResult<ManagedResourceUpdate> {
        validate_resource_adapters(resource_adapters)?;
        let refresh = self.refresh_manifest(client, progress, true)?;

        self.update_all_installed_from_manifest_with_progress(
            resource_adapters,
            &refresh,
            client,
            progress,
        )
    }

    pub fn update_all_installed_from_manifest_with_progress(
        &self,
        resource_adapters: &[&dyn ResourceAdapter],
        refresh: &ArtifactManifestRefresh,
        client: &(impl ResourceHttpClient + ?Sized),
        progress: &impl DownloadProgress,
    ) -> ManagedResourceCommandResult<ManagedResourceUpdate> {
        validate_resource_adapters(resource_adapters)?;

        let manifest = refresh.manifest();
        let installed_tracks = self.list(None)?;
        let mut installs = Vec::new();
        let context = ArtifactInstallContext {
            manifest_source: refresh.source(),
            client,
            progress,
        };

        if let Err(error) =
            self.update_installed_php_pairs(&installed_tracks, manifest, &mut installs, context)
        {
            return Err(partial_update_error(error, installs));
        }
        if let Err(error) =
            self.update_installed_composer(&installed_tracks, manifest, &mut installs, context)
        {
            return Err(partial_update_error(error, installs));
        }
        if let Err(error) = self.update_installed_resources(
            &installed_tracks,
            resource_adapters,
            manifest,
            &mut installs,
            context,
        ) {
            return Err(partial_update_error(error, installs));
        }

        Ok(ManagedResourceUpdate { installs })
    }

    pub fn update_all_installed_from_manifest_prefetched_with_progress(
        &self,
        resource_adapters: &[&dyn ResourceAdapter],
        refresh: &ArtifactManifestRefresh,
        client: &(impl ResourceHttpClient + Sync + ?Sized),
        progress: &(impl DownloadProgress + Sync),
    ) -> ManagedResourceCommandResult<ManagedResourceUpdate> {
        validate_resource_adapters(resource_adapters)?;
        let manifest = refresh.manifest();
        let installed_tracks = self.list(None)?;
        let mut plan = Vec::new();
        let mut planning_error = None;
        let php = php_adapter()?;
        let frankenphp = frankenphp_adapter()?;
        let mut php_tracks = BTreeSet::new();
        collect_installed_tracks(&installed_tracks, php.resource_name(), &mut php_tracks);
        collect_installed_tracks(
            &installed_tracks,
            frankenphp.resource_name(),
            &mut php_tracks,
        );

        for track in &php_tracks {
            self.validate_install_selection(&php, track, manifest)?;
            self.validate_install_selection(&frankenphp, track, manifest)?;
        }
        for track in php_tracks {
            let failure_label = format!("php/frankenphp {track}");
            let result = (|| {
                let php_installed =
                    find_installed_track(&installed_tracks, php.resource_name(), &track);
                let frankenphp_installed =
                    find_installed_track(&installed_tracks, frankenphp.resource_name(), &track);
                if !self.track_needs_update(&php, &track, php_installed, manifest)?
                    && !self.track_needs_update(
                        &frankenphp,
                        &track,
                        frankenphp_installed,
                        manifest,
                    )?
                {
                    return Ok(None);
                }
                let php = self.resolve_install_artifact(&php, track.clone(), refresh)?;
                let frankenphp = self.resolve_install_artifact(&frankenphp, track, refresh)?;

                Ok(Some(PrefetchedUpdate::PhpPair {
                    php,
                    frankenphp: Box::new(frankenphp),
                }))
            })();
            match result {
                Ok(Some(update)) => plan.push(update),
                Ok(None) => {}
                Err(error) => {
                    planning_error = Some(ManagedResourceUpdateFailure::new(failure_label, error));
                    break;
                }
            }
        }

        let composer = composer_adapter()?;
        let composer_track = composer_track()?;
        if planning_error.is_none()
            && let Some(installed) =
                find_installed_track(&installed_tracks, composer.resource_name(), &composer_track)
        {
            match self.track_needs_update(&composer, &composer_track, Some(installed), manifest) {
                Ok(true) => match self.resolve_install_artifact(&composer, composer_track, refresh)
                {
                    Ok(resolved) => plan.push(PrefetchedUpdate::Artifact {
                        adapter: &composer,
                        resolved,
                    }),
                    Err(error) => {
                        planning_error = Some(ManagedResourceUpdateFailure::new(
                            "composer 2".to_string(),
                            error,
                        ));
                    }
                },
                Ok(false) => {}
                Err(error) => {
                    planning_error = Some(ManagedResourceUpdateFailure::new(
                        "composer 2".to_string(),
                        error,
                    ));
                }
            }
        }

        'adapters: for adapter in resource_adapters {
            if planning_error.is_some() {
                break;
            }
            for installed in installed_tracks
                .iter()
                .filter(|track| track.resource_name() == adapter.resource_name())
            {
                let needs_update = match self.track_needs_update(
                    *adapter,
                    installed.track(),
                    Some(installed),
                    manifest,
                ) {
                    Ok(needs_update) => needs_update,
                    Err(error) => {
                        planning_error = Some(ManagedResourceUpdateFailure::new(
                            format!("{} {}", adapter.resource_name(), installed.track()),
                            error,
                        ));
                        break 'adapters;
                    }
                };
                if !needs_update {
                    continue;
                }
                match self.resolve_install_artifact(*adapter, installed.track().clone(), refresh) {
                    Ok(resolved) => plan.push(PrefetchedUpdate::Artifact {
                        adapter: *adapter,
                        resolved,
                    }),
                    Err(error) => {
                        planning_error = Some(ManagedResourceUpdateFailure::new(
                            format!("{} {}", adapter.resource_name(), installed.track()),
                            error,
                        ));
                        break 'adapters;
                    }
                }
            }
        }

        let artifacts = unique_prefetched_update_artifacts(&plan);
        let artifact_values = artifacts.values().cloned().collect::<Vec<_>>();
        let download_results = ArtifactDownloader::new(self.paths.downloads())
            .download_many_with_progress(&artifact_values, client, progress);
        let downloads = artifacts
            .into_keys()
            .zip(download_results)
            .collect::<BTreeMap<_, _>>();
        let mut installs = Vec::new();
        for index in 0..plan.len() {
            let current_download_failures = prefetched_update_failures(&downloads, &plan[index]);
            if !current_download_failures.is_empty() {
                let download_failures = current_download_failures
                    .into_iter()
                    .chain(
                        plan[index + 1..]
                            .iter()
                            .flat_map(|update| prefetched_update_failures(&downloads, update)),
                    )
                    .chain(planning_error.take())
                    .collect();
                return Err(partial_update_error(
                    combined_update_error(download_failures),
                    installs,
                ));
            }
            let update = plan[index].clone();
            let update_label = update.label();
            let result = match update {
                PrefetchedUpdate::PhpPair { php, frankenphp } => {
                    match (
                        prefetched_update_download(&downloads, &php),
                        prefetched_update_download(&downloads, &frankenphp),
                    ) {
                        (Ok(php_download), Ok(frankenphp_download)) => self
                            .install_resolved_php_pair_with_progress(
                                php,
                                php_download,
                                *frankenphp,
                                frankenphp_download,
                                progress,
                            )
                            .map(|pair| vec![pair.php, pair.frankenphp]),
                        (Err(error), _) | (_, Err(error)) => Err(error),
                    }
                }
                PrefetchedUpdate::Artifact { adapter, resolved } => {
                    match prefetched_update_download(&downloads, &resolved) {
                        Ok(download) => self
                            .install_resolved_artifact_with_progress(
                                adapter, resolved, download, progress,
                            )
                            .map(|install| vec![install]),
                        Err(error) => Err(error),
                    }
                }
            };
            match result {
                Ok(update_installs) => installs.extend(update_installs),
                Err(error) => {
                    let failures =
                        std::iter::once(ManagedResourceUpdateFailure::new(update_label, error))
                            .chain(
                                plan[index + 1..].iter().flat_map(|update| {
                                    prefetched_update_failures(&downloads, update)
                                }),
                            )
                            .chain(planning_error.take())
                            .collect();
                    return Err(partial_update_error(
                        combined_update_error(failures),
                        installs,
                    ));
                }
            }
        }
        if let Some(error) = planning_error {
            return Err(partial_update_error(
                combined_update_error(vec![error]),
                installs,
            ));
        }

        Ok(ManagedResourceUpdate { installs })
    }

    fn update_installed_php_pairs<Client, Progress>(
        &self,
        installed_tracks: &[ManagedResourceTrack],
        manifest: &ArtifactManifest,
        installs: &mut Vec<ManagedResourceInstall>,
        context: ArtifactInstallContext<'_, Client, Progress>,
    ) -> ManagedResourceCommandResult<()>
    where
        Client: ResourceHttpClient + ?Sized,
        Progress: DownloadProgress,
    {
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

            let install = self.prepare_php_pair_install_with_progress(
                &php,
                &frankenphp,
                track,
                manifest,
                context,
            )?;
            if let Err(error) = self.record_php_pair_install(&install) {
                return Err(self.rollback_php_pair_after_error(&install, error));
            }
            installs.push(install.php);
            installs.push(install.frankenphp);
        }

        Ok(())
    }

    fn update_installed_composer<Client, Progress>(
        &self,
        installed_tracks: &[ManagedResourceTrack],
        manifest: &ArtifactManifest,
        installs: &mut Vec<ManagedResourceInstall>,
        context: ArtifactInstallContext<'_, Client, Progress>,
    ) -> ManagedResourceCommandResult<()>
    where
        Client: ResourceHttpClient + ?Sized,
        Progress: DownloadProgress,
    {
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

        installs.push(self.install_track(
            &composer,
            track,
            manifest,
            context.manifest_source,
            context.client,
            context.progress,
        )?);

        Ok(())
    }

    fn update_installed_resources<Client, Progress>(
        &self,
        installed_tracks: &[ManagedResourceTrack],
        resource_adapters: &[&dyn ResourceAdapter],
        manifest: &ArtifactManifest,
        installs: &mut Vec<ManagedResourceInstall>,
        context: ArtifactInstallContext<'_, Client, Progress>,
    ) -> ManagedResourceCommandResult<()>
    where
        Client: ResourceHttpClient + ?Sized,
        Progress: DownloadProgress,
    {
        for adapter in resource_adapters {
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
                    context.manifest_source,
                    context.client,
                    context.progress,
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
        self.validate_installed_track(installed)?;

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

    fn validate_installed_track(
        &self,
        installed: &ManagedResourceTrack,
    ) -> ManagedResourceCommandResult<()> {
        ArtifactInstaller::new(self.paths.resources()).validate_installed_release(
            installed.resource_name(),
            installed.track(),
            installed.installed_version(),
            installed.current_artifact_path(),
        )?;

        Ok(())
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

fn report_resource_operation(
    progress: &(impl DownloadProgress + ?Sized),
    operation: ResourceOperation<'_>,
    started_at: Instant,
    outcome: ResourceOperationOutcome<'_>,
) {
    progress.operation_finished(ResourceOperationEvent {
        operation,
        elapsed: started_at.elapsed(),
        outcome,
    });
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

impl ManagedResourceInstallArtifact {
    pub fn artifact(&self) -> &ManifestArtifact {
        &self.artifact
    }

    pub fn download_required(&self) -> bool {
        self.download_required
    }

    pub fn target_platform(&self) -> TargetPlatform {
        self.target_platform
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

    pub fn rollback_caddy(&self, paths: &PvPaths) -> ManagedResourceCommandResult<bool> {
        let Some(install) = self
            .installs
            .iter()
            .find(|install| install.resource_name.as_str() == "caddy")
        else {
            return Ok(false);
        };
        let Some(previous_release) = install.artifact_install.previous_release() else {
            return Ok(false);
        };
        let Some(releases_dir) = install.current_artifact_path.parent() else {
            return Err(ResourcesError::InvalidArtifactLayout {
                resource: install.resource_name.as_str().to_string(),
                reason: format!(
                    "installed artifact path `{}` has no release directory",
                    install.current_artifact_path
                ),
            }
            .into());
        };
        let previous_artifact_path = releases_dir.join(previous_release);

        let installer = ArtifactInstaller::new(paths.resources());
        installer.switch_to_previous_release(&install.artifact_install)?;
        let record_previous_release = || -> ManagedResourceCommandResult<()> {
            let mut database = Database::open(paths)?;
            database.record_managed_resource_track_installed(
                install.resource_name.as_str(),
                install.track.as_str(),
                previous_release,
                &previous_artifact_path,
            )?;

            Ok(())
        };
        if let Err(original_error) = record_previous_release() {
            if let Err(rollback_error) =
                installer.switch_to_installed_release(&install.artifact_install)
            {
                return Err(ManagedResourceCommandError::RollbackFailed {
                    original_error: Box::new(original_error),
                    rollback_error,
                });
            }

            return Err(original_error);
        }

        Ok(true)
    }
}

fn partial_update_error(
    error: ManagedResourceCommandError,
    installs: Vec<ManagedResourceInstall>,
) -> ManagedResourceCommandError {
    if installs.is_empty() {
        return error;
    }

    ManagedResourceCommandError::PartialUpdate {
        source: Box::new(error),
        update: ManagedResourceUpdate { installs },
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

fn validate_resolved_install_artifact(
    commands: &ManagedResourceCommands,
    adapter: &(impl ResourceAdapter + ?Sized),
    resolved: &ManagedResourceInstallArtifact,
) -> ManagedResourceCommandResult<()> {
    registry::resolve_canonical(adapter.resource_name().as_str())?;
    if resolved.target_platform != commands.target_platform {
        return Err(ResourcesError::InvalidArtifactLayout {
            resource: adapter.resource_name().as_str().to_string(),
            reason: format!(
                "artifact was resolved for {}, not {}",
                resolved.target_platform, commands.target_platform
            ),
        }
        .into());
    }
    validate_artifact_matches_request(
        adapter.resource_name(),
        resolved.artifact.track(),
        &resolved.artifact,
    )?;

    Ok(())
}

fn validate_prefetched_install(
    commands: &ManagedResourceCommands,
    adapter: &(impl ResourceAdapter + ?Sized),
    resolved: &ManagedResourceInstallArtifact,
    download: Option<&ArtifactDownload>,
) -> ManagedResourceCommandResult<()> {
    validate_resolved_install_artifact(commands, adapter, resolved)?;
    if !resolved.download_required() {
        return Ok(());
    }
    let download = download.ok_or_else(|| ResourcesError::MissingArtifactDownload {
        resource: resolved.artifact.resource_name().as_str().to_string(),
        artifact_version: resolved.artifact.artifact_version().as_str().to_string(),
    })?;
    download.validate_for(&resolved.artifact)?;

    Ok(())
}

fn validate_resource_adapters(
    resource_adapters: &[&dyn ResourceAdapter],
) -> ManagedResourceCommandResult<()> {
    for adapter in resource_adapters {
        registry::resolve_canonical(adapter.resource_name().as_str())?;
    }

    Ok(())
}

fn unique_prefetched_update_artifacts(
    plan: &[PrefetchedUpdate<'_>],
) -> BTreeMap<ArtifactDownloadKey, ManifestArtifact> {
    let mut artifacts = BTreeMap::new();
    for update in plan {
        match update {
            PrefetchedUpdate::PhpPair { php, frankenphp } => {
                insert_required_update_artifact(&mut artifacts, php);
                insert_required_update_artifact(&mut artifacts, frankenphp);
            }
            PrefetchedUpdate::Artifact { resolved, .. } => {
                insert_required_update_artifact(&mut artifacts, resolved);
            }
        }
    }

    artifacts
}

fn insert_required_update_artifact(
    artifacts: &mut BTreeMap<ArtifactDownloadKey, ManifestArtifact>,
    resolved: &ManagedResourceInstallArtifact,
) {
    if !resolved.download_required() {
        return;
    }
    artifacts
        .entry(artifact_download_key(resolved.artifact()))
        .or_insert_with(|| resolved.artifact().clone());
}

fn artifact_download_key(artifact: &ManifestArtifact) -> ArtifactDownloadKey {
    (
        artifact.resource_name().as_str().to_string(),
        artifact.artifact_version().as_str().to_string(),
        artifact.sha256().as_str().to_string(),
    )
}

fn prefetched_update_download<'download>(
    downloads: &'download BTreeMap<ArtifactDownloadKey, crate::Result<ArtifactDownload>>,
    resolved: &ManagedResourceInstallArtifact,
) -> ManagedResourceCommandResult<Option<&'download ArtifactDownload>> {
    if !resolved.download_required() {
        return Ok(None);
    }
    let artifact = resolved.artifact();
    downloads
        .get(&artifact_download_key(artifact))
        .ok_or_else(|| ResourcesError::MissingArtifactDownload {
            resource: artifact.resource_name().as_str().to_string(),
            artifact_version: artifact.artifact_version().as_str().to_string(),
        })?
        .as_ref()
        .map(Some)
        .map_err(|error| error.clone().into())
}

fn prefetched_update_failures(
    downloads: &BTreeMap<ArtifactDownloadKey, crate::Result<ArtifactDownload>>,
    update: &PrefetchedUpdate<'_>,
) -> Vec<ManagedResourceUpdateFailure> {
    let mut failures = Vec::new();
    match update {
        PrefetchedUpdate::PhpPair { php, frankenphp } => {
            for (resource_name, resolved) in [("php", php), ("frankenphp", frankenphp)] {
                if let Err(error) = prefetched_update_download(downloads, resolved) {
                    failures.push(ManagedResourceUpdateFailure::new(
                        format!("{resource_name} {}", resolved.artifact().track()),
                        error,
                    ));
                }
            }
        }
        PrefetchedUpdate::Artifact { resolved, .. } => {
            if let Err(error) = prefetched_update_download(downloads, resolved) {
                failures.push(ManagedResourceUpdateFailure::new(update.label(), error));
            }
        }
    }

    failures
}

fn combined_update_error(
    mut failures: Vec<ManagedResourceUpdateFailure>,
) -> ManagedResourceCommandError {
    if failures.len() == 1 {
        return *failures.remove(0).error;
    }

    ManagedResourceCommandError::UpdateFailures { failures }
}

fn format_update_failures(failures: &[ManagedResourceUpdateFailure]) -> String {
    failures
        .iter()
        .map(|failure| format!("{}: {}", failure.label, failure.error))
        .collect::<Vec<_>>()
        .join("; ")
}

fn composer_track() -> ManagedResourceCommandResult<TrackName> {
    Ok(TrackName::new("2")?)
}
