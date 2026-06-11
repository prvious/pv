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
use std::time::{Duration, Instant};

use camino::Utf8Path;
use resources::{ManagedResourceCommands, ResourceAdapter, TrackName, TrackSelector};
use state::{
    Database, EnvContextValues, ManagedResourceDesiredState, ManagedResourceTrackRecord, PortOwner,
    PortRequest, ProjectRecord, PvPaths, RUNTIME_PORT_FALLBACK_END, RUNTIME_PORT_FALLBACK_START,
    ResourceAllocationRecord, RuntimeObservedStatus, RuntimeSubject, StateError,
};
use tokio::time::{sleep, timeout};

use crate::{DaemonError, ProcessSpec, ProcessSupervisor, ReadinessCheck, wait_for_readiness};

const RESOURCE_HOST: &str = "127.0.0.1";
const RESOURCE_READINESS_TIMEOUT: Duration = Duration::from_secs(15);
const ASYNC_READINESS_POLL_INTERVAL: Duration = Duration::from_millis(50);
const RESOURCE_STOP_GRACE_PERIOD: Duration = Duration::from_secs(10);
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
}

impl ManagedResourceRuntimeCatalog {
    pub(crate) fn production() -> Self {
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

        Self {
            adapters,
            install_options: ManagedResourceInstallOptions {
                manifest_url: resources::default_artifact_manifest_url().to_string(),
                target_platform: current_target_platform(),
            },
        }
    }

    pub(crate) fn without_adapters() -> Self {
        Self {
            adapters: BTreeMap::new(),
            install_options: ManagedResourceInstallOptions {
                manifest_url: resources::default_artifact_manifest_url().to_string(),
                target_platform: current_target_platform(),
            },
        }
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
        }
    }

    fn adapter(&self, resource_name: &str) -> Option<&dyn ManagedResourceRuntimeAdapter> {
        self.adapters.get(resource_name).map(Box::as_ref)
    }
}

pub(crate) async fn reconcile_project_resources(
    paths: &PvPaths,
    database: &mut Database,
    project: &ProjectRecord,
    plan: &crate::project_env::ProjectResourcePlan,
) -> Result<(), DaemonError> {
    let catalog = ManagedResourceRuntimeCatalog::production();

    reconcile_project_resources_with_catalog(paths, database, project, plan, &catalog).await
}

pub(crate) async fn reconcile_project_resources_with_catalog(
    paths: &PvPaths,
    database: &mut Database,
    project: &ProjectRecord,
    plan: &crate::project_env::ProjectResourcePlan,
    catalog: &ManagedResourceRuntimeCatalog,
) -> Result<(), DaemonError> {
    let supervisor = ProcessSupervisor::new(paths.clone());
    let demanded_tracks = plan
        .resources
        .iter()
        .map(|resource| (resource.resource_name.clone(), resource.track.clone()))
        .collect::<BTreeSet<_>>();

    stop_undemanded_catalog_runtimes(paths, database, catalog, &supervisor, &demanded_tracks)
        .await?;

    for resource in &plan.resources {
        reconcile_resource_track(
            paths,
            database,
            project,
            plan,
            catalog,
            &supervisor,
            resource,
        )
        .await?;
    }

    Ok(())
}

pub(crate) async fn reconcile_system_resources(paths: &PvPaths) -> Result<(), DaemonError> {
    let catalog = ManagedResourceRuntimeCatalog::production();
    let mut database = Database::open(paths)?;

    reconcile_system_resources_with_catalog(paths, &mut database, &catalog).await
}

pub(crate) async fn reconcile_system_resources_with_catalog(
    paths: &PvPaths,
    database: &mut Database,
    catalog: &ManagedResourceRuntimeCatalog,
) -> Result<(), DaemonError> {
    let supervisor = ProcessSupervisor::new(paths.clone());
    let demanded_tracks = BTreeSet::new();

    stop_undemanded_catalog_runtimes(paths, database, catalog, &supervisor, &demanded_tracks).await
}

async fn reconcile_resource_track(
    paths: &PvPaths,
    database: &mut Database,
    project: &ProjectRecord,
    plan: &crate::project_env::ProjectResourcePlan,
    catalog: &ManagedResourceRuntimeCatalog,
    supervisor: &ProcessSupervisor,
    resource: &state::ProjectManagedResourceInput,
) -> Result<(), DaemonError> {
    let subject = RuntimeSubject::Resource {
        name: resource.resource_name.clone(),
        track: resource.track.clone(),
    };
    let Some(adapter) = catalog.adapter(&resource.resource_name) else {
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
        let track_record =
            ensure_track_artifact(paths, database, catalog, adapter, resource).await?;
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
            if ports_occupied_without_recorded_runtime(paths, supervisor, resource, &ports)?
                && attempt < RESOURCE_START_ATTEMPTS
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
            };
            let mut runtime_attempt = ResourceRuntimeAttempt {
                paths,
                database,
                project,
                plan,
                adapter,
                supervisor,
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

async fn ensure_track_artifact(
    paths: &PvPaths,
    database: &mut Database,
    catalog: &ManagedResourceRuntimeCatalog,
    adapter: &dyn ManagedResourceRuntimeAdapter,
    resource: &state::ProjectManagedResourceInput,
) -> Result<ManagedResourceTrackRecord, DaemonError> {
    if let Some(record) = installed_track(database, &resource.resource_name, &resource.track)? {
        return Ok(record);
    }

    let artifact_adapter = adapter.artifact_adapter()?;
    let install_options = catalog.install_options.clone();
    let install_paths = paths.clone();
    let resource_name = resource.resource_name.clone();
    let track = resource.track.clone();

    tokio::task::spawn_blocking(move || {
        install_missing_track_blocking(
            install_paths,
            install_options,
            artifact_adapter,
            resource_name,
            track,
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

fn install_missing_track_blocking(
    paths: PvPaths,
    install_options: ManagedResourceInstallOptions,
    adapter: resources::RuntimeArtifactAdapter,
    resource_name: String,
    track: String,
) -> Result<(), DaemonError> {
    let commands = ManagedResourceCommands::new(
        paths,
        install_options.manifest_url,
        install_options.target_platform,
    );
    let client = resources::UreqResourceHttpClient::default();
    let track = TrackName::new(track)?;

    commands.install(&adapter, TrackSelector::Track(track), &client)?;
    if adapter.resource_name().as_str() != resource_name {
        return Err(DaemonError::UnexpectedProtocolResponse {
            reason: format!(
                "runtime adapter installed `{}` while reconciling `{resource_name}`",
                adapter.resource_name()
            ),
        });
    }

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
    if let ManagedResourceReadiness::TcpHttp(check) = readiness
        && crate::supervisor::probe_readiness_once(check).await.is_ok()
    {
        return Err(DaemonError::NonPvManagedResourceRuntimeListener { name: spec.name });
    }

    let mut process = supervisor.start(spec.clone()).await?;
    if let Err(error) = wait_for_managed_resource_readiness(readiness, readiness_timeout).await {
        process.stop(RESOURCE_STOP_GRACE_PERIOD).await?;
        cleanup_started_runtime_files(&spec)?;

        return Err(error);
    }
    tokio::time::sleep(Duration::from_millis(25)).await;
    if process.has_exited()? {
        cleanup_started_runtime_files(&spec)?;

        return Err(DaemonError::UnexpectedProtocolResponse {
            reason: format!(
                "runtime `{}` exited before readiness was verified",
                spec.name
            ),
        });
    }

    Ok(())
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
    demanded_tracks: &BTreeSet<(String, String)>,
) -> Result<(), DaemonError> {
    let tracks = database.managed_resource_tracks()?;

    for track in tracks {
        let Some(_adapter) = catalog.adapter(&track.resource_name) else {
            continue;
        };
        if track.usage_count > 0
            || demanded_tracks.contains(&(track.resource_name.clone(), track.track.clone()))
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

fn current_target_platform() -> resources::TargetPlatform {
    if cfg!(target_arch = "aarch64") {
        resources::TargetPlatform::DarwinArm64
    } else {
        resources::TargetPlatform::DarwinAmd64
    }
}

#[cfg(test)]
#[doc(hidden)]
pub(crate) fn fake_runtime_catalog(
    manifest_url: &str,
) -> Result<ManagedResourceRuntimeCatalog, DaemonError> {
    Ok(ManagedResourceRuntimeCatalog::with_adapter(
        ManagedResourceInstallOptions {
            manifest_url: manifest_url.to_string(),
            target_platform: current_target_platform(),
        },
        fake::FakeMailpitRuntimeAdapter::new()?,
    ))
}

#[cfg(test)]
#[doc(hidden)]
pub(crate) fn fake_unready_runtime_catalog(
    manifest_url: &str,
) -> Result<ManagedResourceRuntimeCatalog, DaemonError> {
    Ok(ManagedResourceRuntimeCatalog::with_adapter(
        ManagedResourceInstallOptions {
            manifest_url: manifest_url.to_string(),
            target_platform: current_target_platform(),
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
            target_platform: current_target_platform(),
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
            target_platform: current_target_platform(),
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
            target_platform: current_target_platform(),
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
            target_platform: current_target_platform(),
        },
        postgres::PostgresRuntimeAdapter::with_readiness_timeout(readiness_timeout),
    ))
}
