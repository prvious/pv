#[cfg(test)]
mod fake;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::net::TcpListener;
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use resources::{
    ManagedResourceCommands, ResourceAdapter, ResourceName, ResourcesError, TrackName,
    TrackSelector,
};
use state::{
    Database, EnvContextValues, ManagedResourceTrackRecord, PortOwner, PortRequest, ProjectRecord,
    PvPaths, RUNTIME_PORT_FALLBACK_END, RUNTIME_PORT_FALLBACK_START, ResourceAllocationRecord,
    RuntimeObservedStatus, RuntimeSubject, StateError,
};

use crate::{DaemonError, ProcessSpec, ProcessSupervisor, ReadinessCheck, wait_for_readiness};

const DEFAULT_MANIFEST_URL: &str = "https://artifacts.prvious.test/manifest.json";
const RESOURCE_HOST: &str = "127.0.0.1";
const RESOURCE_READINESS_TIMEOUT: Duration = Duration::from_secs(15);
const RESOURCE_STOP_GRACE_PERIOD: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ManagedResourcePortSpec {
    pub name: &'static str,
    pub preferred_port: u16,
    pub env_key: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ManagedResourcePortAssignment {
    pub name: String,
    pub port: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ManagedResourceRuntimeContext {
    pub resource_name: String,
    pub track: String,
    pub artifact_path: camino::Utf8PathBuf,
    pub data_dir: camino::Utf8PathBuf,
    pub ports: BTreeMap<String, u16>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ManagedResourceArtifactAdapter {
    resource_name: ResourceName,
    executable_relative_path: Utf8PathBuf,
}

impl ManagedResourceArtifactAdapter {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "production adapter modules construct daemon Managed Resource artifact adapters in follow-up PRs"
        )
    )]
    pub(crate) fn new(
        resource_name: &str,
        executable_relative_path: impl Into<Utf8PathBuf>,
    ) -> Result<Self, DaemonError> {
        Ok(Self {
            resource_name: ResourceName::new(resource_name)?,
            executable_relative_path: executable_relative_path.into(),
        })
    }

    pub(crate) fn executable_path(&self, release: &Utf8Path) -> Utf8PathBuf {
        release.join(&self.executable_relative_path)
    }
}

impl ResourceAdapter for ManagedResourceArtifactAdapter {
    fn resource_name(&self) -> &ResourceName {
        &self.resource_name
    }

    fn validate_installation(&self, root: &Utf8Path) -> resources::Result<()> {
        let executable_path = self.executable_path(root);
        if path_is_file(&executable_path)? {
            return Ok(());
        }

        Err(ResourcesError::InvalidArtifactLayout {
            resource: self.resource_name.as_str().to_string(),
            reason: format!("missing executable `{}`", self.executable_relative_path),
        })
    }
}

pub(crate) trait ManagedResourceRuntimeAdapter: Send + Sync {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "production adapter catalog registration uses resource names in follow-up PRs"
        )
    )]
    fn resource_name(&self) -> &'static str;

    fn artifact_adapter(&self) -> Result<ManagedResourceArtifactAdapter, DaemonError>;

    fn port_specs(&self) -> &'static [ManagedResourcePortSpec];

    fn build_process_spec(
        &self,
        paths: &PvPaths,
        context: &ManagedResourceRuntimeContext,
    ) -> Result<ProcessSpec, DaemonError>;

    fn readiness(
        &self,
        context: &ManagedResourceRuntimeContext,
    ) -> Result<ReadinessCheck, DaemonError>;

    #[cfg(test)]
    fn readiness_timeout(&self) -> Duration {
        RESOURCE_READINESS_TIMEOUT
    }

    fn resource_env(
        &self,
        context: &ManagedResourceRuntimeContext,
    ) -> Result<EnvContextValues, DaemonError>;

    fn reconcile_allocations(
        &self,
        _paths: &PvPaths,
        _database: &mut Database,
        _context: &ManagedResourceRuntimeContext,
        _allocations: &[ResourceAllocationRecord],
    ) -> Result<(), DaemonError> {
        Ok(())
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
    strict_unsupported_resources: bool,
}

impl ManagedResourceRuntimeCatalog {
    pub(crate) fn production() -> Self {
        Self {
            adapters: BTreeMap::new(),
            install_options: ManagedResourceInstallOptions {
                manifest_url: DEFAULT_MANIFEST_URL.to_string(),
                target_platform: current_target_platform(),
            },
            strict_unsupported_resources: false,
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
            strict_unsupported_resources: true,
        }
    }

    fn adapter(&self, resource_name: &str) -> Option<&dyn ManagedResourceRuntimeAdapter> {
        self.adapters.get(resource_name).map(Box::as_ref)
    }

    fn contains(&self, resource_name: &str) -> bool {
        self.adapters.contains_key(resource_name)
    }

    fn should_error_on_unsupported_resource(&self) -> bool {
        self.strict_unsupported_resources
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
    let Some(adapter) = catalog.adapter(&resource.resource_name) else {
        if !catalog.should_error_on_unsupported_resource() {
            return Ok(());
        }

        return Err(DaemonError::UnsupportedManagedResourceRuntime {
            resource: resource.resource_name.clone(),
        });
    };
    let track_record = ensure_track_artifact(paths, database, catalog, adapter, resource).await?;
    let Some(artifact_path) = track_record.current_artifact_path else {
        return Err(DaemonError::ManagedResourceArtifactMissing {
            resource: resource.resource_name.clone(),
            track: resource.track.clone(),
        });
    };
    let port_assignments =
        assign_named_ports(database, adapter, &resource.resource_name, &resource.track)?;
    let ports = port_assignments
        .into_iter()
        .map(|assignment| (assignment.name, assignment.port))
        .collect::<BTreeMap<_, _>>();
    let context = ManagedResourceRuntimeContext {
        resource_name: resource.resource_name.clone(),
        track: resource.track.clone(),
        artifact_path,
        data_dir: paths.resource_data_dir(&resource.resource_name, &resource.track),
        ports,
    };
    let subject = RuntimeSubject::Resource {
        name: resource.resource_name.clone(),
        track: resource.track.clone(),
    };
    let result = async {
        let spec = adapter.build_process_spec(paths, &context)?;
        let readiness = adapter.readiness(&context)?;
        let readiness_timeout = adapter_readiness_timeout(adapter);

        start_or_adopt_runtime(supervisor, spec, readiness, readiness_timeout).await?;

        let env = adapter.resource_env(&context)?;
        database.record_managed_resource_track_env_context(
            &resource.resource_name,
            &resource.track,
            &env,
        )?;
        let allocations = desired_allocations(database, project, plan, resource)?;
        adapter.reconcile_allocations(paths, database, &context, &allocations)?;
        database.record_runtime_observed_snapshot(
            subject.clone(),
            RuntimeObservedStatus::Running,
            Some("Managed Resource runtime is ready"),
        )?;

        Ok::<(), DaemonError>(())
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
    let record = database
        .managed_resource_tracks()?
        .into_iter()
        .find(|record| {
            record.resource_name == resource_name
                && record.track == track
                && record.current_artifact_path.is_some()
        });

    Ok(record)
}

fn install_missing_track_blocking(
    paths: PvPaths,
    install_options: ManagedResourceInstallOptions,
    adapter: ManagedResourceArtifactAdapter,
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
) -> Result<Vec<ManagedResourcePortAssignment>, DaemonError> {
    let mut assignments = Vec::new();

    for port_spec in adapter.port_specs() {
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

        assignments.push(ManagedResourcePortAssignment {
            name: port_spec.name.to_string(),
            port: assignment.port,
        });
    }

    Ok(assignments)
}

async fn start_or_adopt_runtime(
    supervisor: &ProcessSupervisor,
    spec: ProcessSpec,
    readiness: ReadinessCheck,
    readiness_timeout: Duration,
) -> Result<(), DaemonError> {
    if supervisor.adopt(&spec)?.is_some() {
        wait_for_readiness(readiness, readiness_timeout).await?;

        return Ok(());
    }
    if crate::supervisor::probe_readiness_once(&readiness)
        .await
        .is_ok()
    {
        return Err(DaemonError::UnexpectedProtocolResponse {
            reason: format!(
                "runtime `{}` is listening but no PV-owned process could be verified",
                spec.name
            ),
        });
    }

    let mut process = supervisor.start(spec.clone()).await?;
    if let Err(error) = wait_for_readiness(readiness, readiness_timeout).await {
        process.stop(RESOURCE_STOP_GRACE_PERIOD).await?;
        cleanup_started_runtime_files(&spec)?;

        return Err(error);
    }
    if process.has_exited()? {
        return Err(DaemonError::UnexpectedProtocolResponse {
            reason: format!(
                "runtime `{}` exited before readiness was verified",
                spec.name
            ),
        });
    }

    Ok(())
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
        if !catalog.contains(&track.resource_name)
            || track.usage_count > 0
            || demanded_tracks.contains(&(track.resource_name.clone(), track.track.clone()))
        {
            continue;
        }

        let Some(adapter) = catalog.adapter(&track.resource_name) else {
            continue;
        };

        stop_resource_runtime(paths, database, supervisor, adapter, &track).await?;
    }

    Ok(())
}

async fn stop_resource_runtime(
    paths: &PvPaths,
    database: &mut Database,
    supervisor: &ProcessSupervisor,
    adapter: &dyn ManagedResourceRuntimeAdapter,
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
    cleanup_resource_runtime(paths, database, adapter, track)?;

    Ok(())
}

fn cleanup_resource_runtime(
    paths: &PvPaths,
    database: &mut Database,
    adapter: &dyn ManagedResourceRuntimeAdapter,
    track: &ManagedResourceTrackRecord,
) -> Result<(), DaemonError> {
    delete_optional_file(&paths.resource_pid(&track.resource_name, &track.track))?;
    delete_optional_file(&paths.resource_runtime_metadata(&track.resource_name, &track.track))?;
    delete_optional_file(&paths.resource_runtime_config(&track.resource_name, &track.track))?;

    for port_spec in adapter.port_specs() {
        database.release_port(PortOwner::Resource {
            name: track.resource_name.clone(),
            track: track.track.clone(),
            port: port_spec.name.to_string(),
        })?;
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

#[expect(
    clippy::disallowed_methods,
    reason = "Managed Resource artifact validation owns direct filesystem metadata checks"
)]
fn path_is_file(path: &Utf8Path) -> resources::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.is_file()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(ResourcesError::Filesystem {
            path: path.to_string(),
            reason: source.to_string(),
        }),
    }
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
