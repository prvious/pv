#[cfg(test)]
mod fake;
mod mailpit;
pub(crate) mod mysql;
#[cfg(test)]
mod mysql_tests;
mod postgres;
mod redis;
mod rustfs;
pub(crate) mod sql;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::io;
use std::net::TcpListener;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use camino::Utf8Path;
use protocol::{
    ManagedResourceUpdateCheck as ProtocolUpdateCheck,
    ManagedResourceUpdateCheckTrack as ProtocolUpdateCheckTrack,
};
use resources::{ManagedResourceCommands, ResourceAdapter, TrackName};
use state::{
    Database, EnvContextValues, ManagedResourceDesiredState, ManagedResourceTrackRecord, PortOwner,
    PortRequest, PostgresPreloadLibrary, ProjectRecord, PvPaths, RUNTIME_PORT_FALLBACK_END,
    RUNTIME_PORT_FALLBACK_START, ResourceAllocationRecord, RuntimeObservedStatus, RuntimeSubject,
    StateError,
};
use tokio::time::{sleep, timeout};

use crate::jobs::DaemonDownloadProgress;
use crate::project_env::DemandedResourceTrack;
use crate::{
    DaemonError, ManagedResourceProjectFailure, ProcessSpec, ProcessSupervisor, ReadinessCheck,
    wait_for_readiness,
};

const RESOURCE_HOST: &str = "127.0.0.1";
const RESOURCE_READINESS_TIMEOUT: Duration = Duration::from_secs(15);
const ASYNC_READINESS_POLL_INTERVAL: Duration = Duration::from_millis(50);
const RESOURCE_STOP_GRACE_PERIOD: Duration = Duration::from_secs(10);
const RESOURCE_PROCESS_EXIT_POLL_INTERVAL: Duration = Duration::from_millis(25);
const RESOURCE_START_ATTEMPTS: usize = 10;
const RESERVED_RESOURCE_PORT_NAME: &str = "default";

pub(crate) type ManagedResourceReadinessFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), DaemonError>> + Send + 'a>>;
pub(crate) type ManagedResourcePreparationFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), DaemonError>> + Send + 'a>>;
pub(crate) type ManagedResourceAllocationFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), DaemonError>> + Send + 'a>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ManagedResourcePortSpec {
    pub name: &'static str,
    pub preferred_port: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ManagedResourceRuntimeContext {
    pub resource_name: String,
    pub track: String,
    pub artifact_path: camino::Utf8PathBuf,
    pub data_dir: camino::Utf8PathBuf,
    pub ports: BTreeMap<String, u16>,
    pub env: EnvContextValues,
    pub postgres_preload_libraries: Vec<PostgresPreloadLibrary>,
}

pub(crate) enum ManagedResourceReadiness {
    TcpHttp(ReadinessCheck),
    Async(AsyncManagedResourceReadiness),
}

pub(crate) struct AsyncManagedResourceReadiness {
    name: String,
    check: Box<dyn Fn() -> ManagedResourceReadinessFuture<'static> + Send + Sync>,
}

impl ManagedResourceReadiness {
    pub(crate) fn async_check(
        name: impl Into<String>,
        check: impl Fn() -> ManagedResourceReadinessFuture<'static> + Send + Sync + 'static,
    ) -> Self {
        Self::Async(AsyncManagedResourceReadiness {
            name: name.into(),
            check: Box::new(check),
        })
    }
}

impl From<ReadinessCheck> for ManagedResourceReadiness {
    fn from(check: ReadinessCheck) -> Self {
        Self::TcpHttp(check)
    }
}

pub(crate) trait ManagedResourceRuntimeAdapter: Send + Sync {
    fn resource_name(&self) -> &'static str;

    fn artifact_adapter(&self) -> Result<resources::RuntimeArtifactAdapter, DaemonError>;

    fn port_specs(&self) -> &'static [ManagedResourcePortSpec];

    fn prepare_runtime<'a>(
        &'a self,
        _paths: &'a PvPaths,
        _context: &'a ManagedResourceRuntimeContext,
    ) -> ManagedResourcePreparationFuture<'a> {
        Box::pin(async { Ok(()) })
    }

    fn build_process_spec(
        &self,
        paths: &PvPaths,
        context: &ManagedResourceRuntimeContext,
    ) -> Result<ProcessSpec, DaemonError>;

    fn readiness(
        &self,
        context: &ManagedResourceRuntimeContext,
    ) -> Result<ManagedResourceReadiness, DaemonError>;

    #[cfg(test)]
    fn readiness_timeout(&self) -> Duration {
        RESOURCE_READINESS_TIMEOUT
    }

    fn resource_env(
        &self,
        context: &ManagedResourceRuntimeContext,
    ) -> Result<EnvContextValues, DaemonError>;

    fn reconcile_allocations<'a>(
        &'a self,
        _paths: &'a PvPaths,
        _database: &'a mut Database,
        _context: &'a ManagedResourceRuntimeContext,
        _resource_env: &'a EnvContextValues,
        _allocations: &'a [ResourceAllocationRecord],
    ) -> ManagedResourceAllocationFuture<'a> {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ManagedResourceInstallOptions {
    pub manifest_url: String,
    pub target_platform: resources::TargetPlatform,
}

pub(crate) struct ManagedResourceRuntimeCatalog {
    adapters: BTreeMap<&'static str, Box<dyn ManagedResourceRuntimeAdapter>>,
    install_options: ManagedResourceInstallOptions,
    http_client: Option<Arc<dyn resources::ResourceHttpClient + Send + Sync>>,
}

#[derive(Debug)]
pub(crate) struct ManagedResourceUpdateReport {
    pub installed_count: usize,
    pub updated_count: usize,
    update: resources::ManagedResourceUpdate,
    failure: Option<DaemonError>,
}

impl ManagedResourceUpdateReport {
    pub(crate) fn rollback_caddy(&self, paths: &PvPaths) -> Result<bool, DaemonError> {
        self.update.rollback_caddy(paths).map_err(DaemonError::from)
    }

    pub(crate) fn into_result(self) -> Result<Self, DaemonError> {
        let Some(source) = self.failure else {
            return Ok(self);
        };

        Err(DaemonError::ManagedResourcePartialUpdateFailed {
            update: self.update,
            source: Box::new(source),
        })
    }
}

impl ManagedResourceRuntimeCatalog {
    pub(crate) fn production() -> Result<Self, DaemonError> {
        let mut adapters: BTreeMap<&'static str, Box<dyn ManagedResourceRuntimeAdapter>> =
            BTreeMap::new();
        adapters.insert(
            mailpit::MailpitRuntimeAdapter::NAME,
            Box::new(mailpit::MailpitRuntimeAdapter::new()),
        );
        let redis = redis::RedisRuntimeAdapter::new();
        adapters.insert(redis.resource_name(), Box::new(redis));
        adapters.insert("rustfs", Box::new(rustfs::RustfsRuntimeAdapter));
        adapters.insert(
            mysql::RESOURCE_NAME,
            Box::new(mysql::MysqlRuntimeAdapter::new()),
        );
        let postgres = postgres::PostgresRuntimeAdapter::new();
        adapters.insert(postgres.resource_name(), Box::new(postgres));

        Ok(Self {
            adapters,
            install_options: ManagedResourceInstallOptions {
                manifest_url: resources::default_artifact_manifest_url().to_string(),
                target_platform: resources::TargetPlatform::current()?,
            },
            http_client: None,
        })
    }

    pub(crate) fn without_adapters() -> Result<Self, DaemonError> {
        Self::without_adapters_with_manifest_url(resources::default_artifact_manifest_url())
    }

    pub(crate) fn without_adapters_with_manifest_url(
        manifest_url: impl Into<String>,
    ) -> Result<Self, DaemonError> {
        Ok(Self {
            adapters: BTreeMap::new(),
            install_options: ManagedResourceInstallOptions {
                manifest_url: manifest_url.into(),
                target_platform: resources::TargetPlatform::current()?,
            },
            http_client: None,
        })
    }

    pub(crate) fn without_adapters_with_manifest_client(
        manifest_url: impl Into<String>,
        client: impl resources::ResourceHttpClient + Send + Sync + 'static,
    ) -> Result<Self, DaemonError> {
        Ok(Self {
            adapters: BTreeMap::new(),
            install_options: ManagedResourceInstallOptions {
                manifest_url: manifest_url.into(),
                target_platform: resources::TargetPlatform::current()?,
            },
            http_client: Some(Arc::new(client)),
        })
    }

    #[cfg(test)]
    pub(crate) fn with_adapter(
        install_options: ManagedResourceInstallOptions,
        adapter: impl ManagedResourceRuntimeAdapter + 'static,
    ) -> Self {
        let mut adapters: BTreeMap<&'static str, Box<dyn ManagedResourceRuntimeAdapter>> =
            BTreeMap::new();
        adapters.insert(adapter.resource_name(), Box::new(adapter));

        Self {
            adapters,
            install_options,
            http_client: None,
        }
    }

    fn adapter(&self, resource_name: &str) -> Option<&dyn ManagedResourceRuntimeAdapter> {
        self.adapters.get(resource_name).map(Box::as_ref)
    }

    fn artifact_adapters(&self) -> Result<Vec<resources::RuntimeArtifactAdapter>, DaemonError> {
        self.adapters
            .values()
            .map(|adapter| adapter.artifact_adapter())
            .collect()
    }
}

pub(crate) async fn reconcile_project_resources_with_progress(
    paths: &PvPaths,
    database: &mut Database,
    project: &ProjectRecord,
    plan: &crate::project_env::ProjectResourcePlan,
    demanded_tracks: &BTreeSet<DemandedResourceTrack>,
    progress: DaemonDownloadProgress,
) -> Result<(), DaemonError> {
    let catalog = ManagedResourceRuntimeCatalog::production()?;

    reconcile_project_resources_with_catalog_and_progress(
        paths,
        database,
        project,
        plan,
        &catalog,
        demanded_tracks,
        progress,
    )
    .await
}

pub(crate) async fn reconcile_project_resources_with_catalog_and_progress(
    paths: &PvPaths,
    database: &mut Database,
    project: &ProjectRecord,
    plan: &crate::project_env::ProjectResourcePlan,
    catalog: &ManagedResourceRuntimeCatalog,
    demanded_tracks: &BTreeSet<DemandedResourceTrack>,
    progress: DaemonDownloadProgress,
) -> Result<(), DaemonError> {
    let supervisor = ProcessSupervisor::new(paths.clone());
    let mut demanded_tracks = demanded_tracks.clone();
    demanded_tracks.extend(plan.resources.iter().map(|resource| {
        DemandedResourceTrack::new(resource.resource_name.clone(), resource.track.clone())
    }));

    stop_undemanded_catalog_runtimes(paths, database, catalog, &supervisor, &demanded_tracks)
        .await?;
    let install_requests = missing_project_install_requests(database, plan, catalog);
    let mut prefetched_installs =
        prefetch_missing_project_installs(paths, catalog, install_requests, progress.clone())
            .await?;
    let mut context = ResourceTrackReconciliationContext {
        catalog,
        supervisor: &supervisor,
        progress: &progress,
        prefetched_installs: &mut prefetched_installs,
    };

    for (index, resource) in plan.resources.iter().enumerate() {
        if let Err(error) =
            reconcile_resource_track(paths, database, project, plan, &mut context, resource).await
        {
            let mut failures = vec![ManagedResourceProjectFailure::new(
                resource.resource_name.clone(),
                resource.track.clone(),
                error,
            )];
            failures.extend(take_project_prefetch_failures(
                database,
                &plan.resources[index + 1..],
                context.prefetched_installs,
            )?);

            return Err(combined_project_resource_error(failures));
        }
    }

    Ok(())
}

pub(crate) async fn reconcile_system_resources_with_progress(
    paths: &PvPaths,
    demanded_tracks: &BTreeSet<DemandedResourceTrack>,
    progress: DaemonDownloadProgress,
) -> Result<(), DaemonError> {
    let catalog = ManagedResourceRuntimeCatalog::production()?;
    let mut database = Database::open(paths)?;

    reconcile_system_resources_with_catalog_and_progress(
        paths,
        &mut database,
        &catalog,
        demanded_tracks,
        progress,
    )
    .await
}

pub(crate) fn update_check(
    paths: PvPaths,
    catalog: Option<&ManagedResourceRuntimeCatalog>,
) -> Result<ProtocolUpdateCheck, DaemonError> {
    state::UpdateLock::require_no_update_in_progress(&paths)?;

    match catalog {
        Some(catalog) => update_check_with_catalog(paths, catalog),
        None => {
            let catalog = ManagedResourceRuntimeCatalog::production()?;

            update_check_with_catalog(paths, &catalog)
        }
    }
}

fn update_check_with_catalog(
    paths: PvPaths,
    catalog: &ManagedResourceRuntimeCatalog,
) -> Result<ProtocolUpdateCheck, DaemonError> {
    let commands = ManagedResourceCommands::new(
        paths,
        catalog.install_options.manifest_url.clone(),
        catalog.install_options.target_platform,
    );
    let check = if let Some(client) = catalog.http_client.as_deref() {
        commands.check_updates(client)?
    } else {
        let client = resources::UreqResourceHttpClient::default();

        commands.check_updates(&client)?
    };
    let managed_resources = check
        .tracks()
        .iter()
        .map(protocol_update_check_track)
        .collect();

    Ok(ProtocolUpdateCheck { managed_resources })
}

pub(crate) fn update_installed_with_progress(
    paths: PvPaths,
    catalog: Option<&ManagedResourceRuntimeCatalog>,
    progress: &DaemonDownloadProgress,
) -> Result<ManagedResourceUpdateReport, DaemonError> {
    match catalog {
        Some(catalog) => update_installed_with_catalog(paths, catalog, progress),
        None => {
            let catalog = ManagedResourceRuntimeCatalog::production()?;

            update_installed_with_catalog(paths, &catalog, progress)
        }
    }
}

fn update_installed_with_catalog(
    paths: PvPaths,
    catalog: &ManagedResourceRuntimeCatalog,
    progress: &DaemonDownloadProgress,
) -> Result<ManagedResourceUpdateReport, DaemonError> {
    let commands = ManagedResourceCommands::new(
        paths,
        catalog.install_options.manifest_url.clone(),
        catalog.install_options.target_platform,
    );
    let installed_count = commands.list(None)?.len();
    let artifact_adapters = update_artifact_adapters(catalog)?;
    let resource_adapters = artifact_adapters
        .iter()
        .map(|adapter| adapter as &dyn resources::ResourceAdapter)
        .collect::<Vec<_>>();
    let update_result = if let Some(client) = catalog.http_client.as_deref() {
        let snapshot = progress.latest_manifest_snapshot(&commands, client)?;
        commands.update_all_installed_from_manifest_prefetched_with_progress(
            &resource_adapters,
            &snapshot,
            client,
            progress,
        )
    } else {
        let client = resources::UreqResourceHttpClient::default();
        let snapshot = progress.latest_manifest_snapshot(&commands, &client)?;
        commands.update_all_installed_from_manifest_prefetched_with_progress(
            &resource_adapters,
            &snapshot,
            &client,
            progress,
        )
    };
    let (update, failure) = match update_result {
        Ok(update) => (update, None),
        Err(resources::ManagedResourceCommandError::PartialUpdate { source, update }) => {
            (update, Some((*source).into()))
        }
        Err(error) => return Err(error.into()),
    };

    Ok(ManagedResourceUpdateReport {
        installed_count,
        updated_count: update.installs().len(),
        update,
        failure,
    })
}

fn update_artifact_adapters(
    catalog: &ManagedResourceRuntimeCatalog,
) -> Result<Vec<resources::RuntimeArtifactAdapter>, DaemonError> {
    let mut artifact_adapters = catalog.artifact_adapters()?;
    artifact_adapters.push(resources::caddy_adapter()?);

    Ok(artifact_adapters)
}

fn protocol_update_check_track(
    track: &resources::ManagedResourceUpdateCheckTrack,
) -> ProtocolUpdateCheckTrack {
    track.clone().into()
}

#[cfg(test)]
pub(crate) async fn reconcile_system_resources_with_catalog(
    paths: &PvPaths,
    database: &mut Database,
    catalog: &ManagedResourceRuntimeCatalog,
) -> Result<(), DaemonError> {
    reconcile_system_resources_with_catalog_and_progress(
        paths,
        database,
        catalog,
        &BTreeSet::new(),
        DaemonDownloadProgress::disabled(),
    )
    .await
}

pub(crate) async fn reconcile_system_resources_with_catalog_and_progress(
    paths: &PvPaths,
    database: &mut Database,
    catalog: &ManagedResourceRuntimeCatalog,
    demanded_tracks: &BTreeSet<DemandedResourceTrack>,
    progress: DaemonDownloadProgress,
) -> Result<(), DaemonError> {
    database.record_managed_resource_track_desired(
        "caddy",
        "2",
        ManagedResourceDesiredState::Installed,
    )?;
    let supervisor = ProcessSupervisor::new(paths.clone());

    stop_undemanded_catalog_runtimes(paths, database, catalog, &supervisor, demanded_tracks)
        .await?;

    install_missing_resource_demands_with_catalog_and_progress(
        paths,
        catalog,
        demanded_tracks,
        progress,
    )
    .await
}

pub(crate) async fn install_missing_resource_demands_with_catalog_and_progress(
    paths: &PvPaths,
    catalog: &ManagedResourceRuntimeCatalog,
    demanded_tracks: &BTreeSet<DemandedResourceTrack>,
    progress: DaemonDownloadProgress,
) -> Result<(), DaemonError> {
    let installs = {
        let database = Database::open(paths)?;
        missing_desired_resource_installs_with_demands(&database, catalog, demanded_tracks)?
    };

    install_missing_desired_resource_tracks(
        paths,
        catalog.install_options.clone(),
        catalog.http_client.clone(),
        installs,
        progress,
    )
    .await
}

pub(crate) async fn stop_undemanded_system_resource_runtimes(
    paths: &PvPaths,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
) -> Result<(), DaemonError> {
    // Post-apply usage supersedes conservative discovery demand while failed Projects retain their
    // last-valid usage protection.
    let production_catalog;
    let catalog = if let Some(catalog) = runtime_catalog {
        catalog
    } else {
        production_catalog = ManagedResourceRuntimeCatalog::production()?;
        &production_catalog
    };
    let mut database = Database::open(paths)?;
    let supervisor = ProcessSupervisor::new(paths.clone());

    stop_undemanded_catalog_runtimes(paths, &mut database, catalog, &supervisor, &BTreeSet::new())
        .await
}

async fn install_missing_desired_resource_tracks(
    paths: &PvPaths,
    install_options: ManagedResourceInstallOptions,
    http_client: Option<Arc<dyn resources::ResourceHttpClient + Send + Sync>>,
    installs: DesiredResourceInstallPlan,
    progress: DaemonDownloadProgress,
) -> Result<(), DaemonError> {
    if installs.is_empty() {
        return Ok(());
    }

    let install_paths = paths.clone();

    tokio::task::spawn_blocking(move || {
        install_missing_desired_resource_tracks_blocking(
            install_paths,
            install_options,
            http_client,
            installs,
            progress,
        )
    })
    .await?
}

#[derive(Debug)]
enum DesiredResourceInstall {
    PhpPair {
        track: String,
    },
    Composer,
    Runtime {
        adapter: resources::RuntimeArtifactAdapter,
        resource_name: String,
        track: String,
    },
}

#[derive(Debug, Default)]
struct DesiredResourceInstallPlan {
    installs: Vec<DesiredResourceInstall>,
    failures: Vec<DesiredResourceInstallFailure>,
}

impl DesiredResourceInstallPlan {
    fn is_empty(&self) -> bool {
        self.installs.is_empty() && self.failures.is_empty()
    }
}

#[derive(Debug)]
struct DesiredResourceInstallFailure {
    order: usize,
    label: String,
    error: DaemonError,
}

impl DesiredResourceInstallFailure {
    fn new(order: usize, label: String, error: DaemonError) -> Self {
        Self {
            order,
            label,
            error,
        }
    }

    fn message(&self) -> String {
        format!("{}: {}", self.label, self.error)
    }
}

impl DesiredResourceInstall {
    fn label(&self) -> String {
        match self {
            DesiredResourceInstall::PhpPair { track } => {
                format!("php/frankenphp {track}")
            }
            DesiredResourceInstall::Composer => "composer 2".to_string(),
            DesiredResourceInstall::Runtime {
                resource_name,
                track,
                ..
            } => {
                format!("{resource_name} {track}")
            }
        }
    }
}

#[cfg(test)]
fn missing_desired_resource_installs(
    database: &Database,
    catalog: &ManagedResourceRuntimeCatalog,
) -> Result<DesiredResourceInstallPlan, DaemonError> {
    missing_desired_resource_installs_with_demands(database, catalog, &BTreeSet::new())
}

fn missing_desired_resource_installs_with_demands(
    database: &Database,
    catalog: &ManagedResourceRuntimeCatalog,
    demanded_tracks: &BTreeSet<DemandedResourceTrack>,
) -> Result<DesiredResourceInstallPlan, DaemonError> {
    let mut php_pair_tracks = BTreeSet::new();
    let mut caddy_tracks = BTreeSet::new();
    let mut composer_missing = false;
    let mut runtime_installs = Vec::new();
    let mut failures = Vec::new();

    let records = database.managed_resource_tracks()?;
    let installed_tracks = records
        .iter()
        .filter(|record| {
            record.desired_state == ManagedResourceDesiredState::Installed
                && record.current_artifact_path.is_some()
        })
        .map(|record| {
            DemandedResourceTrack::new(record.resource_name.clone(), record.track.clone())
        })
        .collect::<BTreeSet<_>>();
    let mut missing_tracks = records
        .into_iter()
        .filter(|record| {
            record.desired_state == ManagedResourceDesiredState::Installed
                && record.current_artifact_path.is_none()
        })
        .map(|record| DemandedResourceTrack::new(record.resource_name, record.track))
        .collect::<BTreeSet<_>>();
    missing_tracks.extend(demanded_tracks.difference(&installed_tracks).cloned());

    for demanded_track in missing_tracks {
        let DemandedResourceTrack {
            resource_name,
            track,
        } = demanded_track;
        match resource_name.as_str() {
            "caddy" => {
                caddy_tracks.insert(track);
            }
            "php" | "frankenphp" => {
                php_pair_tracks.insert(track);
            }
            "composer" => {
                if track != "2" {
                    let error = DaemonError::UnexpectedProtocolResponse {
                        reason: format!(
                            "Composer setup default expected track `2`, got `{}`",
                            track
                        ),
                    };
                    failures.push(DesiredResourceInstallFailure::new(
                        failures.len(),
                        format!("composer {track}"),
                        error,
                    ));
                    continue;
                }
                composer_missing = true;
            }
            _ => {
                let Some(adapter) = catalog.adapter(&resource_name) else {
                    let error = DaemonError::UnsupportedManagedResourceRuntime {
                        resource: resource_name.clone(),
                    };
                    failures.push(DesiredResourceInstallFailure::new(
                        failures.len(),
                        format!("{resource_name} {track}"),
                        error,
                    ));
                    continue;
                };
                let artifact_adapter = match adapter.artifact_adapter() {
                    Ok(adapter) => adapter,
                    Err(error) => {
                        failures.push(DesiredResourceInstallFailure::new(
                            failures.len(),
                            format!("{resource_name} {track}"),
                            error,
                        ));
                        continue;
                    }
                };
                runtime_installs.push(DesiredResourceInstall::Runtime {
                    adapter: artifact_adapter,
                    resource_name,
                    track,
                });
            }
        }
    }

    let mut installs = Vec::new();
    if !caddy_tracks.is_empty() {
        let caddy_adapter = resources::caddy_adapter()?;
        installs.extend(
            caddy_tracks
                .into_iter()
                .map(|track| DesiredResourceInstall::Runtime {
                    adapter: caddy_adapter.clone(),
                    resource_name: "caddy".to_owned(),
                    track,
                }),
        );
    }
    installs.extend(
        php_pair_tracks
            .into_iter()
            .map(|track| DesiredResourceInstall::PhpPair { track })
            .collect::<Vec<_>>(),
    );
    if composer_missing {
        installs.push(DesiredResourceInstall::Composer);
    }
    installs.extend(runtime_installs);

    Ok(DesiredResourceInstallPlan { installs, failures })
}

fn install_missing_desired_resource_tracks_blocking(
    paths: PvPaths,
    install_options: ManagedResourceInstallOptions,
    http_client: Option<Arc<dyn resources::ResourceHttpClient + Send + Sync>>,
    installs: DesiredResourceInstallPlan,
    progress: DaemonDownloadProgress,
) -> Result<(), DaemonError> {
    let downloads_dir = paths.downloads().to_path_buf();
    let commands = ManagedResourceCommands::new(
        paths,
        install_options.manifest_url,
        install_options.target_platform,
    );
    let default_client = resources::UreqResourceHttpClient::default();
    let client: &(dyn resources::ResourceHttpClient + Send + Sync) =
        http_client.as_deref().unwrap_or(&default_client);
    let DesiredResourceInstallPlan {
        installs,
        mut failures,
    } = installs;
    if installs.is_empty() {
        return finish_desired_resource_install_failures(failures);
    }
    let manifest_snapshot = match progress.manifest_snapshot(&commands, client) {
        Ok(snapshot) => snapshot,
        Err(error) if failures.is_empty() => return Err(error),
        Err(error) => {
            failures.push(DesiredResourceInstallFailure::new(
                failures.len(),
                "artifact manifest".to_owned(),
                error,
            ));

            return finish_desired_resource_install_failures(failures);
        }
    };
    let mut resolved_installs = Vec::new();
    let first_install_order = failures.len();
    for (index, install) in installs.into_iter().enumerate() {
        let order = first_install_order + index;
        let label = install.label();
        match resolve_desired_resource_install(&commands, &manifest_snapshot, install) {
            Ok(install) => resolved_installs.push(ResolvedDesiredResourceInstall {
                order,
                label,
                install,
            }),
            Err(error) => failures.push(DesiredResourceInstallFailure::new(order, label, error)),
        }
    }
    let artifacts = unique_resolved_artifacts(&resolved_installs);
    let downloads = prefetch_artifacts(&downloads_dir, client, artifacts, &progress);

    for resolved in resolved_installs {
        let download_failures =
            resolved_download_failures(&downloads, &resolved.install, resolved.label.as_str());
        if !download_failures.is_empty() {
            failures.extend(download_failures.into_iter().map(|(label, error)| {
                DesiredResourceInstallFailure::new(resolved.order, label, error)
            }));
            continue;
        }
        if let Err(error) =
            install_resolved_desired_resource(&commands, &downloads, &progress, &resolved.install)
        {
            failures.push(DesiredResourceInstallFailure::new(
                resolved.order,
                resolved.label,
                error,
            ));
        }
    }
    finish_desired_resource_install_failures(failures)
}

fn finish_desired_resource_install_failures(
    mut failures: Vec<DesiredResourceInstallFailure>,
) -> Result<(), DaemonError> {
    failures.sort_by_key(|failure| failure.order);
    match failures.len() {
        0 => Ok(()),
        1 => Err(failures.remove(0).error),
        _ => Err(DaemonError::ManagedResourceDefaultInstallFailures {
            failures: failures
                .into_iter()
                .map(|failure| failure.message())
                .collect(),
        }),
    }
}

#[derive(Debug)]
struct ResolvedDesiredResourceInstall {
    order: usize,
    label: String,
    install: ResolvedDesiredResourceInstallKind,
}

#[derive(Debug)]
enum ResolvedDesiredResourceInstallKind {
    PhpPair {
        php: resources::ManagedResourceInstallArtifact,
        frankenphp: Box<resources::ManagedResourceInstallArtifact>,
    },
    Composer {
        adapter: resources::RuntimeArtifactAdapter,
        artifact: resources::ManagedResourceInstallArtifact,
    },
    Runtime {
        adapter: resources::RuntimeArtifactAdapter,
        artifact: resources::ManagedResourceInstallArtifact,
    },
}

type ArtifactDownloadKey = (String, String, String);

fn resolve_desired_resource_install(
    commands: &ManagedResourceCommands,
    manifest: &resources::ArtifactManifestRefresh,
    install: DesiredResourceInstall,
) -> Result<ResolvedDesiredResourceInstallKind, DaemonError> {
    match install {
        DesiredResourceInstall::PhpPair { track } => {
            let track = TrackName::new(track)?;
            let php_adapter = resources::php_adapter()?;
            let frankenphp_adapter = resources::frankenphp_adapter()?;
            let php = commands.resolve_install_artifact(&php_adapter, track.clone(), manifest)?;
            let frankenphp =
                commands.resolve_install_artifact(&frankenphp_adapter, track, manifest)?;

            Ok(ResolvedDesiredResourceInstallKind::PhpPair {
                php,
                frankenphp: Box::new(frankenphp),
            })
        }
        DesiredResourceInstall::Composer => {
            let adapter = resources::composer_adapter()?;
            let artifact =
                commands.resolve_install_artifact(&adapter, TrackName::new("2")?, manifest)?;

            Ok(ResolvedDesiredResourceInstallKind::Composer { adapter, artifact })
        }
        DesiredResourceInstall::Runtime {
            adapter,
            resource_name,
            track,
        } => {
            if adapter.resource_name().as_str() != resource_name {
                return Err(DaemonError::UnexpectedProtocolResponse {
                    reason: format!(
                        "runtime adapter resolved `{}` while reconciling `{resource_name}`",
                        adapter.resource_name()
                    ),
                });
            }
            let artifact =
                commands.resolve_install_artifact(&adapter, TrackName::new(track)?, manifest)?;

            Ok(ResolvedDesiredResourceInstallKind::Runtime { adapter, artifact })
        }
    }
}

fn unique_resolved_artifacts(
    installs: &[ResolvedDesiredResourceInstall],
) -> BTreeMap<ArtifactDownloadKey, resources::ManifestArtifact> {
    let mut artifacts = BTreeMap::new();
    for resolved in installs {
        match &resolved.install {
            ResolvedDesiredResourceInstallKind::PhpPair { php, frankenphp } => {
                insert_required_artifact(&mut artifacts, php);
                insert_required_artifact(&mut artifacts, frankenphp);
            }
            ResolvedDesiredResourceInstallKind::Composer { artifact, .. }
            | ResolvedDesiredResourceInstallKind::Runtime { artifact, .. } => {
                insert_required_artifact(&mut artifacts, artifact);
            }
        }
    }

    artifacts
}

fn insert_required_artifact(
    artifacts: &mut BTreeMap<ArtifactDownloadKey, resources::ManifestArtifact>,
    resolved: &resources::ManagedResourceInstallArtifact,
) {
    if !resolved.download_required() {
        return;
    }
    let artifact = resolved.artifact();
    artifacts
        .entry(artifact_download_key(artifact))
        .or_insert_with(|| artifact.clone());
}

fn artifact_download_key(artifact: &resources::ManifestArtifact) -> ArtifactDownloadKey {
    (
        artifact.resource_name().as_str().to_owned(),
        artifact.artifact_version().as_str().to_owned(),
        artifact.sha256().as_str().to_owned(),
    )
}

fn prefetch_artifacts(
    downloads_dir: &Utf8Path,
    client: &(impl resources::ResourceHttpClient + Sync + ?Sized),
    artifacts: BTreeMap<ArtifactDownloadKey, resources::ManifestArtifact>,
    progress: &(impl resources::DownloadProgress + Sync),
) -> BTreeMap<ArtifactDownloadKey, resources::Result<resources::ArtifactDownload>> {
    let (keys, artifacts): (Vec<_>, Vec<_>) = artifacts.into_iter().unzip();
    let downloads = resources::ArtifactDownloader::new(downloads_dir)
        .download_many_with_progress(&artifacts, client, progress);

    keys.into_iter().zip(downloads).collect()
}

fn install_resolved_desired_resource(
    commands: &ManagedResourceCommands,
    downloads: &BTreeMap<ArtifactDownloadKey, resources::Result<resources::ArtifactDownload>>,
    progress: &DaemonDownloadProgress,
    install: &ResolvedDesiredResourceInstallKind,
) -> Result<(), DaemonError> {
    match install {
        ResolvedDesiredResourceInstallKind::PhpPair { php, frankenphp } => {
            let php_download = prefetched_download(downloads, php)?;
            let frankenphp_download = prefetched_download(downloads, frankenphp)?;
            commands.install_resolved_php_pair_with_progress(
                php.clone(),
                php_download,
                frankenphp.as_ref().clone(),
                frankenphp_download,
                progress,
            )?;
        }
        ResolvedDesiredResourceInstallKind::Composer { adapter, artifact } => {
            let download = prefetched_download(downloads, artifact)?;
            commands.install_resolved_artifact_with_progress(
                adapter,
                artifact.clone(),
                download,
                progress,
            )?;
        }
        ResolvedDesiredResourceInstallKind::Runtime { adapter, artifact } => {
            let download = prefetched_download(downloads, artifact)?;
            commands.install_resolved_artifact_with_progress(
                adapter,
                artifact.clone(),
                download,
                progress,
            )?;
        }
    }

    Ok(())
}

fn prefetched_download<'downloads>(
    downloads: &'downloads BTreeMap<
        ArtifactDownloadKey,
        resources::Result<resources::ArtifactDownload>,
    >,
    resolved: &resources::ManagedResourceInstallArtifact,
) -> Result<Option<&'downloads resources::ArtifactDownload>, DaemonError> {
    if !resolved.download_required() {
        return Ok(None);
    }
    let artifact = resolved.artifact();
    let result = downloads
        .get(&artifact_download_key(artifact))
        .ok_or_else(|| DaemonError::UnexpectedProtocolResponse {
            reason: format!(
                "missing prefetched artifact for {} {}",
                artifact.resource_name(),
                artifact.artifact_version()
            ),
        })?;

    result
        .as_ref()
        .map(Some)
        .map_err(|error| resources::ManagedResourceCommandError::from(error.clone()).into())
}

fn resolved_download_failures(
    downloads: &BTreeMap<ArtifactDownloadKey, resources::Result<resources::ArtifactDownload>>,
    install: &ResolvedDesiredResourceInstallKind,
    label: &str,
) -> Vec<(String, DaemonError)> {
    let mut failures = Vec::new();
    match install {
        ResolvedDesiredResourceInstallKind::PhpPair { php, frankenphp } => {
            for (resource_name, resolved) in [("php", php), ("frankenphp", frankenphp.as_ref())] {
                if let Err(error) = prefetched_download(downloads, resolved) {
                    failures.push((
                        format!("{resource_name} {}", resolved.artifact().track()),
                        error,
                    ));
                }
            }
        }
        ResolvedDesiredResourceInstallKind::Composer { artifact, .. }
        | ResolvedDesiredResourceInstallKind::Runtime { artifact, .. } => {
            if let Err(error) = prefetched_download(downloads, artifact) {
                failures.push((label.to_owned(), error));
            }
        }
    }

    failures
}

struct ResourceTrackReconciliationContext<'context> {
    catalog: &'context ManagedResourceRuntimeCatalog,
    supervisor: &'context ProcessSupervisor,
    progress: &'context DaemonDownloadProgress,
    prefetched_installs: &'context mut BTreeMap<ProjectTrackKey, PrefetchedProjectInstall>,
}

type ProjectTrackKey = (String, String);

enum PrefetchedProjectInstall {
    Ready {
        adapter: resources::RuntimeArtifactAdapter,
        resolved: Box<resources::ManagedResourceInstallArtifact>,
        download: Option<resources::ArtifactDownload>,
    },
    Failed(DaemonError),
}

struct ResolvedProjectInstall {
    key: ProjectTrackKey,
    adapter: resources::RuntimeArtifactAdapter,
    resolved: resources::ManagedResourceInstallArtifact,
}

enum ProjectInstallRequest {
    Resolve {
        key: ProjectTrackKey,
        adapter: resources::RuntimeArtifactAdapter,
    },
    Failed {
        key: ProjectTrackKey,
        error: DaemonError,
    },
}

async fn reconcile_resource_track(
    paths: &PvPaths,
    database: &mut Database,
    project: &ProjectRecord,
    plan: &crate::project_env::ProjectResourcePlan,
    reconciliation: &mut ResourceTrackReconciliationContext<'_>,
    resource: &state::ProjectManagedResourceInput,
) -> Result<(), DaemonError> {
    let subject = RuntimeSubject::Resource {
        name: resource.resource_name.clone(),
        track: resource.track.clone(),
    };
    let Some(adapter) = reconciliation.catalog.adapter(&resource.resource_name) else {
        if unsupported_resource_has_seeded_env_context(database, resource)? {
            return Ok(());
        }

        let error = DaemonError::UnsupportedManagedResourceRuntime {
            resource: resource.resource_name.clone(),
        };
        database.record_runtime_observed_snapshot(
            subject,
            RuntimeObservedStatus::Failed,
            Some(&error.to_string()),
        )?;

        return Err(error);
    };
    let result = async {
        let track_record = ensure_track_artifact(
            paths,
            database,
            resource,
            reconciliation.progress,
            reconciliation.prefetched_installs,
        )
        .await?;
        let Some(artifact_path) = track_record.current_artifact_path else {
            return Err(DaemonError::ManagedResourceArtifactMissing {
                resource: resource.resource_name.clone(),
                track: resource.track.clone(),
            });
        };
        let mut attempt = 0;

        loop {
            attempt += 1;
            let ports =
                assign_named_ports(database, adapter, &resource.resource_name, &resource.track)?;
            if ports_occupied_without_recorded_runtime(
                paths,
                reconciliation.supervisor,
                resource,
                &ports,
            )? && attempt < RESOURCE_START_ATTEMPTS
            {
                cleanup_resource_runtime_files(paths, resource)?;
                release_resource_track_ports(database, &resource.resource_name, &resource.track)?;

                continue;
            }
            let context = ManagedResourceRuntimeContext {
                resource_name: resource.resource_name.clone(),
                track: resource.track.clone(),
                artifact_path: artifact_path.clone(),
                data_dir: paths.resource_data_dir(&resource.resource_name, &resource.track),
                ports,
                env: track_record.env.clone(),
                postgres_preload_libraries: if resource.resource_name == "postgres" {
                    database.postgres_track_preload_libraries(&resource.track)?
                } else {
                    Vec::new()
                },
            };
            let mut runtime_attempt = ResourceRuntimeAttempt {
                paths,
                database,
                project,
                plan,
                adapter,
                supervisor: reconciliation.supervisor,
                resource,
                subject: &subject,
            };
            let result = runtime_attempt.run(&context).await;

            if matches!(
                result,
                Err(DaemonError::NonPvManagedResourceRuntimeListener { .. })
            ) && attempt < RESOURCE_START_ATTEMPTS
            {
                cleanup_resource_runtime_files(paths, resource)?;
                release_resource_track_ports(database, &resource.resource_name, &resource.track)?;

                continue;
            }

            break result;
        }
    }
    .await;

    if let Err(error) = &result {
        database.record_runtime_observed_snapshot(
            subject,
            RuntimeObservedStatus::Failed,
            Some(&error.to_string()),
        )?;
    }

    result
}

struct ResourceRuntimeAttempt<'a> {
    paths: &'a PvPaths,
    database: &'a mut Database,
    project: &'a ProjectRecord,
    plan: &'a crate::project_env::ProjectResourcePlan,
    adapter: &'a dyn ManagedResourceRuntimeAdapter,
    supervisor: &'a ProcessSupervisor,
    resource: &'a state::ProjectManagedResourceInput,
    subject: &'a RuntimeSubject,
}

impl ResourceRuntimeAttempt<'_> {
    async fn run(&mut self, context: &ManagedResourceRuntimeContext) -> Result<(), DaemonError> {
        let env = self.adapter.resource_env(context)?;
        let context = ManagedResourceRuntimeContext {
            env: env.clone(),
            ..context.clone()
        };
        self.database.record_managed_resource_track_env_context(
            &self.resource.resource_name,
            &self.resource.track,
            &env,
        )?;
        let spec = self.adapter.build_process_spec(self.paths, &context)?;
        self.adapter.prepare_runtime(self.paths, &context).await?;
        let readiness = self.adapter.readiness(&context)?;
        let readiness_timeout = adapter_readiness_timeout(self.adapter);

        start_or_adopt_runtime(self.supervisor, spec, &readiness, readiness_timeout).await?;

        let allocations =
            desired_allocations(self.database, self.project, self.plan, self.resource)?;
        self.adapter
            .reconcile_allocations(self.paths, self.database, &context, &env, &allocations)
            .await?;
        self.database.record_runtime_observed_snapshot(
            self.subject.clone(),
            RuntimeObservedStatus::Running,
            Some("Managed Resource runtime is ready"),
        )?;

        Ok(())
    }
}

fn unsupported_resource_has_seeded_env_context(
    database: &Database,
    resource: &state::ProjectManagedResourceInput,
) -> Result<bool, DaemonError> {
    let has_context = database
        .managed_resource_tracks()?
        .into_iter()
        .any(|track| {
            track.resource_name == resource.resource_name
                && track.track == resource.track
                && !track.env.is_empty()
        });

    Ok(has_context)
}

fn missing_project_install_requests(
    database: &Database,
    plan: &crate::project_env::ProjectResourcePlan,
    catalog: &ManagedResourceRuntimeCatalog,
) -> Vec<ProjectInstallRequest> {
    let mut requests = Vec::new();
    for resource in &plan.resources {
        let key = (resource.resource_name.clone(), resource.track.clone());
        match installed_track(database, &resource.resource_name, &resource.track) {
            Ok(Some(_installed)) => continue,
            Ok(None) => {}
            Err(error) => {
                requests.push(ProjectInstallRequest::Failed { key, error });
                break;
            }
        }
        let Some(adapter) = catalog.adapter(&resource.resource_name) else {
            match unsupported_resource_has_seeded_env_context(database, resource) {
                Ok(true) => continue,
                Ok(false) => break,
                Err(error) => {
                    requests.push(ProjectInstallRequest::Failed { key, error });
                    break;
                }
            }
        };
        match adapter.artifact_adapter() {
            Ok(adapter) => requests.push(ProjectInstallRequest::Resolve { key, adapter }),
            Err(error) => {
                requests.push(ProjectInstallRequest::Failed { key, error });
                break;
            }
        }
    }

    requests
}

fn take_project_prefetch_failures(
    database: &mut Database,
    resources: &[state::ProjectManagedResourceInput],
    prefetched_installs: &mut BTreeMap<ProjectTrackKey, PrefetchedProjectInstall>,
) -> Result<Vec<ManagedResourceProjectFailure>, DaemonError> {
    let mut failures = Vec::new();
    for resource in resources {
        let key = (resource.resource_name.clone(), resource.track.clone());
        if !matches!(
            prefetched_installs.get(&key),
            Some(PrefetchedProjectInstall::Failed(_))
        ) {
            continue;
        }
        let Some(PrefetchedProjectInstall::Failed(error)) = prefetched_installs.remove(&key) else {
            continue;
        };
        database.record_runtime_observed_snapshot(
            RuntimeSubject::Resource {
                name: resource.resource_name.clone(),
                track: resource.track.clone(),
            },
            RuntimeObservedStatus::Failed,
            Some(&error.to_string()),
        )?;
        failures.push(ManagedResourceProjectFailure::new(
            resource.resource_name.clone(),
            resource.track.clone(),
            error,
        ));
    }

    Ok(failures)
}

fn combined_project_resource_error(
    mut failures: Vec<ManagedResourceProjectFailure>,
) -> DaemonError {
    if failures.len() == 1 {
        return failures.remove(0).into_error();
    }

    DaemonError::ManagedResourceProjectFailures { failures }
}

async fn prefetch_missing_project_installs(
    paths: &PvPaths,
    catalog: &ManagedResourceRuntimeCatalog,
    requests: Vec<ProjectInstallRequest>,
    progress: DaemonDownloadProgress,
) -> Result<BTreeMap<ProjectTrackKey, PrefetchedProjectInstall>, DaemonError> {
    if requests.is_empty() {
        return Ok(BTreeMap::new());
    }

    let install_paths = paths.clone();
    let install_options = catalog.install_options.clone();
    let http_client = catalog.http_client.clone();
    tokio::task::spawn_blocking(move || {
        prefetch_missing_project_installs_blocking(
            install_paths,
            install_options,
            http_client,
            requests,
            progress,
        )
    })
    .await?
}

fn prefetch_missing_project_installs_blocking(
    paths: PvPaths,
    install_options: ManagedResourceInstallOptions,
    http_client: Option<Arc<dyn resources::ResourceHttpClient + Send + Sync>>,
    requests: Vec<ProjectInstallRequest>,
    progress: DaemonDownloadProgress,
) -> Result<BTreeMap<ProjectTrackKey, PrefetchedProjectInstall>, DaemonError> {
    let downloads_dir = paths.downloads().to_path_buf();
    let commands = ManagedResourceCommands::new(
        paths,
        install_options.manifest_url,
        install_options.target_platform,
    );
    let default_client = resources::UreqResourceHttpClient::default();
    let client: &(dyn resources::ResourceHttpClient + Send + Sync) =
        http_client.as_deref().unwrap_or(&default_client);
    let mut prefetched = BTreeMap::new();
    let mut resolve_requests = Vec::new();
    for request in requests {
        match request {
            ProjectInstallRequest::Resolve { key, adapter } => {
                resolve_requests.push((key, adapter));
            }
            ProjectInstallRequest::Failed { key, error } => {
                prefetched.insert(key, PrefetchedProjectInstall::Failed(error));
            }
        }
    }
    if resolve_requests.is_empty() {
        return Ok(prefetched);
    }
    let manifest_snapshot = match progress.manifest_snapshot(&commands, client) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            if let Some((key, _adapter)) = resolve_requests.into_iter().next() {
                prefetched.insert(key, PrefetchedProjectInstall::Failed(error));
            }

            return Ok(prefetched);
        }
    };
    let mut resolved_installs = Vec::new();
    for (key, adapter) in resolve_requests {
        let (resource_name, track) = (&key.0, &key.1);
        if adapter.resource_name().as_str() != resource_name {
            let reason = format!(
                "runtime adapter resolved `{}` while reconciling `{resource_name}`",
                adapter.resource_name()
            );
            prefetched.insert(
                key,
                PrefetchedProjectInstall::Failed(DaemonError::UnexpectedProtocolResponse {
                    reason,
                }),
            );
            continue;
        }
        let resolved = match TrackName::new(track)
            .map_err(DaemonError::from)
            .and_then(|track| {
                commands
                    .resolve_install_artifact(&adapter, track, &manifest_snapshot)
                    .map_err(Into::into)
            }) {
            Ok(resolved) => resolved,
            Err(error) => {
                prefetched.insert(key, PrefetchedProjectInstall::Failed(error));
                continue;
            }
        };
        resolved_installs.push(ResolvedProjectInstall {
            key,
            adapter,
            resolved,
        });
    }

    let mut artifacts = BTreeMap::new();
    for install in &resolved_installs {
        insert_required_artifact(&mut artifacts, &install.resolved);
    }
    let downloads = prefetch_artifacts(&downloads_dir, client, artifacts, &progress);
    for install in resolved_installs {
        let download = if install.resolved.download_required() {
            downloads
                .get(&artifact_download_key(install.resolved.artifact()))
                .cloned()
                .unwrap_or_else(|| {
                    Err(resources::ResourcesError::MissingArtifactDownload {
                        resource: install
                            .resolved
                            .artifact()
                            .resource_name()
                            .as_str()
                            .to_string(),
                        artifact_version: install
                            .resolved
                            .artifact()
                            .artifact_version()
                            .as_str()
                            .to_string(),
                    })
                })
                .map(Some)
        } else {
            Ok(None)
        };
        let prefetched_install = match download {
            Ok(download) => PrefetchedProjectInstall::Ready {
                adapter: install.adapter,
                resolved: Box::new(install.resolved),
                download,
            },
            Err(error) => PrefetchedProjectInstall::Failed(
                resources::ManagedResourceCommandError::from(error).into(),
            ),
        };
        prefetched.insert(install.key, prefetched_install);
    }

    Ok(prefetched)
}

async fn ensure_track_artifact(
    paths: &PvPaths,
    database: &mut Database,
    resource: &state::ProjectManagedResourceInput,
    progress: &DaemonDownloadProgress,
    prefetched_installs: &mut BTreeMap<ProjectTrackKey, PrefetchedProjectInstall>,
) -> Result<ManagedResourceTrackRecord, DaemonError> {
    if let Some(record) = installed_track(database, &resource.resource_name, &resource.track)? {
        return Ok(record);
    }

    let install_paths = paths.clone();
    let progress = progress.clone();
    let key = (resource.resource_name.clone(), resource.track.clone());
    let prefetched = prefetched_installs.remove(&key).ok_or_else(|| {
        DaemonError::UnexpectedProtocolResponse {
            reason: format!(
                "missing prefetched install for {} track {}",
                resource.resource_name, resource.track
            ),
        }
    })?;
    let (artifact_adapter, resolved, download) = match prefetched {
        PrefetchedProjectInstall::Ready {
            adapter,
            resolved,
            download,
        } => (adapter, *resolved, download),
        PrefetchedProjectInstall::Failed(error) => return Err(error),
    };

    tokio::task::spawn_blocking(move || {
        install_prefetched_project_track_blocking(
            install_paths,
            artifact_adapter,
            resolved,
            download,
            progress,
        )
    })
    .await??;

    installed_track(database, &resource.resource_name, &resource.track)?.ok_or_else(|| {
        DaemonError::ManagedResourceArtifactMissing {
            resource: resource.resource_name.clone(),
            track: resource.track.clone(),
        }
    })
}

fn installed_track(
    database: &Database,
    resource_name: &str,
    track: &str,
) -> Result<Option<ManagedResourceTrackRecord>, DaemonError> {
    let Some(record) = database
        .managed_resource_tracks()?
        .into_iter()
        .find(|record| record.resource_name == resource_name && record.track == track)
    else {
        return Ok(None);
    };

    if record.current_artifact_path.is_none() {
        return Ok(None);
    }
    if record.desired_state == ManagedResourceDesiredState::Removed {
        return Err(DaemonError::ManagedResourceTrackRemoved {
            resource: resource_name.to_string(),
            track: track.to_string(),
        });
    }

    Ok(Some(record))
}

fn install_prefetched_project_track_blocking(
    paths: PvPaths,
    adapter: resources::RuntimeArtifactAdapter,
    resolved: resources::ManagedResourceInstallArtifact,
    download: Option<resources::ArtifactDownload>,
    progress: DaemonDownloadProgress,
) -> Result<(), DaemonError> {
    let commands = ManagedResourceCommands::new(
        paths,
        resources::default_artifact_manifest_url(),
        resolved.target_platform(),
    );
    commands.install_resolved_artifact_with_progress(
        &adapter,
        resolved,
        download.as_ref(),
        &progress,
    )?;

    Ok(())
}

fn assign_named_ports(
    database: &mut Database,
    adapter: &dyn ManagedResourceRuntimeAdapter,
    resource_name: &str,
    track: &str,
) -> Result<BTreeMap<String, u16>, DaemonError> {
    let result = assign_named_ports_inner(database, adapter, resource_name, track);

    if result.is_err() {
        release_resource_track_ports(database, resource_name, track)?;
    }

    result
}

fn assign_named_ports_inner(
    database: &mut Database,
    adapter: &dyn ManagedResourceRuntimeAdapter,
    resource_name: &str,
    track: &str,
) -> Result<BTreeMap<String, u16>, DaemonError> {
    let mut assignments = BTreeMap::new();

    for port_spec in adapter.port_specs() {
        if port_spec.name == RESERVED_RESOURCE_PORT_NAME {
            return Err(DaemonError::ManagedResourcePortNameReserved {
                resource: resource_name.to_string(),
                track: track.to_string(),
                port: port_spec.name.to_string(),
            });
        }

        let assignment = database.assign_port(
            PortRequest::resource_port(
                resource_name,
                track,
                port_spec.name,
                port_spec.preferred_port,
                RUNTIME_PORT_FALLBACK_START,
                RUNTIME_PORT_FALLBACK_END,
            ),
            local_loopback_port_available,
        )?;

        assignments.insert(port_spec.name.to_string(), assignment.port);
    }

    Ok(assignments)
}

fn ports_occupied_without_recorded_runtime(
    paths: &PvPaths,
    supervisor: &ProcessSupervisor,
    resource: &state::ProjectManagedResourceInput,
    ports: &BTreeMap<String, u16>,
) -> Result<bool, DaemonError> {
    if ports
        .values()
        .all(|port| local_loopback_port_available(*port))
    {
        return Ok(false);
    }
    let recorded_runtime = supervisor.adopt_recorded(
        &paths.resource_pid(&resource.resource_name, &resource.track),
        &paths.resource_runtime_metadata(&resource.resource_name, &resource.track),
    )?;

    Ok(recorded_runtime.is_none())
}

async fn start_or_adopt_runtime(
    supervisor: &ProcessSupervisor,
    spec: ProcessSpec,
    readiness: &ManagedResourceReadiness,
    readiness_timeout: Duration,
) -> Result<(), DaemonError> {
    if supervisor.adopt(&spec)?.is_some() {
        wait_for_managed_resource_readiness(readiness, readiness_timeout).await?;

        return Ok(());
    }
    if let Some(adopted) = supervisor.adopt_recorded(&spec.pid_path, &spec.metadata_path)? {
        adopted.stop(RESOURCE_STOP_GRACE_PERIOD).await?;
        delete_optional_file(&spec.pid_path)?;
        delete_optional_file(&spec.metadata_path)?;
    } else if let ManagedResourceReadiness::TcpHttp(check) = readiness
        && crate::supervisor::probe_readiness_once(check).await.is_ok()
    {
        return Err(DaemonError::NonPvManagedResourceRuntimeListener { name: spec.name });
    }

    let mut process = supervisor.start(spec.clone()).await?;
    if let Err(error) =
        wait_for_started_runtime_readiness(&mut process, &spec.name, readiness, readiness_timeout)
            .await
    {
        process.stop(RESOURCE_STOP_GRACE_PERIOD).await?;
        cleanup_started_runtime_files(&spec)?;

        return Err(error);
    }
    tokio::time::sleep(RESOURCE_PROCESS_EXIT_POLL_INTERVAL).await;
    if process.has_exited()? {
        cleanup_started_runtime_files(&spec)?;

        return Err(runtime_exited_before_readiness_error(&spec.name));
    }

    Ok(())
}

async fn wait_for_started_runtime_readiness(
    process: &mut crate::supervisor::ManagedProcess,
    runtime_name: &str,
    readiness: &ManagedResourceReadiness,
    readiness_timeout: Duration,
) -> Result<(), DaemonError> {
    let readiness_wait = wait_for_managed_resource_readiness(readiness, readiness_timeout);
    tokio::pin!(readiness_wait);

    loop {
        tokio::select! {
            result = &mut readiness_wait => {
                if result.is_err() && process.has_exited()? {
                    return Err(runtime_exited_before_readiness_error(runtime_name));
                }

                return result;
            }
            () = sleep(RESOURCE_PROCESS_EXIT_POLL_INTERVAL) => {
                if process.has_exited()? {
                    return Err(runtime_exited_before_readiness_error(runtime_name));
                }
            }
        }
    }
}

fn runtime_exited_before_readiness_error(runtime_name: &str) -> DaemonError {
    DaemonError::UnexpectedProtocolResponse {
        reason: format!("runtime `{runtime_name}` exited before readiness was verified"),
    }
}

async fn wait_for_managed_resource_readiness(
    readiness: &ManagedResourceReadiness,
    readiness_timeout: Duration,
) -> Result<(), DaemonError> {
    match readiness {
        ManagedResourceReadiness::TcpHttp(check) => {
            wait_for_readiness(check.clone(), readiness_timeout).await
        }
        ManagedResourceReadiness::Async(check) => {
            wait_for_async_readiness(check, readiness_timeout).await
        }
    }
}

async fn wait_for_async_readiness(
    readiness: &AsyncManagedResourceReadiness,
    readiness_timeout: Duration,
) -> Result<(), DaemonError> {
    let started_at = Instant::now();
    let mut last_error = None;

    while let Some(remaining) = remaining_timeout(started_at, readiness_timeout) {
        match timeout(remaining, (readiness.check)()).await {
            Ok(Ok(())) => return Ok(()),
            Ok(Err(error)) => {
                last_error = Some(error.to_string());
                sleep(remaining.min(ASYNC_READINESS_POLL_INTERVAL)).await;
            }
            Err(elapsed) => {
                last_error = Some(elapsed.to_string());
                break;
            }
        }
    }

    Err(DaemonError::ReadinessTimedOut {
        check: format!("async:{}", readiness.name),
        timeout_ms: readiness_timeout.as_millis(),
        last_error,
    })
}

fn remaining_timeout(started_at: Instant, readiness_timeout: Duration) -> Option<Duration> {
    readiness_timeout
        .checked_sub(started_at.elapsed())
        .filter(|remaining| !remaining.is_zero())
}

fn cleanup_started_runtime_files(spec: &ProcessSpec) -> Result<(), DaemonError> {
    delete_optional_file(&spec.pid_path)?;
    delete_optional_file(&spec.metadata_path)?;
    delete_optional_file(&spec.config_path)?;

    Ok(())
}

fn adapter_readiness_timeout(adapter: &dyn ManagedResourceRuntimeAdapter) -> Duration {
    #[cfg(test)]
    {
        adapter.readiness_timeout()
    }

    #[cfg(not(test))]
    {
        let _adapter = adapter;

        RESOURCE_READINESS_TIMEOUT
    }
}

fn desired_allocations(
    database: &Database,
    project: &ProjectRecord,
    plan: &crate::project_env::ProjectResourcePlan,
    resource: &state::ProjectManagedResourceInput,
) -> Result<Vec<ResourceAllocationRecord>, DaemonError> {
    let Some(allocation_plan) = plan.allocations.get(&resource.resource_name) else {
        return Ok(Vec::new());
    };
    let desired_names = allocation_plan
        .allocations
        .iter()
        .map(|allocation| allocation.allocation_name.as_str())
        .collect::<BTreeSet<_>>();
    let allocations = database
        .resource_allocations(&project.id, &resource.resource_name)?
        .into_iter()
        .filter(|allocation| {
            allocation.track == resource.track
                && desired_names.contains(allocation.allocation_name.as_str())
        })
        .collect();

    Ok(allocations)
}

async fn stop_undemanded_catalog_runtimes(
    paths: &PvPaths,
    database: &mut Database,
    catalog: &ManagedResourceRuntimeCatalog,
    supervisor: &ProcessSupervisor,
    demanded_tracks: &BTreeSet<DemandedResourceTrack>,
) -> Result<(), DaemonError> {
    let tracks = database.managed_resource_tracks()?;

    for track in tracks {
        let Some(_adapter) = catalog.adapter(&track.resource_name) else {
            continue;
        };
        if track.usage_count > 0
            || demanded_tracks.contains(&DemandedResourceTrack::new(
                track.resource_name.clone(),
                track.track.clone(),
            ))
        {
            continue;
        }

        stop_resource_runtime(paths, database, supervisor, &track).await?;
    }

    Ok(())
}

async fn stop_resource_runtime(
    paths: &PvPaths,
    database: &mut Database,
    supervisor: &ProcessSupervisor,
    track: &ManagedResourceTrackRecord,
) -> Result<(), DaemonError> {
    if let Some(adopted) = supervisor.adopt_recorded(
        &paths.resource_pid(&track.resource_name, &track.track),
        &paths.resource_runtime_metadata(&track.resource_name, &track.track),
    )? {
        adopted.stop(RESOURCE_STOP_GRACE_PERIOD).await?;
    }
    database.record_runtime_observed_snapshot(
        RuntimeSubject::Resource {
            name: track.resource_name.clone(),
            track: track.track.clone(),
        },
        RuntimeObservedStatus::Stopped,
        Some("Managed Resource runtime stopped; no Projects require this track"),
    )?;
    cleanup_resource_runtime(paths, database, track)?;

    Ok(())
}

fn cleanup_resource_runtime(
    paths: &PvPaths,
    database: &mut Database,
    track: &ManagedResourceTrackRecord,
) -> Result<(), DaemonError> {
    cleanup_resource_runtime_files_for_track(paths, &track.resource_name, &track.track)?;
    release_resource_track_ports(database, &track.resource_name, &track.track)?;

    Ok(())
}

fn cleanup_resource_runtime_files(
    paths: &PvPaths,
    resource: &state::ProjectManagedResourceInput,
) -> Result<(), DaemonError> {
    cleanup_resource_runtime_files_for_track(paths, &resource.resource_name, &resource.track)
}

fn cleanup_resource_runtime_files_for_track(
    paths: &PvPaths,
    resource_name: &str,
    track: &str,
) -> Result<(), DaemonError> {
    delete_optional_file(&paths.resource_pid(resource_name, track))?;
    delete_optional_file(&paths.resource_runtime_metadata(resource_name, track))?;
    delete_optional_file(&paths.resource_runtime_config(resource_name, track))?;

    Ok(())
}

fn release_resource_track_ports(
    database: &mut Database,
    resource_name: &str,
    track: &str,
) -> Result<(), DaemonError> {
    let port_owners = database
        .assigned_ports()?
        .into_iter()
        .filter_map(|assignment| match assignment.owner {
            PortOwner::Resource {
                name,
                track: owner_track,
                port,
            } if name == resource_name && owner_track == track => Some(PortOwner::Resource {
                name,
                track: owner_track,
                port,
            }),
            _ => None,
        })
        .collect::<Vec<_>>();

    for owner in port_owners {
        database.release_port(owner)?;
    }

    Ok(())
}

fn delete_optional_file(path: &Utf8Path) -> Result<(), DaemonError> {
    match state::fs::delete_file(path) {
        Ok(()) => Ok(()),
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn local_loopback_port_available(port: u16) -> bool {
    TcpListener::bind((RESOURCE_HOST, port)).is_ok()
}

#[cfg(test)]
#[doc(hidden)]
pub(crate) fn fake_runtime_catalog(
    manifest_url: &str,
) -> Result<ManagedResourceRuntimeCatalog, DaemonError> {
    Ok(ManagedResourceRuntimeCatalog::with_adapter(
        ManagedResourceInstallOptions {
            manifest_url: manifest_url.to_string(),
            target_platform: resources::TargetPlatform::current()?,
        },
        fake::FakeMailpitRuntimeAdapter::new()?,
    ))
}

#[cfg(test)]
#[doc(hidden)]
pub(crate) fn fake_runtime_catalog_with_manifest_client(
    manifest_url: &str,
    client: impl resources::ResourceHttpClient + Send + Sync + 'static,
) -> Result<ManagedResourceRuntimeCatalog, DaemonError> {
    let mut catalog = fake_runtime_catalog(manifest_url)?;
    catalog.http_client = Some(Arc::new(client));

    Ok(catalog)
}

#[cfg(test)]
#[doc(hidden)]
pub(crate) fn fake_unready_runtime_catalog(
    manifest_url: &str,
) -> Result<ManagedResourceRuntimeCatalog, DaemonError> {
    Ok(ManagedResourceRuntimeCatalog::with_adapter(
        ManagedResourceInstallOptions {
            manifest_url: manifest_url.to_string(),
            target_platform: resources::TargetPlatform::current()?,
        },
        fake::FakeMailpitRuntimeAdapter::unready()?,
    ))
}

#[cfg(test)]
#[doc(hidden)]
pub(crate) fn mailpit_runtime_catalog(
    manifest_url: &str,
) -> Result<ManagedResourceRuntimeCatalog, DaemonError> {
    Ok(ManagedResourceRuntimeCatalog::with_adapter(
        ManagedResourceInstallOptions {
            manifest_url: manifest_url.to_string(),
            target_platform: resources::TargetPlatform::current()?,
        },
        mailpit::MailpitRuntimeAdapter::new(),
    ))
}

#[cfg(test)]
#[doc(hidden)]
pub(crate) fn rustfs_runtime_catalog(
    manifest_url: &str,
) -> Result<ManagedResourceRuntimeCatalog, DaemonError> {
    Ok(ManagedResourceRuntimeCatalog::with_adapter(
        ManagedResourceInstallOptions {
            manifest_url: manifest_url.to_string(),
            target_platform: resources::TargetPlatform::current()?,
        },
        rustfs::RustfsRuntimeAdapter,
    ))
}

#[cfg(test)]
#[doc(hidden)]
pub(crate) fn postgres_runtime_catalog(
    manifest_url: &str,
) -> Result<ManagedResourceRuntimeCatalog, DaemonError> {
    Ok(ManagedResourceRuntimeCatalog::with_adapter(
        ManagedResourceInstallOptions {
            manifest_url: manifest_url.to_string(),
            target_platform: resources::TargetPlatform::current()?,
        },
        postgres::PostgresRuntimeAdapter::new(),
    ))
}

#[cfg(test)]
#[doc(hidden)]
pub(crate) fn postgres_runtime_catalog_with_readiness_timeout(
    manifest_url: &str,
    readiness_timeout: Duration,
) -> Result<ManagedResourceRuntimeCatalog, DaemonError> {
    Ok(ManagedResourceRuntimeCatalog::with_adapter(
        ManagedResourceInstallOptions {
            manifest_url: manifest_url.to_string(),
            target_platform: resources::TargetPlatform::current()?,
        },
        postgres::PostgresRuntimeAdapter::with_readiness_timeout(readiness_timeout),
    ))
}
