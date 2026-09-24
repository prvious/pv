use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::io;
use std::sync::{Arc, Mutex, OnceLock};

use crate::DaemonError;
use crate::gateway::{
    CADDY_NOT_INSTALLED, GatewayPfRoutingState, ProjectGatewayReconciliationOutcome,
    ReconciliationOutcome, reconcile_gateway_runtimes, reconcile_gateway_runtimes_with_phase_log,
    reconcile_gateway_runtimes_with_phase_log_and_fallback_shutdown,
    reconcile_project_gateway_runtimes_with_phase_log,
    reconcile_project_gateway_runtimes_with_phase_log_and_fallback_shutdown,
};
use crate::ipc::LocalStream;
use crate::managed_resources::{
    ManagedResourceRuntimeCatalog, ManagedResourceUpdateReport,
    reconcile_persisted_resource_track_for_projects_with_progress,
    reconcile_system_resources_with_catalog_and_progress, reconcile_system_resources_with_progress,
    stop_undemanded_system_resource_runtimes, verify_system_resource_installations,
};
#[cfg(test)]
use crate::project_env::reconcile_project_env_with_runtime_catalog_and_progress;
use crate::project_env::{
    DemandedResourceTrack, ProjectApplyOptions, ProjectApplyStage, ProjectDemand,
    discover_project_demand, reconcile_project_env_with_runtime_catalog_and_progress_outcome,
    record_project_env_failure,
};
use crate::reconciliation::{
    EnqueueResult, QueuedReconciliation, ReconciliationJobTiming, ReconciliationQueue,
    ReconciliationScope, RunningReconciliation,
};
use crate::structured_log::{self, PhaseOutcome, ReconciliationPhase, ReconciliationPhaseLog};
use protocol::{DaemonEvent, DaemonResponse, DaemonTransport, write_line};
use state::{
    Database, JobDiagnosticSubject, ManagedResourceDesiredState, ProjectEnvObservedStatus,
    ProjectRecord, PvPaths, ResourceAllocationStatus, RuntimeObservedStatus, RuntimeSubject,
    StateError,
};
use tokio::io::AsyncWrite;
use tokio::sync::mpsc::{Receiver, Sender, channel};
use tokio::sync::{oneshot, watch};
use tokio::time::{Duration, Instant, MissedTickBehavior, interval_at, timeout};

const FOREGROUND_JOB_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
const FOREGROUND_JOB_STREAM_WRITE_TIMEOUT: Duration = Duration::from_millis(100);
const FOREGROUND_JOB_PROGRESS_BUFFER: usize = 16;
const FOREGROUND_JOB_QUEUE_HEARTBEAT: &str = "Waiting for the reconciliation slot";
const STARTUP_JOBS_LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug)]
enum ForegroundJobEvent {
    DownloadProgress {
        resource: String,
        track: String,
        artifact_version: String,
        downloaded_bytes: u64,
        total_bytes: u64,
    },
}

struct CompletedUpdateJob {
    summary: String,
    coverage: Vec<JobDiagnosticSubject>,
}

struct CompletedReconciliationJob {
    summary: String,
    coverage: Vec<JobDiagnosticSubject>,
}

struct FailedUpdateJob {
    error: Box<DaemonError>,
    subject: JobDiagnosticSubject,
}

impl FailedUpdateJob {
    fn new(error: DaemonError, subject: JobDiagnosticSubject) -> Self {
        Self {
            error: Box::new(error),
            subject,
        }
    }
}

#[derive(Debug)]
struct StreamedJobCompletion<Output> {
    result: Output,
    transport_is_open: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct DaemonDownloadProgress {
    sender: Option<Sender<ForegroundJobEvent>>,
    phase_sender: Option<watch::Sender<Vec<ReconciliationPhase>>>,
    phase_log: Option<Arc<Mutex<ReconciliationPhaseLog>>>,
    manifest_snapshot:
        Arc<OnceLock<Result<Arc<resources::ArtifactManifestRefresh>, resources::ResourcesError>>>,
    install_failures: Arc<Mutex<BTreeMap<String, String>>>,
    suppress_operation_phases: bool,
}

impl DaemonDownloadProgress {
    fn new(
        sender: Sender<ForegroundJobEvent>,
        phase_sender: watch::Sender<Vec<ReconciliationPhase>>,
    ) -> Self {
        Self {
            sender: Some(sender),
            phase_sender: Some(phase_sender),
            phase_log: None,
            manifest_snapshot: Arc::new(OnceLock::new()),
            install_failures: Arc::new(Mutex::new(BTreeMap::new())),
            suppress_operation_phases: false,
        }
    }

    pub(crate) fn disabled() -> Self {
        Self {
            sender: None,
            phase_sender: None,
            phase_log: None,
            manifest_snapshot: Arc::new(OnceLock::new()),
            install_failures: Arc::new(Mutex::new(BTreeMap::new())),
            suppress_operation_phases: false,
        }
    }

    fn with_phase_log(mut self, phase_log: ReconciliationPhaseLog) -> Self {
        self.phase_log = Some(Arc::new(Mutex::new(phase_log)));
        self
    }

    pub(crate) fn set_install_failure(&self, label: String, failure: Option<String>) {
        let mut failures = match self.install_failures.lock() {
            Ok(failures) => failures,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(failure) = failure {
            failures.insert(label, failure);
        } else {
            failures.remove(&label);
        }
    }

    pub(crate) fn install_failure(&self, label: &str) -> Option<String> {
        let failures = match self.install_failures.lock() {
            Ok(failures) => failures,
            Err(poisoned) => poisoned.into_inner(),
        };
        failures.get(label).cloned()
    }

    /// Keeps foreground download reporting and structured diagnostics but stops writing timed
    /// operation records, so work already covered by an open phase is not timed again inside
    /// it.
    fn suppressing_operation_phases(mut self) -> Self {
        self.suppress_operation_phases = true;
        self
    }

    /// A clone for retrying the Resources phase. A cached snapshot failure must not be replayed,
    /// so the retry gets a fresh cell and re-fetches; a cached success is kept, so one successful
    /// refresh is still shared across the pass.
    fn retrying_manifest_snapshot(&self) -> Self {
        let mut retry = self.clone();
        if matches!(self.manifest_snapshot.get(), Some(Err(_))) {
            retry.manifest_snapshot = Arc::new(OnceLock::new());
        }

        retry
    }

    fn send_download_progress(
        &self,
        artifact: &resources::ManifestArtifact,
        downloaded_bytes: u64,
    ) {
        let Some(sender) = &self.sender else {
            return;
        };
        let _sent = sender.try_send(ForegroundJobEvent::DownloadProgress {
            resource: artifact.resource_name().as_str().to_string(),
            track: artifact.track().as_str().to_string(),
            artifact_version: artifact.artifact_version().as_str().to_string(),
            downloaded_bytes,
            total_bytes: artifact.size(),
        });
    }

    pub(crate) fn manifest_snapshot(
        &self,
        commands: &resources::ManagedResourceCommands,
        client: &(impl resources::ResourceHttpClient + ?Sized),
    ) -> Result<Arc<resources::ArtifactManifestRefresh>, DaemonError> {
        self.initialize_manifest_snapshot(commands, client, false)
    }

    pub(crate) fn latest_manifest_snapshot(
        &self,
        commands: &resources::ManagedResourceCommands,
        client: &(impl resources::ResourceHttpClient + ?Sized),
    ) -> Result<Arc<resources::ArtifactManifestRefresh>, DaemonError> {
        self.initialize_manifest_snapshot(commands, client, true)
    }

    fn initialize_manifest_snapshot(
        &self,
        commands: &resources::ManagedResourceCommands,
        client: &(impl resources::ResourceHttpClient + ?Sized),
        latest_only: bool,
    ) -> Result<Arc<resources::ArtifactManifestRefresh>, DaemonError> {
        self.manifest_snapshot
            .get_or_init(|| {
                let result = if latest_only {
                    commands.latest_manifest_snapshot_with_progress(client, self)
                } else {
                    commands.manifest_snapshot_with_progress(client, self)
                };

                result.map(Arc::new)
            })
            .clone()
            .map_err(|error| resources::ManagedResourceCommandError::from(error).into())
    }
}

impl resources::DownloadProgress for DaemonDownloadProgress {
    fn report(&self, event: resources::DownloadProgressEvent<'_>) {
        match event {
            resources::DownloadProgressEvent::Started { artifact } => {
                self.send_download_progress(artifact, 0);
            }
            resources::DownloadProgressEvent::Advanced {
                artifact,
                downloaded_bytes,
            }
            | resources::DownloadProgressEvent::Finished {
                artifact,
                downloaded_bytes,
            } => {
                self.send_download_progress(artifact, downloaded_bytes);
            }
        }
    }

    fn operation_started(&self, operation: resources::ResourceOperation<'_>) {
        let Some(phase_log) = &self.phase_log else {
            return;
        };
        let phase_log = match phase_log.lock() {
            Ok(phase_log) => phase_log,
            Err(poisoned) => poisoned.into_inner(),
        };
        phase_log.report_progress(resource_operation_phase(operation));
    }

    fn operation_finished(&self, event: resources::ResourceOperationEvent<'_, '_>) {
        let Some(phase_log) = &self.phase_log else {
            return;
        };
        if self.suppress_operation_phases {
            // The owning phase already times this work, but a cached-manifest fallback still
            // has to be reported, so it is recorded as a plain diagnostic instead.
            if let resources::ResourceOperation::Manifest = event.operation
                && let resources::ResourceOperationOutcome::Fallback { reason } = event.outcome
            {
                let phase_log = match phase_log.lock() {
                    Ok(phase_log) => phase_log,
                    Err(poisoned) => poisoned.into_inner(),
                };
                phase_log.artifact_manifest_fallback(reason);
            }

            return;
        }
        let manifest_operation = matches!(event.operation, resources::ResourceOperation::Manifest);
        let phase = resource_operation_phase(event.operation);
        let (subject, counts) = match event.operation {
            resources::ResourceOperation::Manifest => {
                ("artifact_manifest".to_owned(), vec![("manifest_count", 1)])
            }
            resources::ResourceOperation::Download(artifact) => (
                artifact_subject(artifact),
                vec![("artifact_count", 1), ("artifact_bytes", artifact.size())],
            ),
            resources::ResourceOperation::Install(artifact) => {
                (artifact_subject(artifact), vec![("artifact_count", 1)])
            }
        };
        let (outcome, fields) = match event.outcome {
            resources::ResourceOperationOutcome::Succeeded if manifest_operation => {
                (PhaseOutcome::Succeeded, vec![("manifest_source", "latest")])
            }
            resources::ResourceOperationOutcome::Succeeded => (PhaseOutcome::Succeeded, Vec::new()),
            resources::ResourceOperationOutcome::Failed => (PhaseOutcome::Failed, Vec::new()),
            resources::ResourceOperationOutcome::Skipped => (PhaseOutcome::Skipped, Vec::new()),
            resources::ResourceOperationOutcome::Fallback { reason } => (
                PhaseOutcome::Fallback,
                vec![("manifest_source", "cached"), ("fallback_reason", reason)],
            ),
        };
        let phase_log = match phase_log.lock() {
            Ok(phase_log) => phase_log,
            Err(poisoned) => poisoned.into_inner(),
        };
        phase_log.completed_with_fields(phase, &subject, outcome, event.elapsed, &counts, &fields);
    }
}

fn resource_operation_phase(operation: resources::ResourceOperation<'_>) -> ReconciliationPhase {
    match operation {
        resources::ResourceOperation::Manifest => ReconciliationPhase::Manifest,
        resources::ResourceOperation::Download(_artifact) => ReconciliationPhase::Download,
        resources::ResourceOperation::Install(_artifact) => ReconciliationPhase::Install,
    }
}

fn artifact_subject(artifact: &resources::ManifestArtifact) -> String {
    format!(
        "{}:{}:{}",
        artifact.resource_name(),
        artifact.track(),
        artifact.artifact_version()
    )
}

pub(crate) async fn run_job(
    paths: PvPaths,
    queue: ReconciliationQueue,
    transport: DaemonTransport<LocalStream>,
    kind: &str,
    scope: &str,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    fallback_shutdown: &watch::Receiver<bool>,
) -> Result<(), DaemonError> {
    let parsed_scope = scope.parse::<ReconciliationScope>();
    if kind == "reconcile" {
        return match parsed_scope {
            Ok(parsed_scope) => {
                run_reconciliation_job(
                    paths,
                    queue,
                    transport,
                    parsed_scope,
                    runtime_catalog,
                    fallback_shutdown,
                )
                .await
            }
            Err(error) => {
                run_invalid_reconciliation_scope_job(paths, transport, scope, error).await
            }
        };
    }
    if kind == "update" && scope == "system" {
        return run_update_job(paths, queue, transport, runtime_catalog, fallback_shutdown).await;
    }

    run_started_job(paths, transport, kind, scope).await
}

#[cfg(test)]
pub(crate) async fn run_background_reconciliation_job(
    paths: PvPaths,
    queue: ReconciliationQueue,
    scope: ReconciliationScope,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
) -> Result<(), DaemonError> {
    run_background_reconciliation_job_with_origin(paths, queue, scope, runtime_catalog)
        .await
        .map_err(BackgroundReconciliationError::into_error)
}

pub(crate) enum BackgroundReconciliationError {
    Admission(Box<DaemonError>),
    Execution {
        job_id: String,
        error: Box<DaemonError>,
        recording_error: Option<Box<DaemonError>>,
    },
}

#[cfg(test)]
impl BackgroundReconciliationError {
    pub(crate) fn into_error(self) -> DaemonError {
        match self {
            Self::Admission(error) | Self::Execution { error, .. } => *error,
        }
    }
}

#[cfg(test)]
pub(crate) async fn run_background_reconciliation_job_with_origin(
    paths: PvPaths,
    queue: ReconciliationQueue,
    scope: ReconciliationScope,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
) -> Result<(), BackgroundReconciliationError> {
    let Some(result) = enqueue_background_reconciliation_job(&paths, &queue, scope)? else {
        return Ok(());
    };
    let EnqueueResult::Queued(queued) = result else {
        return Ok(());
    };

    complete_queued_background_reconciliation_job(&paths, queued, runtime_catalog, None).await
}

pub(crate) fn enqueue_background_reconciliation_job(
    paths: &PvPaths,
    queue: &ReconciliationQueue,
    scope: ReconciliationScope,
) -> Result<Option<EnqueueResult>, BackgroundReconciliationError> {
    if let ReconciliationScope::Project { id } = &scope
        && !project_exists(paths, id.as_str())
            .map_err(|error| BackgroundReconciliationError::Admission(Box::new(error)))?
    {
        return Ok(None);
    }

    enqueue_reconciliation_job(paths, queue, scope)
        .map(Some)
        .map_err(|error| BackgroundReconciliationError::Admission(Box::new(error)))
}

fn project_exists(paths: &PvPaths, project_id: &str) -> Result<bool, DaemonError> {
    Ok(Database::open(paths)?.project_by_id(project_id)?.is_some())
}

pub(crate) async fn run_startup_reconciliation_job(
    paths: PvPaths,
    queue: ReconciliationQueue,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    mut shutdown: oneshot::Receiver<()>,
    fallback_shutdown: watch::Receiver<bool>,
) -> Result<(), BackgroundReconciliationError> {
    let result = loop {
        let enqueue_paths = paths.clone();
        let enqueue_queue = queue.clone();
        let mut enqueue_task = tokio::task::spawn_blocking(move || {
            enqueue_startup_reconciliation_job(&enqueue_paths, &enqueue_queue)
        });
        let result = tokio::select! {
            biased;
            result = &mut enqueue_task => result
                .map_err(DaemonError::from)
                .map_err(|error| BackgroundReconciliationError::Admission(Box::new(error)))?,
            _ = &mut shutdown => {
                let enqueue_result = enqueue_task
                    .await
                    .map_err(DaemonError::from)
                    .map_err(|error| BackgroundReconciliationError::Admission(Box::new(error)))?;
                return match enqueue_result {
                    Ok(_result) => Ok(()),
                    Err(DaemonError::State(StateError::CoordinationLockHeld { path }))
                        if path == paths.jobs_lock() =>
                    {
                        Ok(())
                    }
                    Err(error) => Err(BackgroundReconciliationError::Admission(Box::new(error))),
                };
            }
        };

        match result {
            Ok(result) => break result,
            Err(DaemonError::State(StateError::CoordinationLockHeld { path }))
                if path == paths.jobs_lock() =>
            {
                tokio::select! {
                    _ = tokio::time::sleep(STARTUP_JOBS_LOCK_RETRY_INTERVAL) => {}
                    _ = &mut shutdown => return Ok(()),
                }
            }
            Err(error) => {
                return Err(BackgroundReconciliationError::Admission(Box::new(error)));
            }
        }
    };

    let EnqueueResult::Queued(queued) = result else {
        return Ok(());
    };
    let Some(running) = wait_for_startup_reconciliation_turn(queued, &mut shutdown).await else {
        return Ok(());
    };

    complete_running_background_reconciliation_job(
        &paths,
        running,
        runtime_catalog,
        Some(&shutdown),
        Some(&fallback_shutdown),
    )
    .await
}

async fn wait_for_startup_reconciliation_turn(
    queued: QueuedReconciliation,
    shutdown: &mut oneshot::Receiver<()>,
) -> Option<RunningReconciliation> {
    tokio::select! {
        biased;
        _ = shutdown => None,
        running = queued.wait_for_turn() => Some(running),
    }
}

async fn wait_for_fallback_shutdown(shutdown: &mut watch::Receiver<bool>) {
    if shutdown.wait_for(|requested| *requested).await.is_err() {
        std::future::pending::<()>().await;
    }
}

fn fallback_shutdown_requested(shutdown: Option<&watch::Receiver<bool>>) -> bool {
    shutdown.is_some_and(|shutdown| *shutdown.borrow())
}

pub(crate) async fn complete_queued_background_reconciliation_job(
    paths: &PvPaths,
    queued: QueuedReconciliation,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    fallback_shutdown: Option<&watch::Receiver<bool>>,
) -> Result<(), BackgroundReconciliationError> {
    let running = match fallback_shutdown {
        Some(fallback_shutdown) => {
            let mut fallback_shutdown = fallback_shutdown.clone();
            tokio::select! {
                biased;
                _ = wait_for_fallback_shutdown(&mut fallback_shutdown) => return Ok(()),
                running = queued.wait_for_turn() => running,
            }
        }
        None => queued.wait_for_turn().await,
    };

    complete_running_background_reconciliation_job(
        paths,
        running,
        runtime_catalog,
        None,
        fallback_shutdown,
    )
    .await
}

pub(crate) async fn complete_running_background_reconciliation_job(
    paths: &PvPaths,
    running: RunningReconciliation,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    shutdown: Option<&oneshot::Receiver<()>>,
    fallback_shutdown: Option<&watch::Receiver<bool>>,
) -> Result<(), BackgroundReconciliationError> {
    let job_id = running.job_id().to_string();
    let scope = running.scope().clone();
    let completion = complete_reconciliation_job_with_progress_outcome(
        paths,
        &job_id,
        &scope,
        runtime_catalog,
        DaemonDownloadProgress::disabled(),
        running.timing(),
        ReconciliationJobOptions {
            discard_obsolete_project: true,
            fallback_shutdown,
            ..ReconciliationJobOptions::default()
        },
        shutdown,
    )
    .await;

    match completion {
        ReconciliationJobCompletion::Succeeded(_summary) => {
            running.finish();
            Ok(())
        }
        ReconciliationJobCompletion::Cancelled => {
            drop(running);
            Ok(())
        }
        ReconciliationJobCompletion::Failed {
            error,
            recording_error,
        } => {
            running.finish();
            Err(BackgroundReconciliationError::Execution {
                job_id,
                error,
                recording_error,
            })
        }
    }
}

async fn run_reconciliation_job(
    paths: PvPaths,
    queue: ReconciliationQueue,
    mut transport: DaemonTransport<LocalStream>,
    scope: ReconciliationScope,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    fallback_shutdown: &watch::Receiver<bool>,
) -> Result<(), DaemonError> {
    let result = match enqueue_foreground_reconciliation_job(&paths, &queue, scope) {
        Ok(result) => result,
        Err(DaemonError::State(
            error @ (StateError::CoordinationLockHeld { .. } | StateError::ProjectNotFound { .. }),
        )) => {
            write_line(&mut transport, &DaemonResponse::error(error.to_string())).await?;

            return Ok(());
        }
        Err(error) => return Err(error),
    };

    match result {
        EnqueueResult::Queued(queued) => {
            let job_id = queued.job_id().to_string();
            let accepted_result = write_line(
                &mut transport,
                &DaemonResponse::accepted("job accepted", &job_id),
            )
            .await
            .map_err(DaemonError::from);
            let stream_is_open = accepted_result.is_ok();
            let Some((running, stream_is_open)) = wait_for_foreground_turn(
                queued,
                &mut transport,
                stream_is_open,
                FOREGROUND_JOB_HEARTBEAT_INTERVAL,
                Some(fallback_shutdown),
            )
            .await
            else {
                return accepted_result;
            };
            let scope = running.scope().clone();
            let result = stream_started_reconciliation_job_with_fallback(
                paths,
                transport,
                stream_is_open,
                running.job_id(),
                scope,
                runtime_catalog,
                ForegroundReconciliationOptions {
                    timing: running.timing(),
                    fallback_shutdown: Some(fallback_shutdown),
                },
            )
            .await;

            let result = match result {
                ForegroundReconciliationCompletion::Finalized(result) => {
                    running.finish();
                    result
                }
                ForegroundReconciliationCompletion::Cancelled => {
                    drop(running);
                    ReconciliationJobCompletion::Cancelled
                        .into_result()
                        .map(|_summary| ())
                }
            };

            foreground_reconciliation_result(accepted_result, result)
        }
        EnqueueResult::Coalesced(job) => {
            write_line(
                &mut transport,
                &DaemonResponse::accepted("reconciliation already queued or running", job.job_id()),
            )
            .await?;

            Ok(())
        }
    }
}

fn enqueue_foreground_reconciliation_job(
    paths: &PvPaths,
    queue: &ReconciliationQueue,
    scope: ReconciliationScope,
) -> Result<EnqueueResult, DaemonError> {
    require_project_scope_exists(paths, &scope)?;
    let result = enqueue_reconciliation_job(paths, queue, scope.clone())?;
    if matches!(&result, EnqueueResult::Coalesced(_)) {
        require_project_scope_exists(paths, &scope)?;
    }

    Ok(result)
}

fn require_project_scope_exists(
    paths: &PvPaths,
    scope: &ReconciliationScope,
) -> Result<(), DaemonError> {
    if let ReconciliationScope::Project { id } = scope
        && !project_exists(paths, id.as_str())?
    {
        return Err(StateError::ProjectNotFound {
            target: id.to_string(),
        }
        .into());
    }

    Ok(())
}

pub(crate) fn enqueue_reconciliation_job(
    paths: &PvPaths,
    queue: &ReconciliationQueue,
    scope: ReconciliationScope,
) -> Result<EnqueueResult, DaemonError> {
    let scope_text = scope.to_string();
    let abandon_paths = paths.clone();

    queue.enqueue_mutating_with_abandon(
        paths,
        scope,
        || start_reconciliation_job(paths, &scope_text),
        move |job_id| {
            let _result = abandon_reconciliation_job(&abandon_paths, job_id);
        },
    )
}

fn enqueue_startup_reconciliation_job(
    paths: &PvPaths,
    queue: &ReconciliationQueue,
) -> Result<EnqueueResult, DaemonError> {
    let abandon_paths = paths.clone();

    queue.enqueue_startup_with_abandon(
        paths,
        || start_reconciliation_job(paths, "system"),
        move |job_id| {
            let _result = abandon_reconciliation_job(&abandon_paths, job_id);
        },
    )
}

async fn run_update_job(
    paths: PvPaths,
    queue: ReconciliationQueue,
    mut transport: DaemonTransport<LocalStream>,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    fallback_shutdown: &watch::Receiver<bool>,
) -> Result<(), DaemonError> {
    let result = match enqueue_update_job(&paths, &queue) {
        Ok(result) => result,
        Err(DaemonError::State(error @ StateError::CoordinationLockHeld { .. })) => {
            write_line(&mut transport, &DaemonResponse::error(error.to_string())).await?;

            return Ok(());
        }
        Err(error) => return Err(error),
    };

    match result {
        EnqueueResult::Queued(queued) => {
            let job_id = queued.job_id().to_string();
            let accepted_result = write_line(
                &mut transport,
                &DaemonResponse::accepted("job accepted", &job_id),
            )
            .await
            .map_err(DaemonError::from);
            let stream_is_open = accepted_result.is_ok();
            let Some((running, stream_is_open)) = wait_for_foreground_turn(
                queued,
                &mut transport,
                stream_is_open,
                FOREGROUND_JOB_HEARTBEAT_INTERVAL,
                Some(fallback_shutdown),
            )
            .await
            else {
                return accepted_result;
            };
            let result = stream_started_update_job(
                paths,
                transport,
                stream_is_open,
                running.job_id(),
                runtime_catalog,
                running.timing(),
                Some(fallback_shutdown),
            )
            .await;

            let result = match result {
                ForegroundReconciliationCompletion::Finalized(result) => {
                    running.finish();
                    result
                }
                ForegroundReconciliationCompletion::Cancelled => {
                    drop(running);
                    ReconciliationJobCompletion::Cancelled
                        .into_result()
                        .map(|_summary| ())
                }
            };

            foreground_reconciliation_result(accepted_result, result)
        }
        EnqueueResult::Coalesced(_job) => {
            write_coalesced_update_response(&mut transport).await?;

            Ok(())
        }
    }
}

async fn write_coalesced_update_response<Stream>(
    transport: &mut DaemonTransport<Stream>,
) -> Result<(), DaemonError>
where
    Stream: AsyncWrite + Unpin,
{
    write_line(
        transport,
        &DaemonResponse::error("update already queued or running"),
    )
    .await?;

    Ok(())
}

fn enqueue_update_job(
    paths: &PvPaths,
    queue: &ReconciliationQueue,
) -> Result<EnqueueResult, DaemonError> {
    let abandon_paths = paths.clone();

    queue.enqueue_system_update_with_abandon(
        paths,
        || start_update_job(paths),
        move |job_id| {
            let _result = abandon_update_job(&abandon_paths, job_id);
        },
    )
}

async fn stream_started_update_job<Stream>(
    paths: PvPaths,
    mut transport: DaemonTransport<Stream>,
    stream_is_open: bool,
    job_id: &str,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    timing: ReconciliationJobTiming,
    fallback_shutdown: Option<&watch::Receiver<bool>>,
) -> ForegroundReconciliationCompletion
where
    Stream: AsyncWrite + Unpin,
{
    let started_stream_result = if stream_is_open {
        async {
            write_line(
                &mut transport,
                &DaemonEvent::JobStarted {
                    job_id,
                    kind: "update",
                    scope: "system",
                },
            )
            .await?;
            write_line(
                &mut transport,
                &DaemonEvent::Log {
                    job_id,
                    message: "Managed Resource update started",
                },
            )
            .await?;

            Ok::<(), DaemonError>(())
        }
        .await
    } else {
        Ok(())
    };

    let (update_result, transport_is_open) = if stream_is_open && started_stream_result.is_ok() {
        let (event_sender, event_receiver) = channel(FOREGROUND_JOB_PROGRESS_BUFFER);
        let (phase_sender, phase_receiver) = watch::channel(Vec::new());
        let progress = DaemonDownloadProgress::new(event_sender, phase_sender);
        let completion = complete_streamed_output_with_heartbeat_and_events(
            &mut transport,
            job_id,
            "Managed Resource update still running",
            FOREGROUND_JOB_HEARTBEAT_INTERVAL,
            complete_update_job_with_progress(
                &paths,
                job_id,
                runtime_catalog,
                progress,
                timing,
                fallback_shutdown,
            ),
            event_receiver,
            phase_receiver,
        )
        .await;

        (completion.result, completion.transport_is_open)
    } else {
        (
            complete_update_job_with_progress(
                &paths,
                job_id,
                runtime_catalog,
                DaemonDownloadProgress::disabled(),
                timing,
                fallback_shutdown,
            )
            .await,
            false,
        )
    };
    if matches!(update_result, ReconciliationJobCompletion::Cancelled) {
        return ForegroundReconciliationCompletion::Cancelled;
    }
    let update_result = update_result.into_result();
    if let Err(error) = started_stream_result {
        return ForegroundReconciliationCompletion::Finalized(Err(error));
    }

    if !stream_is_open || !transport_is_open {
        return ForegroundReconciliationCompletion::Finalized(update_result.map(|_summary| ()));
    }

    match update_result {
        Ok(summary) => {
            let result = write_foreground_terminal_event(
                &mut transport,
                &DaemonEvent::JobCompleted {
                    job_id,
                    summary: &summary,
                },
            )
            .await;
            if let Err(error) = result {
                return ForegroundReconciliationCompletion::Finalized(Err(error));
            }
        }
        Err(error) => {
            let error_message = error.to_string();
            let result = write_foreground_terminal_event(
                &mut transport,
                &DaemonEvent::JobFailed {
                    job_id,
                    error: &error_message,
                },
            )
            .await;
            if let Err(error) = result {
                return ForegroundReconciliationCompletion::Finalized(Err(error));
            }
        }
    }

    ForegroundReconciliationCompletion::Finalized(Ok(()))
}

fn foreground_reconciliation_result(
    accepted_result: Result<(), DaemonError>,
    reconciliation_result: Result<(), DaemonError>,
) -> Result<(), DaemonError> {
    reconciliation_result?;
    accepted_result
}

async fn wait_for_foreground_turn<Stream>(
    queued: QueuedReconciliation,
    transport: &mut DaemonTransport<Stream>,
    mut stream_is_open: bool,
    heartbeat_interval: Duration,
    fallback_shutdown: Option<&watch::Receiver<bool>>,
) -> Option<(RunningReconciliation, bool)>
where
    Stream: AsyncWrite + Unpin,
{
    let job_id = queued.job_id().to_string();
    let wait_for_turn = queued.wait_for_turn();
    tokio::pin!(wait_for_turn);
    let mut fallback_shutdown = fallback_shutdown.cloned();
    let mut heartbeat = interval_at(Instant::now() + heartbeat_interval, heartbeat_interval);
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            biased;
            _ = async {
                if let Some(fallback_shutdown) = fallback_shutdown.as_mut() {
                    wait_for_fallback_shutdown(fallback_shutdown).await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => return None,
            running = &mut wait_for_turn => return Some((running, stream_is_open)),
            _ = heartbeat.tick(), if stream_is_open => {
                let event = DaemonEvent::Log {
                    job_id: &job_id,
                    message: FOREGROUND_JOB_QUEUE_HEARTBEAT,
                };
                if !matches!(
                    timeout(
                        FOREGROUND_JOB_STREAM_WRITE_TIMEOUT,
                        write_line(transport, &event),
                    )
                    .await,
                    Ok(Ok(()))
                ) {
                    stream_is_open = false;
                }
            }
        }
    }
}

#[cfg(test)]
async fn stream_started_reconciliation_job<Stream>(
    paths: PvPaths,
    transport: DaemonTransport<Stream>,
    stream_is_open: bool,
    job_id: &str,
    scope: ReconciliationScope,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    timing: ReconciliationJobTiming,
) -> Result<(), DaemonError>
where
    Stream: AsyncWrite + Unpin,
{
    stream_started_reconciliation_job_with_fallback(
        paths,
        transport,
        stream_is_open,
        job_id,
        scope,
        runtime_catalog,
        ForegroundReconciliationOptions {
            timing,
            fallback_shutdown: None,
        },
    )
    .await
    .into_result()
}

struct ForegroundReconciliationOptions<'a> {
    timing: ReconciliationJobTiming,
    fallback_shutdown: Option<&'a watch::Receiver<bool>>,
}

enum ForegroundReconciliationCompletion {
    Finalized(Result<(), DaemonError>),
    Cancelled,
}

impl ForegroundReconciliationCompletion {
    #[cfg(test)]
    fn into_result(self) -> Result<(), DaemonError> {
        match self {
            Self::Finalized(result) => result,
            Self::Cancelled => ReconciliationJobCompletion::Cancelled
                .into_result()
                .map(|_summary| ()),
        }
    }
}

async fn stream_started_reconciliation_job_with_fallback<Stream>(
    paths: PvPaths,
    mut transport: DaemonTransport<Stream>,
    stream_is_open: bool,
    job_id: &str,
    scope: ReconciliationScope,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    options: ForegroundReconciliationOptions<'_>,
) -> ForegroundReconciliationCompletion
where
    Stream: AsyncWrite + Unpin,
{
    let ForegroundReconciliationOptions {
        timing,
        fallback_shutdown,
    } = options;
    let scope_text = scope.to_string();
    let started_stream_result = if stream_is_open {
        async {
            write_line(
                &mut transport,
                &DaemonEvent::JobStarted {
                    job_id,
                    kind: "reconcile",
                    scope: &scope_text,
                },
            )
            .await?;
            let message = reconciliation_started_message(&scope);
            write_line(&mut transport, &DaemonEvent::Log { job_id, message }).await?;

            Ok::<(), DaemonError>(())
        }
        .await
    } else {
        Ok(())
    };

    let (reconciliation_result, transport_is_open) =
        if stream_is_open && started_stream_result.is_ok() {
            let (event_sender, event_receiver) = channel(FOREGROUND_JOB_PROGRESS_BUFFER);
            let (phase_sender, phase_receiver) = watch::channel(Vec::new());
            let progress = DaemonDownloadProgress::new(event_sender, phase_sender);
            let completion = complete_streamed_output_with_heartbeat_and_events(
                &mut transport,
                job_id,
                "Reconciliation still running",
                FOREGROUND_JOB_HEARTBEAT_INTERVAL,
                complete_reconciliation_job_with_progress_outcome(
                    &paths,
                    job_id,
                    &scope,
                    runtime_catalog,
                    progress,
                    timing,
                    ReconciliationJobOptions {
                        fallback_shutdown,
                        ..ReconciliationJobOptions::default()
                    },
                    None,
                ),
                event_receiver,
                phase_receiver,
            )
            .await;

            (completion.result, completion.transport_is_open)
        } else {
            (
                complete_reconciliation_job_with_progress_outcome(
                    &paths,
                    job_id,
                    &scope,
                    runtime_catalog,
                    DaemonDownloadProgress::disabled(),
                    timing,
                    ReconciliationJobOptions {
                        fallback_shutdown,
                        ..ReconciliationJobOptions::default()
                    },
                    None,
                )
                .await,
                false,
            )
        };
    if matches!(
        reconciliation_result,
        ReconciliationJobCompletion::Cancelled
    ) {
        return ForegroundReconciliationCompletion::Cancelled;
    }
    let reconciliation_result = reconciliation_result.into_result();
    if let Err(error) = started_stream_result {
        return ForegroundReconciliationCompletion::Finalized(Err(error));
    }

    if !stream_is_open || !transport_is_open {
        return ForegroundReconciliationCompletion::Finalized(
            reconciliation_result.map(|_summary| ()),
        );
    }

    let result = match reconciliation_result {
        Ok(summary) => {
            write_foreground_terminal_event(
                &mut transport,
                &DaemonEvent::JobCompleted {
                    job_id,
                    summary: &summary,
                },
            )
            .await
        }
        Err(error) => {
            let error_message = error.to_string();
            write_foreground_terminal_event(
                &mut transport,
                &DaemonEvent::JobFailed {
                    job_id,
                    error: &error_message,
                },
            )
            .await
        }
    };

    ForegroundReconciliationCompletion::Finalized(result)
}

#[cfg(test)]
async fn complete_streamed_job_with_heartbeat<Stream, Completion>(
    transport: &mut DaemonTransport<Stream>,
    job_id: &str,
    heartbeat_message: &'static str,
    heartbeat_interval: Duration,
    completion: Completion,
) -> Result<String, DaemonError>
where
    Stream: AsyncWrite + Unpin,
    Completion: Future<Output = Result<String, DaemonError>>,
{
    let mut heartbeat = interval_at(Instant::now() + heartbeat_interval, heartbeat_interval);
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
    tokio::pin!(completion);

    loop {
        tokio::select! {
            result = &mut completion => return result,
            _ = heartbeat.tick() => {
                let heartbeat_event = DaemonEvent::Log {
                    job_id,
                    message: heartbeat_message,
                };
                let heartbeat_result = write_line(transport, &heartbeat_event);
                tokio::select! {
                    result = &mut completion => return result,
                    _ = timeout(FOREGROUND_JOB_STREAM_WRITE_TIMEOUT, heartbeat_result) => {}
                }
            }
        }
    }
}

#[cfg(test)]
async fn complete_streamed_job_with_heartbeat_and_events<Stream, Completion>(
    transport: &mut DaemonTransport<Stream>,
    job_id: &str,
    heartbeat_message: &'static str,
    heartbeat_interval: Duration,
    completion: Completion,
    events: Receiver<ForegroundJobEvent>,
    phases: watch::Receiver<Vec<ReconciliationPhase>>,
) -> StreamedJobCompletion<Result<String, DaemonError>>
where
    Stream: AsyncWrite + Unpin,
    Completion: Future<Output = Result<String, DaemonError>>,
{
    complete_streamed_output_with_heartbeat_and_events(
        transport,
        job_id,
        heartbeat_message,
        heartbeat_interval,
        completion,
        events,
        phases,
    )
    .await
}

async fn complete_streamed_output_with_heartbeat_and_events<Stream, Completion, Output>(
    transport: &mut DaemonTransport<Stream>,
    job_id: &str,
    heartbeat_message: &'static str,
    heartbeat_interval: Duration,
    completion: Completion,
    mut events: Receiver<ForegroundJobEvent>,
    mut phases: watch::Receiver<Vec<ReconciliationPhase>>,
) -> StreamedJobCompletion<Output>
where
    Stream: AsyncWrite + Unpin,
    Completion: Future<Output = Output>,
{
    let mut heartbeat = interval_at(Instant::now() + heartbeat_interval, heartbeat_interval);
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
    tokio::pin!(completion);
    let mut events_open = true;
    let mut phases_open = true;
    let mut next_phase = 0;

    loop {
        tokio::select! {
            biased;
            phase_changed = phases.changed(), if phases_open => {
                if phase_changed.is_err() {
                    phases_open = false;
                    continue;
                }
                if !write_pending_phases(transport, job_id, &mut phases, &mut next_phase).await {
                    phases_open = false;
                }
            }
            result = &mut completion => {
                return finish_streamed_job(
                    transport,
                    job_id,
                    &mut phases,
                    &mut next_phase,
                    result,
                )
                .await;
            }
            event = events.recv(), if events_open => {
                if let Some(event) = event {
                    let write_result = write_foreground_job_event(transport, job_id, event);
                    tokio::select! {
                        result = &mut completion => {
                            return finish_streamed_job(
                                transport,
                                job_id,
                                &mut phases,
                                &mut next_phase,
                                result,
                            )
                            .await;
                        }
                        write_result = timeout(FOREGROUND_JOB_STREAM_WRITE_TIMEOUT, write_result) => {
                            if !matches!(write_result, Ok(Ok(()))) {
                                return StreamedJobCompletion {
                                    result: completion.await,
                                    transport_is_open: false,
                                };
                            }
                        }
                    }
                } else {
                    events_open = false;
                }
            }
            _ = heartbeat.tick() => {
                let heartbeat_event = DaemonEvent::Log {
                    job_id,
                    message: heartbeat_message,
                };
                let heartbeat_result = write_line(transport, &heartbeat_event);
                tokio::select! {
                    result = &mut completion => {
                        return finish_streamed_job(
                            transport,
                            job_id,
                            &mut phases,
                            &mut next_phase,
                            result,
                        )
                        .await;
                    }
                    heartbeat_result = timeout(FOREGROUND_JOB_STREAM_WRITE_TIMEOUT, heartbeat_result) => {
                        if !matches!(heartbeat_result, Ok(Ok(()))) {
                            return StreamedJobCompletion {
                                result: completion.await,
                                transport_is_open: false,
                            };
                        }
                    }
                }
            }
        }
    }
}

async fn finish_streamed_job<Stream, Output>(
    transport: &mut DaemonTransport<Stream>,
    job_id: &str,
    phases: &mut watch::Receiver<Vec<ReconciliationPhase>>,
    next_phase: &mut usize,
    result: Output,
) -> StreamedJobCompletion<Output>
where
    Stream: AsyncWrite + Unpin,
{
    let has_pending_phase = phases.borrow().len() > *next_phase;
    if has_pending_phase {
        write_pending_phases(transport, job_id, phases, next_phase).await;
    }

    StreamedJobCompletion {
        result,
        transport_is_open: true,
    }
}

async fn write_pending_phases<Stream>(
    transport: &mut DaemonTransport<Stream>,
    job_id: &str,
    phases: &mut watch::Receiver<Vec<ReconciliationPhase>>,
    next_phase: &mut usize,
) -> bool
where
    Stream: AsyncWrite + Unpin,
{
    let pending = {
        let phases = phases.borrow_and_update();
        phases.get(*next_phase..).unwrap_or_default().to_vec()
    };
    *next_phase += pending.len();

    for phase in pending {
        let event = DaemonEvent::Progress {
            job_id,
            message: phase.as_str(),
        };
        if !matches!(
            timeout(
                FOREGROUND_JOB_STREAM_WRITE_TIMEOUT,
                write_line(transport, &event),
            )
            .await,
            Ok(Ok(()))
        ) {
            return false;
        }
    }

    true
}

async fn write_foreground_terminal_event<Stream>(
    transport: &mut DaemonTransport<Stream>,
    event: &impl serde::Serialize,
) -> Result<(), DaemonError>
where
    Stream: AsyncWrite + Unpin,
{
    match timeout(
        FOREGROUND_JOB_STREAM_WRITE_TIMEOUT,
        write_line(transport, event),
    )
    .await
    {
        Ok(result) => result.map_err(DaemonError::from),
        Err(_error) => Err(DaemonError::Io(io::Error::new(
            io::ErrorKind::TimedOut,
            "foreground job terminal event write timed out",
        ))),
    }
}

async fn write_foreground_job_event<Stream>(
    transport: &mut DaemonTransport<Stream>,
    job_id: &str,
    event: ForegroundJobEvent,
) -> Result<(), DaemonError>
where
    Stream: AsyncWrite + Unpin,
{
    match event {
        ForegroundJobEvent::DownloadProgress {
            resource,
            track,
            artifact_version,
            downloaded_bytes,
            total_bytes,
        } => {
            write_line(
                transport,
                &DaemonEvent::DownloadProgress {
                    job_id,
                    resource: &resource,
                    track: &track,
                    artifact_version: &artifact_version,
                    downloaded_bytes,
                    total_bytes,
                },
            )
            .await?;
        }
    }

    Ok(())
}

fn start_reconciliation_job(paths: &PvPaths, scope: &str) -> Result<String, DaemonError> {
    let mut database = Database::open(paths)?;
    let job = database.start_job("reconcile", scope)?;
    structured_log::job_started(paths, &job.id, "reconcile", scope);

    Ok(job.id)
}

fn start_update_job(paths: &PvPaths) -> Result<String, DaemonError> {
    let mut database = Database::open(paths)?;
    let job = database.start_job("update", "system")?;
    structured_log::job_started(paths, &job.id, "update", "system");

    Ok(job.id)
}

fn abandon_reconciliation_job(paths: &PvPaths, job_id: &str) -> Result<(), DaemonError> {
    abandon_job(
        paths,
        job_id,
        "reconcile",
        "reconciliation was abandoned before completion",
    )
}

fn abandon_update_job(paths: &PvPaths, job_id: &str) -> Result<(), DaemonError> {
    abandon_job(
        paths,
        job_id,
        "update",
        "Managed Resource update was abandoned before completion",
    )
}

fn abandon_job(
    paths: &PvPaths,
    job_id: &str,
    kind: &str,
    message: &str,
) -> Result<(), DaemonError> {
    let result: Result<(), DaemonError> = (|| {
        let mut database = Database::open(paths)?;
        database.fail_job(job_id, message)?;

        Ok(())
    })();
    if let Err(error) = &result {
        structured_log::job_abandonment_failed(paths, job_id, kind, &error.to_string());
    }

    result
}

#[cfg(test)]
async fn complete_update_job(
    paths: &PvPaths,
    job_id: &str,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
) -> Result<String, DaemonError> {
    complete_update_job_with_progress(
        paths,
        job_id,
        runtime_catalog,
        DaemonDownloadProgress::disabled(),
        ReconciliationJobTiming::immediate(),
        None,
    )
    .await
    .into_result()
}

async fn complete_update_job_with_progress(
    paths: &PvPaths,
    job_id: &str,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    progress: DaemonDownloadProgress,
    timing: ReconciliationJobTiming,
    fallback_shutdown: Option<&watch::Receiver<bool>>,
) -> ReconciliationJobCompletion {
    let phase_log = ReconciliationPhaseLog::new(paths, job_id, "update", "system")
        .with_progress(progress.phase_sender.clone());
    phase_log.completed(
        ReconciliationPhase::Queue,
        "job",
        PhaseOutcome::Succeeded,
        timing.queue_wait(),
        &[],
    );
    let progress = progress.with_phase_log(phase_log.clone());
    let result = complete_update_job_inner(
        paths,
        runtime_catalog,
        progress,
        &phase_log,
        fallback_shutdown,
    )
    .await;
    let result = match result {
        Ok(ReconciliationOutcome::Completed(completed)) => Ok(completed),
        Ok(ReconciliationOutcome::Cancelled) => return ReconciliationJobCompletion::Cancelled,
        Err(error) => Err(error),
    };
    let finalization_timer = phase_log.start(ReconciliationPhase::Finalization, "job");

    let completion = match result {
        Ok(completed) => {
            let mut coverage = vec![JobDiagnosticSubject::UpdateAssessment];
            coverage.extend(completed.coverage.iter().cloned());
            let recording_result =
                Database::open(paths)
                    .map_err(DaemonError::from)
                    .and_then(|mut database| {
                        database
                            .complete_job_with_coverage(job_id, &completed.summary, &coverage)
                            .map_err(DaemonError::from)
                    });
            match recording_result {
                Ok(()) => {
                    structured_log::job_completed(
                        paths,
                        job_id,
                        "update",
                        "system",
                        &completed.summary,
                    );
                    ReconciliationJobCompletion::Succeeded(completed.summary)
                }
                Err(error) => {
                    structured_log::job_completion_recording_failed(
                        paths,
                        job_id,
                        "update",
                        "system",
                        &completed.summary,
                        &error.to_string(),
                    );
                    ReconciliationJobCompletion::Failed {
                        error: Box::new(error),
                        recording_error: None,
                    }
                }
            }
        }
        Err(failure) => {
            let error_message = failure.error.to_string();
            let recording_error = Database::open(paths)
                .map_err(DaemonError::from)
                .and_then(|mut database| {
                    database
                        .fail_job_with_subject(job_id, &error_message, &failure.subject)
                        .map_err(DaemonError::from)
                })
                .err()
                .map(Box::new);
            if let Some(recording_error) = &recording_error {
                structured_log::job_failure_recording_failed(
                    paths,
                    job_id,
                    "update",
                    "system",
                    &error_message,
                    &recording_error.to_string(),
                );
            } else {
                structured_log::job_failed(paths, job_id, "update", "system", &error_message);
            }
            ReconciliationJobCompletion::Failed {
                error: failure.error,
                recording_error,
            }
        }
    };
    finalization_timer.finish(PhaseOutcome::from_succeeded(completion.is_succeeded()), &[]);

    completion
}

async fn complete_update_job_inner(
    paths: &PvPaths,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    progress: DaemonDownloadProgress,
    phase_log: &ReconciliationPhaseLog,
    fallback_shutdown: Option<&watch::Receiver<bool>>,
) -> Result<ReconciliationOutcome<CompletedUpdateJob>, FailedUpdateJob> {
    if fallback_shutdown_requested(fallback_shutdown) {
        return Ok(ReconciliationOutcome::Cancelled);
    }
    let report = if runtime_catalog.is_none() {
        let update_paths = paths.clone();
        let update_progress = progress.clone();
        tokio::task::spawn_blocking(move || {
            crate::managed_resources::update_installed_with_progress(
                update_paths,
                None,
                &update_progress,
            )
        })
        .await
        .map_err(|error| {
            FailedUpdateJob::new(error.into(), JobDiagnosticSubject::UpdateAssessment)
        })?
    } else {
        crate::managed_resources::update_installed_with_progress(
            paths.clone(),
            runtime_catalog,
            &progress,
        )
    }
    .map_err(|error| FailedUpdateJob::new(error, JobDiagnosticSubject::UpdateAssessment))?;

    let report = match report.into_result() {
        Ok(report) => report,
        Err(update_error) => {
            if fallback_shutdown_requested(fallback_shutdown) {
                return Err(FailedUpdateJob::new(
                    update_error,
                    JobDiagnosticSubject::UpdateAssessment,
                ));
            }
            return Err(reconcile_partial_update_failure(
                paths,
                runtime_catalog,
                progress,
                phase_log,
                update_error,
                fallback_shutdown,
            )
            .await);
        }
    };

    if report.updated_count == 0 {
        return Ok(ReconciliationOutcome::Completed(CompletedUpdateJob {
            summary: unchanged_update_summary(&report),
            coverage: Vec::new(),
        }));
    }
    if fallback_shutdown_requested(fallback_shutdown) {
        return Ok(ReconciliationOutcome::Cancelled);
    }

    let mut failure_subject = JobDiagnosticSubject::SystemReconciliation;
    let reconciliation_result = complete_system_reconciliation_with_progress(
        paths,
        runtime_catalog,
        progress,
        phase_log,
        None,
        fallback_shutdown,
        Some(&mut failure_subject),
    )
    .await;
    let completed = match reconciliation_result {
        Ok(ReconciliationOutcome::Completed(completed)) => completed,
        Ok(ReconciliationOutcome::Cancelled) => return Ok(ReconciliationOutcome::Cancelled),
        Err(error) => {
            return Err(compensate_caddy_update_failure(
                paths,
                &report,
                error,
                failure_subject,
                phase_log,
                fallback_shutdown,
            )
            .await);
        }
    };

    let summary = format!(
        "updated {} artifact(s); reconciled: {}",
        report.updated_count, completed.summary,
    );

    Ok(ReconciliationOutcome::Completed(CompletedUpdateJob {
        summary,
        coverage: completed.coverage,
    }))
}

async fn compensate_caddy_update_failure(
    paths: &PvPaths,
    report: &ManagedResourceUpdateReport,
    original_error: DaemonError,
    subject: JobDiagnosticSubject,
    phase_log: &ReconciliationPhaseLog,
    fallback_shutdown: Option<&watch::Receiver<bool>>,
) -> FailedUpdateJob {
    match report.rollback_caddy(paths) {
        Ok(true) if fallback_shutdown_requested(fallback_shutdown) => {
            FailedUpdateJob::new(original_error, subject)
        }
        Ok(true) => match fallback_shutdown {
            Some(fallback_shutdown) => {
                match reconcile_gateway_runtimes_with_phase_log_and_fallback_shutdown(
                    paths,
                    phase_log,
                    fallback_shutdown,
                )
                .await
                {
                    Ok(ReconciliationOutcome::Completed(_) | ReconciliationOutcome::Cancelled) => {
                        FailedUpdateJob::new(
                            original_error,
                            if subject == JobDiagnosticSubject::GatewayRuntime {
                                JobDiagnosticSubject::UpdateAssessment
                            } else {
                                subject
                            },
                        )
                    }
                    Err(recovery_error) => FailedUpdateJob::new(
                        caddy_compensation_error(original_error, recovery_error),
                        subject,
                    ),
                }
            }
            None => match reconcile_gateway_runtimes(paths).await {
                Ok(_summary) => FailedUpdateJob::new(
                    original_error,
                    if subject == JobDiagnosticSubject::GatewayRuntime {
                        JobDiagnosticSubject::UpdateAssessment
                    } else {
                        subject
                    },
                ),
                Err(recovery_error) => FailedUpdateJob::new(
                    caddy_compensation_error(original_error, recovery_error),
                    subject,
                ),
            },
        },
        Ok(false) => FailedUpdateJob::new(original_error, subject),
        Err(rollback_error) => FailedUpdateJob::new(
            caddy_compensation_error(original_error, rollback_error),
            subject,
        ),
    }
}

async fn reconcile_partial_update_failure(
    paths: &PvPaths,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    progress: DaemonDownloadProgress,
    phase_log: &ReconciliationPhaseLog,
    update_error: DaemonError,
    fallback_shutdown: Option<&watch::Receiver<bool>>,
) -> FailedUpdateJob {
    let reconciliation_result = complete_system_reconciliation_with_progress(
        paths,
        runtime_catalog,
        progress,
        phase_log,
        None,
        fallback_shutdown,
        None,
    )
    .await;
    match reconciliation_result {
        Ok(ReconciliationOutcome::Completed(_) | ReconciliationOutcome::Cancelled) => {
            FailedUpdateJob::new(update_error, JobDiagnosticSubject::UpdateAssessment)
        }
        Err(reconciliation_error) => FailedUpdateJob::new(
            partial_update_reconciliation_error(update_error, reconciliation_error),
            JobDiagnosticSubject::UpdateAssessment,
        ),
    }
}

fn partial_update_reconciliation_error(
    update_error: DaemonError,
    reconciliation_error: DaemonError,
) -> DaemonError {
    DaemonError::PartialUpdateReconciliationFailed {
        source: Box::new(update_error),
        reconciliation: Box::new(reconciliation_error),
    }
}

fn caddy_compensation_error(
    original_error: DaemonError,
    compensation_error: DaemonError,
) -> DaemonError {
    DaemonError::CaddyUpdateCompensationFailed {
        source: Box::new(original_error),
        compensation: Box::new(compensation_error),
    }
}

fn unchanged_update_summary(report: &ManagedResourceUpdateReport) -> String {
    if report.installed_count == 0 {
        "none installed".to_string()
    } else {
        "current".to_string()
    }
}

/// Projects with recorded backing-resource demands must not render environments
/// from changed or failed runtimes. Drifted demands and changed PHP identities are
/// refused outright; unready demands are refused once they are established through
/// active allocations or provisioned track env contexts. The track a scope reconciles
/// is left to its Resources phase, which repairs those allocations before the staged
/// apply and skips the Project when the repair fails. Projects without any recorded
/// demand are tolerated until the Resources phase provisions them. Only the given
/// Projects are examined, so a scope never records failures for Projects it does
/// not apply.
fn unready_established_resource_projects(
    paths: &PvPaths,
    projects: &[ProjectRecord],
    reconciled_resource: &str,
    reconciled_track: &str,
) -> Result<BTreeMap<String, DaemonError>, DaemonError> {
    let database = Database::open(paths)?;
    let tracks = database.managed_resource_tracks()?;
    let observations = database.runtime_observed_states()?;
    let mut failures = BTreeMap::new();
    for project in projects {
        let demands = database.project_managed_resources(&project.id)?;
        if demands.is_empty() {
            continue;
        }
        if !crate::project_env::project_resource_demands_match_recorded(paths, &demands, project) {
            failures.insert(
                project.id.clone(),
                DaemonError::ProjectEnvDependenciesNotApplied {
                    project_id: project.id.clone(),
                    reason: "serving mode, hostnames, resource tracks, or allocation identities differ from their last applied state"
                        .to_owned(),
                },
            );
            continue;
        }
        if !crate::project_env::project_php_identity_matches_applied(paths, &database, project) {
            failures.insert(
                project.id.clone(),
                DaemonError::ProjectEnvDependenciesNotApplied {
                    project_id: project.id.clone(),
                    reason: "PHP track or extensions differ from their last applied state"
                        .to_owned(),
                },
            );
            continue;
        }
        for demand in &demands {
            let allocations = database.resource_allocations(&project.id, &demand.resource_name)?;
            let has_env_context = tracks.iter().any(|track| {
                track.resource_name == demand.resource_name
                    && track.track == demand.track
                    && !track.env.is_empty()
            });
            let has_active_allocations = allocations
                .iter()
                .any(|allocation| allocation.status != ResourceAllocationStatus::Inactive);
            if !has_active_allocations && !has_env_context {
                continue;
            }
            // A running shared runtime cannot vouch for a Project-specific allocation that is
            // still Desired or Failed, so unready allocations decide before runtime observations.
            // Demand for the (resource, track) this scope reconciles is exempt: the scope's
            // Resources phase repairs those allocations first and, when they are still unready,
            // records a Project failure that skips the Project from the staged apply.
            let reconciled_by_this_scope =
                demand.resource_name == reconciled_resource && demand.track == reconciled_track;
            if !reconciled_by_this_scope
                && let Some(allocation) = allocations.iter().find(|allocation| {
                    !matches!(
                        allocation.status,
                        ResourceAllocationStatus::Ready | ResourceAllocationStatus::Inactive
                    )
                })
            {
                failures.insert(
                    project.id.clone(),
                    config::ConfigError::MissingAllocationEnvContext {
                        resource: demand.resource_name.clone(),
                        allocation: allocation.allocation_name.clone(),
                    }
                    .into(),
                );
                break;
            }
            let observed = observations.iter().find(|observed| {
                matches!(
                    &observed.subject,
                    RuntimeSubject::Resource { name, track }
                        if name == &demand.resource_name && track == &demand.track
                )
            });
            let failure = match observed {
                None => Some(DaemonError::ProjectEnvDependenciesNotApplied {
                    project_id: project.id.clone(),
                    reason: format!(
                        "required resource {} track {} has no observed state",
                        demand.resource_name, demand.track
                    ),
                }),
                Some(observed) => match observed.status {
                    RuntimeObservedStatus::Running => None,
                    failed => {
                        let status = match failed {
                            RuntimeObservedStatus::Failed => "failed",
                            RuntimeObservedStatus::Degraded => "degraded",
                            RuntimeObservedStatus::Stopped => "stopped",
                            RuntimeObservedStatus::Pending => "pending",
                            RuntimeObservedStatus::Running => "running",
                        };
                        Some(DaemonError::ProjectEnvDependenciesNotApplied {
                            project_id: project.id.clone(),
                            reason: format!(
                                "required resource {} track {} is {status}: {}",
                                demand.resource_name,
                                demand.track,
                                observed
                                    .message
                                    .as_deref()
                                    .unwrap_or("no diagnostic recorded")
                            ),
                        })
                    }
                },
            };
            if let Some(error) = failure {
                failures.insert(project.id.clone(), error);
                break;
            }
        }
    }
    Ok(failures)
}

#[cfg(test)]
async fn complete_managed_resource_reconciliation_with_progress(
    paths: &PvPaths,
    name: &crate::reconciliation::ReconciliationScopeComponent,
    track: &crate::reconciliation::ReconciliationScopeComponent,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    progress: DaemonDownloadProgress,
    phase_log: &ReconciliationPhaseLog,
) -> Result<CompletedReconciliationJob, DaemonError> {
    complete_managed_resource_reconciliation_with_progress_and_fallback_shutdown(
        paths,
        name,
        track,
        runtime_catalog,
        progress,
        phase_log,
        None,
    )
    .await?
    .into_completed()
}

async fn complete_managed_resource_reconciliation_with_progress_and_fallback_shutdown(
    paths: &PvPaths,
    name: &crate::reconciliation::ReconciliationScopeComponent,
    track: &crate::reconciliation::ReconciliationScopeComponent,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    progress: DaemonDownloadProgress,
    phase_log: &ReconciliationPhaseLog,
    fallback_shutdown: Option<&watch::Receiver<bool>>,
) -> Result<ReconciliationOutcome<CompletedReconciliationJob>, DaemonError> {
    if fallback_shutdown_requested(fallback_shutdown) {
        return Ok(ReconciliationOutcome::Cancelled);
    }

    let dependent_projects = Database::open(paths)?
        .projects_demanding_managed_resource_track(name.as_str(), track.as_str())?;
    let strict_failures = unready_established_resource_projects(
        paths,
        &dependent_projects,
        name.as_str(),
        track.as_str(),
    )?;
    let mut skip_projects: BTreeSet<String> = BTreeSet::new();
    if !strict_failures.is_empty() {
        let mut database = Database::open(paths)?;
        for (project_id, error) in &strict_failures {
            // Keep an existing failure cause rather than replacing it with this recheck's refusal.
            let has_existing_failure = database
                .project_env_observed_state(project_id)?
                .is_some_and(|observed| observed.status == ProjectEnvObservedStatus::Failed);
            if !has_existing_failure {
                record_project_env_failure(&mut database, project_id, &error.to_string())?;
            }
            skip_projects.insert(project_id.clone());
        }
    }
    if fallback_shutdown_requested(fallback_shutdown) {
        return cancel_or_preserve_reconciliation_errors(strict_failures.into_values());
    }

    let record_timer = phase_log.start(ReconciliationPhase::ProjectApply, "linked_projects");
    let record_result = reconcile_system_projects_with_progress(
        paths,
        runtime_catalog,
        &BTreeSet::new(),
        &BTreeMap::new(),
        &progress,
        ProjectApplyStage::RecordRequirements,
        &linked_projects(paths)?,
        &skip_projects,
        fallback_shutdown,
    )
    .await;
    finish_project_phase(record_timer, &record_result);
    let record_failures = record_result?.failures;
    if fallback_shutdown_requested(fallback_shutdown) {
        return cancel_or_preserve_reconciliation_errors(
            strict_failures.into_values().chain(record_failures),
        );
    }

    let resources_timer = phase_log.start(
        ReconciliationPhase::Resources,
        format!("{}:{}", name.as_str(), track.as_str()),
    );
    let resources_result = reconcile_persisted_resource_track_for_projects_with_progress(
        paths,
        name.as_str(),
        track.as_str(),
        runtime_catalog,
        &dependent_projects,
        progress.clone().suppressing_operation_phases(),
        fallback_shutdown,
    )
    .await;
    resources_timer.finish(
        match resources_result {
            Ok((false, _)) => PhaseOutcome::Skipped,
            Ok((true, _)) => PhaseOutcome::Succeeded,
            Err(_) => PhaseOutcome::Failed,
        },
        &[("resource_count", 1)],
    );
    let (_, resource_failures) = resources_result?;
    if !resource_failures.is_empty() {
        let mut database = Database::open(paths)?;
        for (project_id, error) in &resource_failures {
            record_project_env_failure(&mut database, project_id, &error.to_string())?;
            skip_projects.insert(project_id.clone());
        }
    }
    if fallback_shutdown_requested(fallback_shutdown) {
        return cancel_or_preserve_reconciliation_errors(
            strict_failures
                .into_values()
                .chain(resource_failures.into_values())
                .chain(record_failures),
        );
    }

    let project_timer = phase_log.start(ReconciliationPhase::ProjectApply, "linked_projects");
    // The staged apply covers established dependents only: Projects that never
    // demanded this track have no persisted state for it, and recording their
    // requirements above must not pull them into this scope's apply.
    let project_result = reconcile_system_projects_with_progress(
        paths,
        runtime_catalog,
        &BTreeSet::new(),
        &BTreeMap::new(),
        &progress,
        ProjectApplyStage::CompleteStagedApply,
        &dependent_projects,
        &skip_projects,
        fallback_shutdown,
    )
    .await;
    finish_project_phase(project_timer, &project_result);
    let mut project_report = project_result?;
    for (project_id, error) in strict_failures {
        let project_label = dependent_projects
            .iter()
            .find(|project| project.id == project_id)
            .map(|project| {
                project
                    .primary_hostname
                    .as_deref()
                    .unwrap_or(&project.slug)
                    .to_owned()
            })
            .unwrap_or(project_id.clone());
        project_report
            .failures
            .push(DaemonError::ProjectReconciliation {
                project_label,
                source: Box::new(error),
            });
    }
    if fallback_shutdown_requested(fallback_shutdown) {
        let failures = record_failures
            .into_iter()
            .chain(resource_failures.into_values())
            .chain(project_report.failures)
            .collect::<Vec<_>>();
        return cancel_or_preserve_reconciliation_errors(failures);
    }
    let summary =
        managed_resource_reconciliation_summary(name.as_str(), track.as_str(), &project_report);
    let mut coverage = vec![JobDiagnosticSubject::Resource {
        name: name.as_str().to_owned(),
        track: track.as_str().to_owned(),
    }];
    coverage.extend(project_report.successful_project_coverage());

    Ok(ReconciliationOutcome::Completed(
        CompletedReconciliationJob { summary, coverage },
    ))
}

/// Applies persisted environments after a resource change. Only tests exercise this path
/// directly now; job-orchestrated scopes apply through the staged flows instead.
#[cfg(test)]
fn reconcile_persisted_project_envs(
    paths: &PvPaths,
    projects: &[ProjectRecord],
    mut resource_failures: BTreeMap<String, DaemonError>,
) -> Result<SystemProjectReconciliationReport, DaemonError> {
    use crate::project_env::reconcile_project_env_from_persisted_state;
    let mut report = SystemProjectReconciliationReport {
        total: projects.len(),
        ..SystemProjectReconciliationReport::default()
    };

    for project in projects {
        let project_label = project.primary_hostname.as_deref().unwrap_or(&project.slug);
        if let Some(error) = resource_failures.remove(&project.id) {
            let error_message = error.to_string();
            let recording_result =
                Database::open(paths)
                    .map_err(DaemonError::from)
                    .and_then(|mut database| {
                        record_project_env_failure(&mut database, &project.id, &error_message)
                    });
            if let Err(recording) = recording_result {
                return Err(DaemonError::ProjectAllocationFailureRecordingFailed {
                    project_id: project.id.clone(),
                    allocation: Box::new(error),
                    recording: Box::new(recording),
                });
            }
            report.failures.push(DaemonError::ProjectReconciliation {
                project_label: project_label.to_owned(),
                source: Box::new(error),
            });
            continue;
        }
        let mut database = Database::open(paths)?;
        match reconcile_project_env_from_persisted_state(paths, &mut database, &project.id) {
            Ok(summary) => {
                report.succeeded += 1;
                report.successful_project_ids.push(project.id.clone());
                report.summaries.push(summary.to_owned());
            }
            Err(error @ DaemonError::ProjectEnvFailureRecordingFailed { .. }) => {
                return Err(error);
            }
            Err(error) => {
                report.failures.push(DaemonError::ProjectReconciliation {
                    project_label: project_label.to_owned(),
                    source: Box::new(error),
                });
            }
        }
    }

    Ok(report)
}

#[cfg(test)]
async fn complete_reconciliation_job(
    paths: &PvPaths,
    job_id: &str,
    scope: &ReconciliationScope,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    timing: ReconciliationJobTiming,
    pf_routing_state: Option<GatewayPfRoutingState>,
) -> Result<String, DaemonError> {
    complete_reconciliation_job_with_progress(
        paths,
        job_id,
        scope,
        runtime_catalog,
        DaemonDownloadProgress::disabled(),
        timing,
        pf_routing_state,
    )
    .await
}

#[cfg(test)]
async fn complete_reconciliation_job_with_progress(
    paths: &PvPaths,
    job_id: &str,
    scope: &ReconciliationScope,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    progress: DaemonDownloadProgress,
    timing: ReconciliationJobTiming,
    pf_routing_state: Option<GatewayPfRoutingState>,
) -> Result<String, DaemonError> {
    complete_reconciliation_job_with_progress_outcome(
        paths,
        job_id,
        scope,
        runtime_catalog,
        progress,
        timing,
        ReconciliationJobOptions {
            pf_routing_state,
            ..ReconciliationJobOptions::default()
        },
        None,
    )
    .await
    .into_result()
}

enum ReconciliationJobCompletion {
    Succeeded(String),
    Cancelled,
    Failed {
        error: Box<DaemonError>,
        recording_error: Option<Box<DaemonError>>,
    },
}

impl ReconciliationJobCompletion {
    fn into_result(self) -> Result<String, DaemonError> {
        match self {
            Self::Succeeded(summary) => Ok(summary),
            Self::Cancelled => Err(DaemonError::UnexpectedProtocolResponse {
                reason: "foreground reconciliation was cancelled without a shutdown signal"
                    .to_owned(),
            }),
            Self::Failed {
                error,
                recording_error,
            } => Err(*recording_error.unwrap_or(error)),
        }
    }

    fn is_succeeded(&self) -> bool {
        matches!(self, Self::Succeeded(_))
    }
}

#[derive(Default)]
struct ReconciliationJobOptions<'a> {
    discard_obsolete_project: bool,
    pf_routing_state: Option<GatewayPfRoutingState>,
    fallback_shutdown: Option<&'a watch::Receiver<bool>>,
}

#[expect(
    clippy::too_many_arguments,
    reason = "the startup-only shutdown signal and cloneable test fallback have distinct scopes"
)]
async fn complete_reconciliation_job_with_progress_outcome(
    paths: &PvPaths,
    job_id: &str,
    scope: &ReconciliationScope,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    progress: DaemonDownloadProgress,
    timing: ReconciliationJobTiming,
    options: ReconciliationJobOptions<'_>,
    shutdown: Option<&oneshot::Receiver<()>>,
) -> ReconciliationJobCompletion {
    let discard_obsolete_project = options.discard_obsolete_project;
    let pf_routing_state = options.pf_routing_state;
    let fallback_shutdown = options.fallback_shutdown;
    let scope_text = scope.to_string();
    let phase_log = ReconciliationPhaseLog::new(paths, job_id, "reconcile", &scope_text)
        .with_progress(progress.phase_sender.clone());
    phase_log.completed(
        ReconciliationPhase::Queue,
        "job",
        PhaseOutcome::Succeeded,
        timing.queue_wait(),
        &[],
    );
    let obsolete_project = match (discard_obsolete_project, scope) {
        (true, ReconciliationScope::Project { id }) => {
            project_exists(paths, id.as_str()).map(|exists| !exists)
        }
        _ => Ok(false),
    };
    let progress = progress.with_phase_log(phase_log.clone());
    let effective_scope = scope.effective();
    let mut failure_subject = None;
    let result = match obsolete_project {
        Err(error) => Err(error),
        Ok(true) => {
            if let ReconciliationScope::Project { id } = scope {
                phase_log.report_progress(ReconciliationPhase::ProjectApply);
                phase_log.completed(
                    ReconciliationPhase::ProjectApply,
                    id.as_str(),
                    PhaseOutcome::Skipped,
                    Duration::ZERO,
                    &[],
                );
            }
            Ok(ReconciliationOutcome::Completed(
                CompletedReconciliationJob {
                    summary: "Project was removed before background reconciliation; skipped"
                        .to_owned(),
                    coverage: Vec::new(),
                },
            ))
        }
        Ok(false) => match &effective_scope {
            ReconciliationScope::System => {
                complete_system_reconciliation_with_progress(
                    paths,
                    runtime_catalog,
                    progress,
                    &phase_log,
                    shutdown,
                    fallback_shutdown,
                    None,
                )
                .await
            }
            ReconciliationScope::Resource { name, .. }
                if gateway_runtime_resource(name.as_str()) =>
            {
                complete_gateway_reconciliation(paths, &phase_log, fallback_shutdown).await
            }
            ReconciliationScope::Resource { name, track } => {
                complete_managed_resource_reconciliation_with_progress_and_fallback_shutdown(
                    paths,
                    name,
                    track,
                    runtime_catalog,
                    progress,
                    &phase_log,
                    fallback_shutdown,
                )
                .await
            }
            ReconciliationScope::Project { id } => {
                complete_project_reconciliation_with_progress_and_fallback(
                    paths,
                    id,
                    runtime_catalog,
                    progress,
                    &phase_log,
                    &mut failure_subject,
                    ReconciliationJobOptions {
                        pf_routing_state,
                        fallback_shutdown,
                        ..ReconciliationJobOptions::default()
                    },
                )
                .await
            }
        },
    };

    let result = match result {
        Ok(ReconciliationOutcome::Completed(completed)) => Ok(completed),
        Ok(ReconciliationOutcome::Cancelled) => return ReconciliationJobCompletion::Cancelled,
        Err(error) => Err(error),
    };

    let coverage_count = result
        .as_ref()
        .map_or(0, |completed| completed.coverage.len());
    let finalization_timer = phase_log.start(ReconciliationPhase::Finalization, "job");
    let final_result = match result {
        Ok(completed) => {
            let completion_result = (|| {
                let mut database = Database::open(paths)?;
                database.complete_job_with_coverage(
                    job_id,
                    &completed.summary,
                    &completed.coverage,
                )?;

                Ok::<(), DaemonError>(())
            })();
            match completion_result {
                Ok(()) => {
                    structured_log::job_completed(
                        paths,
                        job_id,
                        "reconcile",
                        &scope_text,
                        &completed.summary,
                    );

                    ReconciliationJobCompletion::Succeeded(completed.summary)
                }
                Err(error) => finish_failed_reconciliation_job(
                    paths,
                    job_id,
                    &scope_text,
                    error,
                    failure_subject.as_ref(),
                ),
            }
        }
        Err(error) => finish_failed_reconciliation_job(
            paths,
            job_id,
            &scope_text,
            error,
            failure_subject.as_ref(),
        ),
    };
    finalization_timer.finish(
        PhaseOutcome::from_succeeded(final_result.is_succeeded()),
        &[
            (
                "total_execution_ms",
                duration_milliseconds(timing.execution_elapsed()),
            ),
            ("coverage_count", usize_as_u64(coverage_count)),
        ],
    );

    final_result
}

fn finish_failed_reconciliation_job(
    paths: &PvPaths,
    job_id: &str,
    scope: &str,
    error: DaemonError,
    subject: Option<&JobDiagnosticSubject>,
) -> ReconciliationJobCompletion {
    let recording_error = fail_reconciliation_job(paths, job_id, scope, &error, subject)
        .err()
        .map(Box::new);

    ReconciliationJobCompletion::Failed {
        error: Box::new(error),
        recording_error,
    }
}

fn fail_reconciliation_job(
    paths: &PvPaths,
    job_id: &str,
    scope: &str,
    error: &DaemonError,
    subject: Option<&JobDiagnosticSubject>,
) -> Result<(), DaemonError> {
    let error_message = error.to_string();
    let mut database = Database::open(paths)?;
    if let Some(subject) = subject {
        database.fail_job_with_subject(job_id, &error_message, subject)?;
    } else {
        database.fail_job(job_id, &error_message)?;
    }
    structured_log::job_failed(paths, job_id, "reconcile", scope, &error_message);

    Ok(())
}

async fn complete_gateway_reconciliation(
    paths: &PvPaths,
    phase_log: &ReconciliationPhaseLog,
    fallback_shutdown: Option<&watch::Receiver<bool>>,
) -> Result<ReconciliationOutcome<CompletedReconciliationJob>, DaemonError> {
    let summary = match fallback_shutdown {
        Some(fallback_shutdown) => {
            match reconcile_gateway_runtimes_with_phase_log_and_fallback_shutdown(
                paths,
                phase_log,
                fallback_shutdown,
            )
            .await?
            {
                ReconciliationOutcome::Completed(summary) => summary,
                ReconciliationOutcome::Cancelled => return Ok(ReconciliationOutcome::Cancelled),
            }
        }
        None => reconcile_gateway_runtimes_with_phase_log(paths, phase_log).await?,
    };

    Ok(ReconciliationOutcome::Completed(
        CompletedReconciliationJob {
            summary,
            coverage: vec![JobDiagnosticSubject::GatewayRuntime],
        },
    ))
}

async fn complete_system_reconciliation_with_progress(
    paths: &PvPaths,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    progress: DaemonDownloadProgress,
    phase_log: &ReconciliationPhaseLog,
    shutdown: Option<&oneshot::Receiver<()>>,
    fallback_shutdown: Option<&watch::Receiver<bool>>,
    mut failure_subject: Option<&mut JobDiagnosticSubject>,
) -> Result<ReconciliationOutcome<CompletedReconciliationJob>, DaemonError> {
    if fallback_shutdown_requested(fallback_shutdown) {
        return Ok(ReconciliationOutcome::Cancelled);
    }

    let discovery_timer = phase_log.start(ReconciliationPhase::DemandDiscovery, "linked_projects");
    let discovery_result = discover_system_project_demand(paths);
    finish_demand_discovery_phase(discovery_timer, &discovery_result);
    let demand = discovery_result?;
    if fallback_shutdown_requested(fallback_shutdown) {
        return Ok(ReconciliationOutcome::Cancelled);
    }

    let resources_timer = phase_log.start(ReconciliationPhase::Resources, "desired_resources");
    let mut resources_progress = progress.clone().suppressing_operation_phases();
    let mut resources_result = reconcile_system_resources_with_runtime_catalog_and_progress(
        paths,
        runtime_catalog,
        &demand.resource_tracks,
        resources_progress.clone(),
    )
    .await;
    resources_timer.finish(PhaseOutcome::from_succeeded(resources_result.is_ok()), &[]);
    if fallback_shutdown_requested(fallback_shutdown) {
        return resources_result.map(|()| ReconciliationOutcome::Cancelled);
    }
    // Retry the install once before applying the Projects. No Project Apply downloads, so a
    // retry after one could never recover the apply that needed the artifact. The retry is its
    // own timed phase and suppresses nested operation records, so the log stays coherent. If it
    // still fails, the later read-only check decides whether current applied demand still needs it.
    // Skip the retry when shutdown was already requested: a second blocking download would
    // hold the shutdown drain with no one left to consume its result.
    let shutdown_requested = shutdown.is_some_and(|shutdown| !shutdown.is_empty())
        || fallback_shutdown_requested(fallback_shutdown);
    if resources_result.is_err() && !shutdown_requested {
        let retry_timer = phase_log.start(ReconciliationPhase::Resources, "desired_resources");
        resources_progress = progress
            .retrying_manifest_snapshot()
            .suppressing_operation_phases();
        resources_result = reconcile_system_resources_with_runtime_catalog_and_progress(
            paths,
            runtime_catalog,
            &demand.resource_tracks,
            resources_progress.clone(),
        )
        .await;
        retry_timer.finish(PhaseOutcome::from_succeeded(resources_result.is_ok()), &[]);
        if fallback_shutdown_requested(fallback_shutdown) {
            return resources_result.map(|()| ReconciliationOutcome::Cancelled);
        }
    }
    let mut resource_tracks = demand.resource_tracks;
    let mut project_demands = demand.project_demands;
    let has_late_resource_demand =
        discover_late_system_project_demand(paths, &mut resource_tracks, &mut project_demands)?;
    if fallback_shutdown_requested(fallback_shutdown) {
        return resources_result.map(|()| ReconciliationOutcome::Cancelled);
    }
    if has_late_resource_demand {
        let late_timer = phase_log.start(ReconciliationPhase::Resources, "desired_resources");
        resources_result = reconcile_system_resources_with_runtime_catalog_and_progress(
            paths,
            runtime_catalog,
            &resource_tracks,
            resources_progress,
        )
        .await;
        late_timer.finish(PhaseOutcome::from_succeeded(resources_result.is_ok()), &[]);
        if fallback_shutdown_requested(fallback_shutdown) {
            return resources_result.map(|()| ReconciliationOutcome::Cancelled);
        }
    }
    let projects = linked_projects(paths)?;
    if fallback_shutdown_requested(fallback_shutdown) {
        return resources_result.map(|()| ReconciliationOutcome::Cancelled);
    }
    let project_timer = phase_log.start(ReconciliationPhase::ProjectApply, "linked_projects");
    let project_result = reconcile_system_projects_with_progress(
        paths,
        runtime_catalog,
        &resource_tracks,
        &project_demands,
        &progress,
        ProjectApplyStage::CompleteStagedApply,
        &projects,
        &BTreeSet::new(),
        fallback_shutdown,
    )
    .await;
    finish_project_phase(project_timer, &project_result);
    if fallback_shutdown_requested(fallback_shutdown) {
        return cancel_or_preserve_system_reconciliation_errors(
            resources_result,
            project_result,
            Ok(()),
        );
    }
    let cleanup_result = stop_undemanded_system_resource_runtimes(paths, runtime_catalog).await;
    if fallback_shutdown_requested(fallback_shutdown) {
        return cancel_or_preserve_system_reconciliation_errors(
            resources_result,
            project_result,
            cleanup_result,
        );
    }
    if resources_result.is_err()
        || project_result
            .as_ref()
            .is_ok_and(|report| !report.failures.is_empty())
    {
        resources_result = verify_system_resources_after_project_application(
            paths,
            runtime_catalog,
            project_result.as_ref().ok(),
            &progress,
        );
    }
    if fallback_shutdown_requested(fallback_shutdown) {
        return cancel_or_preserve_system_reconciliation_errors(
            resources_result,
            project_result,
            cleanup_result,
        );
    }
    let has_system_failure = resources_result.is_err()
        || project_result
            .as_ref()
            .map_or(true, |report| !report.failures.is_empty())
        || cleanup_result.is_err();
    if let Some(subject) = failure_subject.as_deref_mut() {
        *subject = JobDiagnosticSubject::GatewayRuntime;
    }
    let gateway_result = match fallback_shutdown {
        Some(fallback_shutdown) => {
            reconcile_gateway_runtimes_with_phase_log_and_fallback_shutdown(
                paths,
                phase_log,
                fallback_shutdown,
            )
            .await
        }
        None => reconcile_gateway_runtimes_with_phase_log(paths, phase_log)
            .await
            .map(ReconciliationOutcome::Completed),
    };
    let (project_report, gateway_summary) = match (
        resources_result,
        project_result,
        cleanup_result,
        gateway_result,
    ) {
        (
            Ok(()),
            Ok(project_report),
            Ok(()),
            Ok(ReconciliationOutcome::Completed(gateway_summary)),
        ) => (project_report, gateway_summary),
        (Ok(()), Ok(project_report), Ok(()), Ok(ReconciliationOutcome::Cancelled)) => {
            if !project_report.failures.is_empty()
                && let Some(subject) = failure_subject.as_deref_mut()
            {
                *subject = JobDiagnosticSubject::SystemReconciliation;
            }
            return cancel_or_preserve_system_reconciliation_errors(
                Ok(()),
                Ok(project_report),
                Ok(()),
            );
        }
        (resources_result, project_result, cleanup_result, gateway_result) => {
            if has_system_failure && let Some(subject) = failure_subject.as_deref_mut() {
                *subject = JobDiagnosticSubject::SystemReconciliation;
            }
            let project_failures = match project_result {
                Ok(report) => report.failures,
                Err(error) => vec![error],
            };
            return Err(combined_system_reconciliation_error(
                resources_result
                    .err()
                    .into_iter()
                    .chain(project_failures)
                    .chain(cleanup_result.err())
                    .chain(gateway_result.err())
                    .collect(),
            ));
        }
    };
    let summary = system_reconciliation_summary(&project_report, &gateway_summary);
    if let Some(subject) = failure_subject {
        *subject = JobDiagnosticSubject::SystemReconciliation;
    }
    let coverage = completed_system_reconciliation_coverage(paths, &project_report)?;

    Ok(ReconciliationOutcome::Completed(
        CompletedReconciliationJob { summary, coverage },
    ))
}

fn cancel_or_preserve_system_reconciliation_errors(
    resources_result: Result<(), DaemonError>,
    project_result: Result<SystemProjectReconciliationReport, DaemonError>,
    cleanup_result: Result<(), DaemonError>,
) -> Result<ReconciliationOutcome<CompletedReconciliationJob>, DaemonError> {
    let project_failures = match project_result {
        Ok(report) => report.failures,
        Err(error) => vec![error],
    };
    let failures = resources_result
        .err()
        .into_iter()
        .chain(project_failures)
        .chain(cleanup_result.err());
    cancel_or_preserve_reconciliation_errors(failures)
}

fn cancel_or_preserve_reconciliation_errors<T>(
    failures: impl IntoIterator<Item = DaemonError>,
) -> Result<ReconciliationOutcome<T>, DaemonError> {
    let failures = failures.into_iter().collect::<Vec<_>>();
    if failures.is_empty() {
        Ok(ReconciliationOutcome::Cancelled)
    } else {
        Err(combined_system_reconciliation_error(failures))
    }
}

#[cfg(test)]
async fn complete_project_reconciliation_with_progress(
    paths: &PvPaths,
    id: &crate::reconciliation::ReconciliationScopeComponent,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    progress: DaemonDownloadProgress,
    phase_log: &ReconciliationPhaseLog,
    pf_routing_state: Option<GatewayPfRoutingState>,
    failure_subject: &mut Option<JobDiagnosticSubject>,
) -> Result<CompletedReconciliationJob, DaemonError> {
    complete_project_reconciliation_with_progress_and_fallback(
        paths,
        id,
        runtime_catalog,
        progress,
        phase_log,
        failure_subject,
        ReconciliationJobOptions {
            pf_routing_state,
            ..ReconciliationJobOptions::default()
        },
    )
    .await?
    .into_completed()
}

async fn complete_project_reconciliation_with_progress_and_fallback(
    paths: &PvPaths,
    id: &crate::reconciliation::ReconciliationScopeComponent,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    progress: DaemonDownloadProgress,
    phase_log: &ReconciliationPhaseLog,
    failure_subject: &mut Option<JobDiagnosticSubject>,
    options: ReconciliationJobOptions<'_>,
) -> Result<ReconciliationOutcome<CompletedReconciliationJob>, DaemonError> {
    let pf_routing_state = options.pf_routing_state;
    let fallback_shutdown = options.fallback_shutdown;
    let project_result =
        reconcile_project_env_and_missing_resources_with_progress_and_fallback_shutdown(
            paths,
            id.as_str(),
            runtime_catalog,
            progress.clone(),
            phase_log,
            fallback_shutdown,
        )
        .await;
    let project_env_summary = match project_result {
        Ok(ReconciliationOutcome::Completed(summary)) => summary,
        Ok(ReconciliationOutcome::Cancelled) => return Ok(ReconciliationOutcome::Cancelled),
        Err(project_error) => {
            if fallback_shutdown_requested(fallback_shutdown) {
                return Err(project_error);
            }
            let gateway_result = match fallback_shutdown {
                Some(fallback_shutdown) => {
                    reconcile_gateway_runtimes_with_phase_log_and_fallback_shutdown(
                        paths,
                        phase_log,
                        fallback_shutdown,
                    )
                    .await
                }
                None => reconcile_gateway_runtimes_with_phase_log(paths, phase_log)
                    .await
                    .map(ReconciliationOutcome::Completed),
            };
            return match gateway_result {
                Ok(ReconciliationOutcome::Completed(_) | ReconciliationOutcome::Cancelled) => {
                    Err(project_error)
                }
                Err(gateway_error) => Err(combined_system_reconciliation_error(vec![
                    project_error,
                    gateway_error,
                ])),
            };
        }
    };
    let gateway_outcome = match fallback_shutdown {
        Some(fallback_shutdown) => {
            match reconcile_project_gateway_runtimes_with_phase_log_and_fallback_shutdown(
                paths,
                id.as_str(),
                pf_routing_state,
                phase_log,
                Some(fallback_shutdown),
            )
            .await?
            {
                ReconciliationOutcome::Completed(outcome) => outcome,
                ReconciliationOutcome::Cancelled => return Ok(ReconciliationOutcome::Cancelled),
            }
        }
        None => {
            reconcile_project_gateway_runtimes_with_phase_log(
                paths,
                id.as_str(),
                pf_routing_state,
                phase_log,
            )
            .await?
        }
    };
    let (gateway_summary, gateway_evaluated) = match gateway_outcome {
        ProjectGatewayReconciliationOutcome::Reconciled {
            summary,
            gateway_evaluated,
        } => (summary, gateway_evaluated),
        ProjectGatewayReconciliationOutcome::PromoteSystem => {
            *failure_subject = Some(JobDiagnosticSubject::SystemReconciliation);
            return complete_system_reconciliation_with_progress(
                paths,
                runtime_catalog,
                progress,
                phase_log,
                None,
                fallback_shutdown,
                None,
            )
            .await;
        }
    };
    let summary = if gateway_summary == CADDY_NOT_INSTALLED {
        project_env_summary.as_str().to_string()
    } else {
        format!("{}; {gateway_summary}", project_env_summary.as_str())
    };
    let mut coverage = vec![JobDiagnosticSubject::Project {
        id: id.as_str().to_owned(),
    }];
    if gateway_evaluated {
        coverage.push(JobDiagnosticSubject::GatewayRuntime);
    }

    Ok(ReconciliationOutcome::Completed(
        CompletedReconciliationJob { summary, coverage },
    ))
}

fn finish_project_phase(
    timer: structured_log::PhaseTimer,
    result: &Result<SystemProjectReconciliationReport, DaemonError>,
) {
    let (project_count, succeeded_count, failed_count) =
        result.as_ref().map_or((0, 0, 0), |report| {
            (report.total, report.succeeded, report.failures.len())
        });
    let succeeded = result
        .as_ref()
        .is_ok_and(|report| report.failures.is_empty());
    timer.finish(
        PhaseOutcome::from_succeeded(succeeded),
        &[
            ("project_count", usize_as_u64(project_count)),
            ("succeeded_count", usize_as_u64(succeeded_count)),
            ("failed_count", usize_as_u64(failed_count)),
        ],
    );
}

fn finish_demand_discovery_phase(
    timer: structured_log::PhaseTimer,
    result: &Result<SystemProjectDemandReport, DaemonError>,
) {
    let (outcome, project_count, fallback_count) = match result {
        Ok(report) if report.fallback_count > 0 => (
            PhaseOutcome::Fallback,
            report.project_count,
            report.fallback_count,
        ),
        Ok(report) => (PhaseOutcome::Succeeded, report.project_count, 0),
        Err(_) => (PhaseOutcome::Failed, 0, 0),
    };
    timer.finish(
        outcome,
        &[
            ("project_count", usize_as_u64(project_count)),
            ("fallback_count", usize_as_u64(fallback_count)),
        ],
    );
}

fn duration_milliseconds(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn usize_as_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
async fn reconcile_project_env_and_missing_resources(
    paths: &PvPaths,
    project_id: &str,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
) -> Result<crate::project_env::ProjectEnvReconciliationSummary, DaemonError> {
    reconcile_project_env_and_missing_resources_with_progress_and_fallback_shutdown(
        paths,
        project_id,
        runtime_catalog,
        DaemonDownloadProgress::disabled(),
        &ReconciliationPhaseLog::new(
            paths,
            "test_job",
            "reconcile",
            &format!("project:{project_id}"),
        ),
        None,
    )
    .await?
    .into_completed()
}

async fn reconcile_project_env_and_missing_resources_with_progress_and_fallback_shutdown(
    paths: &PvPaths,
    project_id: &str,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    progress: DaemonDownloadProgress,
    phase_log: &ReconciliationPhaseLog,
    fallback_shutdown: Option<&watch::Receiver<bool>>,
) -> Result<ReconciliationOutcome<crate::project_env::ProjectEnvReconciliationSummary>, DaemonError>
{
    if fallback_shutdown_requested(fallback_shutdown) {
        return Ok(ReconciliationOutcome::Cancelled);
    }

    let record_timer = phase_log.start(ReconciliationPhase::ProjectApply, project_id);
    let record_result = reconcile_project_env_with_runtime_catalog_and_progress_outcome(
        paths,
        project_id,
        runtime_catalog,
        None,
        &BTreeSet::new(),
        progress.clone(),
        ProjectApplyOptions::new(ProjectApplyStage::RecordRequirements, fallback_shutdown),
    )
    .await;
    record_timer.finish(
        match &record_result {
            Ok(ReconciliationOutcome::Completed(_)) => PhaseOutcome::Succeeded,
            Ok(ReconciliationOutcome::Cancelled) => PhaseOutcome::Skipped,
            Err(_) => PhaseOutcome::Failed,
        },
        &[("project_count", 1)],
    );
    let recorded = match record_result? {
        ReconciliationOutcome::Completed(recorded) => recorded,
        ReconciliationOutcome::Cancelled => return Ok(ReconciliationOutcome::Cancelled),
    };
    if fallback_shutdown_requested(fallback_shutdown) {
        return Ok(ReconciliationOutcome::Cancelled);
    }
    let requested_php_extensions = recorded.requested_php_extensions();
    let recorded_tracks = recorded.recorded_tracks().clone();

    let resources_timer = phase_log.start(ReconciliationPhase::Resources, "desired_resources");
    let resources_result = install_project_resources(
        paths,
        &recorded_tracks,
        runtime_catalog,
        &BTreeSet::new(),
        progress.clone(),
        requested_php_extensions,
    )
    .await;
    resources_timer.finish(
        match &resources_result {
            Ok(install) if install.deferred_error.is_some() => PhaseOutcome::Failed,
            Ok(install) if install.installed => PhaseOutcome::Succeeded,
            Ok(_) => PhaseOutcome::Skipped,
            Err(_) => PhaseOutcome::Failed,
        },
        &[],
    );
    let deferred_resources_error = resources_result?.deferred_error;
    if fallback_shutdown_requested(fallback_shutdown) {
        return match deferred_resources_error {
            Some(error) => Err(error),
            None => Ok(ReconciliationOutcome::Cancelled),
        };
    }

    let apply_timer = phase_log.start(ReconciliationPhase::ProjectApply, project_id);
    let apply_result = reconcile_project_env_with_runtime_catalog_and_progress_outcome(
        paths,
        project_id,
        runtime_catalog,
        None,
        &BTreeSet::new(),
        progress,
        ProjectApplyOptions::new(ProjectApplyStage::CompleteStagedApply, fallback_shutdown),
    )
    .await;
    apply_timer.finish(
        match &apply_result {
            Ok(ReconciliationOutcome::Completed(_)) => PhaseOutcome::Succeeded,
            Ok(ReconciliationOutcome::Cancelled) => PhaseOutcome::Skipped,
            Err(_) => PhaseOutcome::Failed,
        },
        &[("project_count", 1)],
    );

    let result = match (apply_result, deferred_resources_error) {
        (Ok(ReconciliationOutcome::Completed(summary)), None) => Ok(summary),
        (Ok(ReconciliationOutcome::Cancelled), None) => {
            return Ok(ReconciliationOutcome::Cancelled);
        }
        (Ok(_), Some(repair_error)) => Err(repair_error),
        (Err(apply_error), None) => Err(apply_error),
        // Both stages failed. The apply is reported as primary because it is the later,
        // Project-scoped failure, and the earlier repair failure is retained rather than
        // dropped.
        (Err(apply_error), Some(repair_error)) => {
            Err(DaemonError::ProjectApplyAfterResourceRepairFailed {
                source: Box::new(apply_error),
                repair: Box::new(repair_error),
            })
        }
    };
    let summary = result?;
    if fallback_shutdown_requested(fallback_shutdown) {
        return Ok(ReconciliationOutcome::Cancelled);
    }

    Ok(ReconciliationOutcome::Completed(summary))
}

/// Installs the Managed Resource tracks the Project declares, then, under the original
/// optional-extension gate, may run the unchanged system-wide repair pass. That pass installs
/// every desired track that is missing, so it can still install and fail on tracks unrelated to
/// this Project.
///
/// A declared-track failure short-circuits, but a repair-pass failure is returned as
/// [`ProjectResourceInstall::deferred_error`] so the caller can still complete the apply that
/// used to run before the repair. `installed` means an install ran, not that an artifact was
/// downloaded.
struct ProjectResourceInstall {
    installed: bool,
    deferred_error: Option<DaemonError>,
}

async fn install_project_resources(
    paths: &PvPaths,
    recorded_tracks: &BTreeSet<DemandedResourceTrack>,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    demanded_tracks: &BTreeSet<DemandedResourceTrack>,
    progress: DaemonDownloadProgress,
    requested_php_extensions: bool,
) -> Result<ProjectResourceInstall, DaemonError> {
    let production_catalog;
    let catalog = match runtime_catalog {
        Some(catalog) => catalog,
        None => {
            production_catalog = ManagedResourceRuntimeCatalog::production()?;
            &production_catalog
        }
    };
    // Install what the Record Requirements pass resolved. A declared track with no runtime
    // adapter has no artifact to install, so it is left to the apply, which decides whether
    // to tolerate it or record and fail it exactly as it always has.
    let declared_tracks = recorded_tracks
        .iter()
        .filter(|track| catalog.has_adapter(track.resource_name.as_str()))
        .cloned()
        .collect::<BTreeSet<_>>();
    let installed = crate::managed_resources::install_missing_resource_tracks(
        paths,
        runtime_catalog,
        &declared_tracks,
        progress.clone().suppressing_operation_phases(),
    )
    .await?;

    if !requested_php_extensions || !missing_gateway_runtime_resource(paths)? {
        return Ok(ProjectResourceInstall {
            installed,
            deferred_error: None,
        });
    }

    let repair_result = reconcile_system_resources_with_runtime_catalog_and_progress(
        paths,
        runtime_catalog,
        demanded_tracks,
        progress.suppressing_operation_phases(),
    )
    .await;

    Ok(ProjectResourceInstall {
        installed: true,
        deferred_error: repair_result.err(),
    })
}

fn missing_gateway_runtime_resource(paths: &PvPaths) -> Result<bool, DaemonError> {
    let database = Database::open(paths)?;
    Ok(database
        .managed_resource_tracks()?
        .into_iter()
        .any(|record| {
            gateway_runtime_resource(&record.resource_name)
                && record.desired_state == ManagedResourceDesiredState::Installed
                && record.current_artifact_path.is_none()
        }))
}

#[derive(Debug, Default)]
struct SystemProjectReconciliationReport {
    total: usize,
    succeeded: usize,
    successful_project_ids: Vec<String>,
    summaries: Vec<String>,
    failures: Vec<DaemonError>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct SystemProjectDemandReport {
    project_count: usize,
    fallback_count: usize,
    resource_tracks: BTreeSet<DemandedResourceTrack>,
    project_demands: BTreeMap<String, ProjectDemand>,
}

impl SystemProjectReconciliationReport {
    fn successful_project_coverage(&self) -> impl Iterator<Item = JobDiagnosticSubject> + '_ {
        self.successful_project_ids
            .iter()
            .cloned()
            .map(|id| JobDiagnosticSubject::Project { id })
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "`projects` scopes the apply to a resource scope's dependents while \
              `skip_project_ids` carries established-demand failures that must be reported \
              without being re-applied; neither is derivable from the other."
)]
async fn reconcile_system_projects_with_progress(
    paths: &PvPaths,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    demanded_tracks: &BTreeSet<DemandedResourceTrack>,
    project_demands: &BTreeMap<String, ProjectDemand>,
    progress: &DaemonDownloadProgress,
    stage: ProjectApplyStage,
    projects: &[ProjectRecord],
    skip_project_ids: &BTreeSet<String>,
    fallback_shutdown: Option<&watch::Receiver<bool>>,
) -> Result<SystemProjectReconciliationReport, DaemonError> {
    let mut report = SystemProjectReconciliationReport {
        total: projects.len(),
        ..SystemProjectReconciliationReport::default()
    };
    // Projects linked after discovery still need prerequisite installation.
    let empty_demand = ProjectDemand::default();

    for project in projects {
        if fallback_shutdown_requested(fallback_shutdown) {
            break;
        }
        if skip_project_ids.contains(&project.id) {
            continue;
        }
        match reconcile_project_env_with_runtime_catalog_and_progress_outcome(
            paths,
            &project.id,
            runtime_catalog,
            Some(project_demands.get(&project.id).unwrap_or(&empty_demand)),
            demanded_tracks,
            progress.clone(),
            ProjectApplyOptions::new(stage, fallback_shutdown),
        )
        .await
        {
            Ok(ReconciliationOutcome::Completed(summary)) => {
                report.succeeded += 1;
                report.successful_project_ids.push(project.id.clone());
                report.summaries.push(summary.as_str().to_owned());
            }
            Ok(ReconciliationOutcome::Cancelled) => break,
            Err(error @ DaemonError::ProjectEnvFailureRecordingFailed { .. }) => {
                return Err(error);
            }
            Err(error) => {
                let project_label = project.primary_hostname.as_deref().unwrap_or(&project.slug);
                report.failures.push(DaemonError::ProjectReconciliation {
                    project_label: project_label.to_owned(),
                    source: Box::new(error),
                });
            }
        }
    }

    Ok(report)
}

/// Adds Projects linked while Resources was running to the demand pinned for Project Apply.
/// Returns whether a new Project added resource demand that needs another full Resources pass.
fn discover_late_system_project_demand(
    paths: &PvPaths,
    resource_tracks: &mut BTreeSet<DemandedResourceTrack>,
    project_demands: &mut BTreeMap<String, ProjectDemand>,
) -> Result<bool, DaemonError> {
    let database = Database::open(paths)?;
    let mut has_late_resource_demand = false;

    for project in database.projects()? {
        if project_demands.contains_key(&project.id) {
            continue;
        }
        let demand = discover_project_demand(paths, &database, &project)?;
        has_late_resource_demand |= !demand.resource_tracks.is_empty();
        resource_tracks.extend(demand.resource_tracks.iter().cloned());
        project_demands.insert(project.id, demand);
    }

    Ok(has_late_resource_demand)
}

#[cfg(test)]
async fn reconcile_system_projects_and_resources_with_progress(
    paths: &PvPaths,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    progress: DaemonDownloadProgress,
    phase_log: &ReconciliationPhaseLog,
) -> Result<SystemProjectReconciliationReport, DaemonError> {
    let discovery_timer = phase_log.start(ReconciliationPhase::DemandDiscovery, "linked_projects");
    let discovery_result = discover_system_project_demand(paths);
    finish_demand_discovery_phase(discovery_timer, &discovery_result);
    let SystemProjectDemandReport {
        mut resource_tracks,
        mut project_demands,
        ..
    } = discovery_result?;

    let resources_timer = phase_log.start(ReconciliationPhase::Resources, "desired_resources");
    let mut resources_result = reconcile_system_resources_with_runtime_catalog_and_progress(
        paths,
        runtime_catalog,
        &resource_tracks,
        progress.clone(),
    )
    .await;
    resources_timer.finish(PhaseOutcome::from_succeeded(resources_result.is_ok()), &[]);
    // Retry the install once before applying the Projects, for the same reason as the logged
    // flow: no Project Apply downloads, so a later retry could not recover this apply.
    if resources_result.is_err() {
        let retry_timer = phase_log.start(ReconciliationPhase::Resources, "desired_resources");
        resources_result = reconcile_system_resources_with_runtime_catalog_and_progress(
            paths,
            runtime_catalog,
            &resource_tracks,
            progress.retrying_manifest_snapshot(),
        )
        .await;
        retry_timer.finish(PhaseOutcome::from_succeeded(resources_result.is_ok()), &[]);
    }
    let has_late_resource_demand =
        discover_late_system_project_demand(paths, &mut resource_tracks, &mut project_demands)?;
    if has_late_resource_demand {
        let late_timer = phase_log.start(ReconciliationPhase::Resources, "desired_resources");
        resources_result = reconcile_system_resources_with_runtime_catalog_and_progress(
            paths,
            runtime_catalog,
            &resource_tracks,
            progress.clone(),
        )
        .await;
        late_timer.finish(PhaseOutcome::from_succeeded(resources_result.is_ok()), &[]);
    }
    let project_timer = phase_log.start(ReconciliationPhase::ProjectApply, "linked_projects");
    let project_result = reconcile_system_projects_with_progress(
        paths,
        runtime_catalog,
        &resource_tracks,
        &project_demands,
        &progress,
        ProjectApplyStage::CompleteStagedApply,
        &linked_projects(paths)?,
        &BTreeSet::new(),
        None,
    )
    .await;
    finish_project_phase(project_timer, &project_result);
    let cleanup_result = stop_undemanded_system_resource_runtimes(paths, runtime_catalog).await;
    if resources_result.is_err()
        || project_result
            .as_ref()
            .is_ok_and(|report| !report.failures.is_empty())
    {
        resources_result = verify_system_resources_after_project_application(
            paths,
            runtime_catalog,
            project_result.as_ref().ok(),
            &progress,
        );
    }

    match (resources_result, project_result, cleanup_result) {
        (Ok(()), Ok(report), Ok(())) => Ok(report),
        (resources_result, project_result, cleanup_result) => {
            let project_failures = match project_result {
                Ok(report) => report.failures,
                Err(error) => vec![error],
            };
            Err(combined_system_reconciliation_error(
                resources_result
                    .err()
                    .into_iter()
                    .chain(project_failures)
                    .chain(cleanup_result.err())
                    .collect(),
            ))
        }
    }
}

fn verify_system_resources_after_project_application(
    paths: &PvPaths,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    project_report: Option<&SystemProjectReconciliationReport>,
    progress: &DaemonDownloadProgress,
) -> Result<(), DaemonError> {
    let database = Database::open(paths)?;
    let mut demanded_tracks = BTreeSet::new();
    for project in database.projects()? {
        // Successful application persisted its selected tracks; rediscovery could reselect PHP.
        if project_report.is_some_and(|report| report.successful_project_ids.contains(&project.id))
        {
            continue;
        }
        demanded_tracks
            .extend(discover_project_demand(paths, &database, &project)?.resource_tracks);
    }
    verify_system_resource_installations(paths, runtime_catalog, demanded_tracks, progress)
}

fn combined_system_reconciliation_error(mut failures: Vec<DaemonError>) -> DaemonError {
    if failures.len() == 1 {
        failures.remove(0)
    } else {
        DaemonError::SystemReconciliationFailures { failures }
    }
}

async fn reconcile_system_resources_with_runtime_catalog_and_progress(
    paths: &PvPaths,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    demanded_tracks: &BTreeSet<DemandedResourceTrack>,
    progress: DaemonDownloadProgress,
) -> Result<(), DaemonError> {
    if let Some(catalog) = runtime_catalog {
        let mut database = Database::open(paths)?;

        return reconcile_system_resources_with_catalog_and_progress(
            paths,
            &mut database,
            catalog,
            demanded_tracks,
            progress,
        )
        .await;
    }

    reconcile_system_resources_with_progress(paths, demanded_tracks, progress).await
}

fn discover_system_project_demand(
    paths: &PvPaths,
) -> Result<SystemProjectDemandReport, DaemonError> {
    let database = Database::open(paths)?;
    let projects = database.projects()?;
    let mut resource_tracks = BTreeSet::new();
    let mut project_demands = BTreeMap::new();
    let mut fallback_count = 0;

    for project in &projects {
        let demand = discover_project_demand(paths, &database, project)?;
        fallback_count += usize::from(demand.used_persisted_state);
        resource_tracks.extend(demand.resource_tracks.iter().cloned());
        project_demands.insert(project.id.clone(), demand);
    }

    Ok(SystemProjectDemandReport {
        project_count: projects.len(),
        fallback_count,
        resource_tracks,
        project_demands,
    })
}

pub(crate) fn linked_projects(paths: &PvPaths) -> Result<Vec<ProjectRecord>, DaemonError> {
    let database = Database::open(paths)?;

    Ok(database.projects()?)
}

fn completed_system_reconciliation_coverage(
    paths: &PvPaths,
    project_report: &SystemProjectReconciliationReport,
) -> Result<Vec<JobDiagnosticSubject>, DaemonError> {
    let database = Database::open(paths)?;
    let mut resource_subjects = BTreeSet::new();
    for project_id in &project_report.successful_project_ids {
        for resource in database.project_managed_resources(project_id)? {
            resource_subjects.insert((resource.resource_name, resource.track));
        }
    }

    let mut coverage = vec![
        JobDiagnosticSubject::SystemReconciliation,
        JobDiagnosticSubject::GatewayRuntime,
    ];
    coverage.extend(project_report.successful_project_coverage());
    coverage.extend(
        resource_subjects
            .into_iter()
            .map(|(name, track)| JobDiagnosticSubject::Resource { name, track }),
    );

    Ok(coverage)
}

fn system_reconciliation_summary(
    project_report: &SystemProjectReconciliationReport,
    gateway_summary: &str,
) -> String {
    let Some(project_summary) = system_project_summary(project_report) else {
        return gateway_summary.to_owned();
    };

    if gateway_summary == CADDY_NOT_INSTALLED {
        project_summary
    } else {
        format!("{project_summary}; {gateway_summary}")
    }
}

fn system_project_summary(report: &SystemProjectReconciliationReport) -> Option<String> {
    if report.total == 0 {
        return None;
    }

    if !report.failures.is_empty() {
        return Some(format!(
            "Project env reconciled for {} of {} Projects; failures: {}",
            report.succeeded,
            report.total,
            report
                .failures
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    if report.summaries.len() == 1 {
        return report.summaries.first().cloned();
    }

    Some(format!(
        "Project env reconciled for {} Projects",
        report.succeeded
    ))
}

fn managed_resource_reconciliation_summary(
    resource_name: &str,
    track: &str,
    project_report: &SystemProjectReconciliationReport,
) -> String {
    let Some(project_summary) = system_project_summary(project_report) else {
        return format!("Managed Resource {resource_name} track {track} reconciled");
    };

    format!("Managed Resource {resource_name} track {track} reconciled; {project_summary}")
}

fn reconciliation_started_message(scope: &ReconciliationScope) -> &'static str {
    let effective_scope = scope.effective();
    match &effective_scope {
        ReconciliationScope::Project { .. } => "Project env reconciliation started",
        ReconciliationScope::System => "System reconciliation started",
        ReconciliationScope::Resource { name, .. } if gateway_runtime_resource(name.as_str()) => {
            "Gateway runtime reconciliation started"
        }
        ReconciliationScope::Resource { .. } => "Managed Resource runtime reconciliation started",
    }
}

fn gateway_runtime_resource(resource_name: &str) -> bool {
    matches!(resource_name, "caddy" | "php" | "frankenphp")
}

#[cfg(test)]
fn complete_or_fail_background_reconciliation(
    paths: &PvPaths,
    job_id: &str,
    operation: impl FnOnce() -> Result<(), DaemonError>,
) -> Result<(), DaemonError> {
    match operation() {
        Ok(()) => Ok(()),
        Err(error) => {
            let error_message = error.to_string();
            let mut database = Database::open(paths)?;
            database.fail_job(job_id, &error_message)?;

            Err(error)
        }
    }
}

pub(crate) fn record_background_reconciliation_error(
    paths: &PvPaths,
    scope: &str,
    error: &DaemonError,
) -> Result<(), DaemonError> {
    let error_message = error.to_string();
    let mut database = Database::open(paths)?;
    let already_recorded = database
        .unresolved_job_failures()?
        .into_iter()
        .any(|failure| {
            failure.job.kind == "reconcile"
                && failure.job.scope == scope
                && failure.job.error.as_deref() == Some(error_message.as_str())
        });

    if already_recorded {
        return Ok(());
    }

    let job = database.start_job("reconcile", scope)?;
    structured_log::job_started(paths, &job.id, "reconcile", scope);
    database.fail_job(&job.id, &error_message)?;
    structured_log::job_failed(paths, &job.id, "reconcile", scope, &error_message);

    Ok(())
}

async fn run_invalid_reconciliation_scope_job(
    paths: PvPaths,
    mut transport: DaemonTransport<LocalStream>,
    scope: &str,
    parse_error: crate::reconciliation::ReconciliationScopeParseError,
) -> Result<(), DaemonError> {
    let mut database = Database::open(&paths)?;
    let job = database.start_job("reconcile", scope)?;
    let error = format!("invalid reconciliation scope `{scope}`: {parse_error}");
    structured_log::job_started(&paths, &job.id, "reconcile", scope);

    let stream_is_open = async {
        write_line(
            &mut transport,
            &DaemonResponse::accepted("job accepted", &job.id),
        )
        .await?;
        write_line(
            &mut transport,
            &DaemonEvent::JobStarted {
                job_id: &job.id,
                kind: "reconcile",
                scope,
            },
        )
        .await?;

        Ok::<(), DaemonError>(())
    }
    .await
    .is_ok();

    database.fail_job(&job.id, &error)?;
    structured_log::job_failed(&paths, &job.id, "reconcile", scope, &error);

    if stream_is_open {
        write_line(
            &mut transport,
            &DaemonEvent::JobFailed {
                job_id: &job.id,
                error: &error,
            },
        )
        .await?;
    }

    Ok(())
}

async fn run_started_job(
    paths: PvPaths,
    mut transport: DaemonTransport<LocalStream>,
    kind: &str,
    scope: &str,
) -> Result<(), DaemonError> {
    let mut database = Database::open(&paths)?;
    let job = database.start_job(kind, scope)?;
    let summary = "stub job completed";
    structured_log::job_started(&paths, &job.id, kind, scope);

    let stream_is_open = async {
        write_line(
            &mut transport,
            &DaemonResponse::accepted("job accepted", &job.id),
        )
        .await?;
        write_line(
            &mut transport,
            &DaemonEvent::JobStarted {
                job_id: &job.id,
                kind,
                scope,
            },
        )
        .await?;
        write_line(
            &mut transport,
            &DaemonEvent::Log {
                job_id: &job.id,
                message: "stub job started",
            },
        )
        .await?;

        Ok::<(), DaemonError>(())
    }
    .await
    .is_ok();

    if kind != "reconcile" || scope.parse::<ReconciliationScope>().is_err() {
        let error = format!("unsupported daemon job `{kind}` with scope `{scope}`");
        database.fail_job(&job.id, &error)?;
        structured_log::job_failed(&paths, &job.id, kind, scope, &error);

        if stream_is_open {
            write_line(
                &mut transport,
                &DaemonEvent::JobFailed {
                    job_id: &job.id,
                    error: &error,
                },
            )
            .await?;
        }

        return Ok(());
    }

    database.complete_job(&job.id, summary)?;
    structured_log::job_completed(&paths, &job.id, kind, scope, summary);
    if !stream_is_open {
        return Ok(());
    }

    let write_result = async {
        write_line(
            &mut transport,
            &DaemonEvent::Progress {
                job_id: &job.id,
                message: "stub job completed without reconciliation work",
            },
        )
        .await?;

        Ok::<(), DaemonError>(())
    }
    .await;

    write_result?;

    write_line(
        &mut transport,
        &DaemonEvent::JobCompleted {
            job_id: &job.id,
            summary,
        },
    )
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet, VecDeque};
    use std::fs;
    use std::io::{self, Write};
    use std::net::{TcpListener, TcpStream};
    use std::os::unix::fs::PermissionsExt;
    use std::pin::Pin;
    use std::process;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::task::{Context, Poll};
    use std::time::Instant;

    use camino::{Utf8Path, Utf8PathBuf};
    use camino_tempfile::tempdir;
    use config::ConfigError;
    use futures_util::StreamExt;
    use insta::{Settings, allow_duplicates, assert_debug_snapshot, assert_snapshot};
    use rcgen::generate_simple_self_signed;
    use resources::{ManagedResourceCommandError, ResourceHttpClient, ResourcesError};
    use rusqlite::{Connection, Error as SqliteError};
    #[cfg(target_os = "macos")]
    use rustix::process::{Pid, test_kill_process, test_kill_process_group};
    use serde_json::json;
    use state::{
        Database, GatewayPort, JobDiagnosticSubject, JobStatus, JobsLock, LinkProjectInput,
        ManagedResourceDesiredState, PortRequest, ProjectEnvObservedStatus,
        ProjectEnvObservedWarningInput, ProjectManagedResourceInput, ProjectMode,
        ProjectPhpRuntimeInput, ProjectReconciliationStateInput, PvPaths, ResourceAllocationInput,
        RuntimeObservedStatus, RuntimeSubject, StateError, UpdateLock,
    };
    use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf, duplex};
    #[cfg(target_os = "macos")]
    use tokio::net::UnixStream;
    use tokio::sync::{mpsc::channel, oneshot, watch};
    use tokio::time::{Duration, sleep, timeout};

    use crate::gateway::{
        GatewayPfRoutingState, reconcile_gateway_runtimes_with_pf_state_for_test,
    };
    use crate::project_env::reconcile_project_env_from_persisted_state;

    #[cfg(target_os = "macos")]
    use super::run_reconciliation_job;
    use super::{
        BackgroundReconciliationError, DaemonDownloadProgress, FOREGROUND_JOB_PROGRESS_BUFFER,
        FOREGROUND_JOB_STREAM_WRITE_TIMEOUT, ForegroundJobEvent, ReconciliationJobCompletion,
        SystemProjectReconciliationReport, abandon_reconciliation_job,
        cancel_or_preserve_system_reconciliation_errors,
        complete_managed_resource_reconciliation_with_progress,
        complete_or_fail_background_reconciliation, complete_project_reconciliation_with_progress,
        complete_queued_background_reconciliation_job, complete_reconciliation_job_with_progress,
        complete_streamed_job_with_heartbeat, complete_streamed_job_with_heartbeat_and_events,
        complete_system_reconciliation_with_progress, complete_update_job,
        complete_update_job_with_progress, completed_system_reconciliation_coverage,
        discover_system_project_demand, enqueue_foreground_reconciliation_job,
        enqueue_reconciliation_job, enqueue_startup_reconciliation_job, enqueue_update_job,
        foreground_reconciliation_result, linked_projects, managed_resource_reconciliation_summary,
        reconcile_persisted_project_envs, reconcile_project_env_and_missing_resources,
        reconcile_project_env_with_runtime_catalog_and_progress,
        reconcile_system_projects_and_resources_with_progress,
        reconcile_system_projects_with_progress,
        reconcile_system_resources_with_runtime_catalog_and_progress,
        record_background_reconciliation_error, run_background_reconciliation_job,
        run_startup_reconciliation_job, start_reconciliation_job, start_update_job,
        stop_undemanded_system_resource_runtimes, stream_started_reconciliation_job,
        stream_started_update_job, system_project_summary, unready_established_resource_projects,
        wait_for_foreground_turn, wait_for_startup_reconciliation_turn,
        write_coalesced_update_response, write_foreground_terminal_event,
    };
    use crate::project_env::{ProjectApplyOptions, ProjectApplyStage};
    use crate::reconciliation::{
        EnqueueResult, ReconciliationJobTiming, ReconciliationQueue, ReconciliationScope,
    };
    use crate::{DaemonError, ProcessSupervisor};

    const OFFLINE_TEST_MANIFEST_URL: &str = "https://127.0.0.1:9/manifest.json";
    const CADDY_TEST_TRACK: &str = "2";
    const CADDY_TEST_ARTIFACT_VERSION: &str = "2.11.4-pv1";
    const CADDY_TEST_ARCHIVE_FILE_NAME: &str = "caddy-2.11.4-pv1-any.tar.gz";
    const PHP_TEST_TRACK: &str = "8.5";
    const PHP_TEST_ARTIFACT_VERSION: &str = "8.5.0-pv1";
    const PHP_TEST_ARCHIVE_FILE_NAME: &str = "php-8.5.0-pv1-any.tar.gz";
    const FRANKENPHP_TEST_ARCHIVE_FILE_NAME: &str = "frankenphp-8.5.0-pv1-any.tar.gz";
    const COMPOSER_TEST_TRACK: &str = "2";
    const COMPOSER_TEST_ARTIFACT_VERSION: &str = "2.8.0-pv1";
    const COMPOSER_TEST_ARCHIVE_FILE_NAME: &str = "composer-2.8.0-pv1-any.tar.gz";
    const STREAMED_RECONCILIATION_PROGRESS_SETUP_TIMEOUT: Duration = Duration::from_secs(3);
    const STREAMED_RECONCILIATION_COMPLETION_TIMEOUT: Duration = Duration::from_secs(5);
    const MAILPIT_TEST_TRACK: &str = "1.0";
    const MAILPIT_TEST_ARTIFACT_VERSION: &str = "1.0.0-pv1";
    const MAILPIT_TEST_ARCHIVE_FILE_NAME: &str = "mailpit-1.0.0-pv1-any.tar.gz";
    const FAKE_MAILPIT_SCRIPT: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/test-fixtures/managed-resources/fake-mailpit.py"
    ));
    const FAKE_CADDY_SCRIPT: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/test-fixtures/gateway/fake-caddy.sh"
    ));
    const FAKE_CADDY_SERVER_SCRIPT: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/test-fixtures/gateway/fake-caddy-server.py"
    ));

    #[tokio::test]
    async fn invalid_project_config_does_not_block_crashed_worker_recovery() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        seed_installed_caddy(&paths)?;
        let certified_key = generate_simple_self_signed(vec![
            "project.test".to_owned(),
            "pv-gateway.localhost".to_owned(),
        ])?;
        state::fs::write_sensitive_file(&paths.ca_certificate(), &certified_key.cert.pem())?;
        state::fs::write_sensitive_file(
            &paths.ca_private_key(),
            &certified_key.signing_key.serialize_pem(),
        )?;
        let mut caddy_guard = SeededRuntimeGuard::with_gateway(paths.clone());
        let release = tempdir.path().join("frankenphp-release");
        caddy_guard.register_worker("8.4", &release);
        let executable = release.join("bin/frankenphp");
        state::fs::write_sensitive_file(
            &executable,
            include_str!("../test-fixtures/gateway/fake-stateful-frankenphp.sh"),
        )?;
        state::fs::write_sensitive_file(
            &Utf8PathBuf::from(format!("{executable}.server.py")),
            include_str!("../test-fixtures/gateway/fake-stateful-runtime-server.py"),
        )?;
        set_executable(&executable)?;
        let project_path = tempdir.path().join("project");
        let config_path = project_path.join("pv.yml");
        state::fs::write_sensitive_file(&config_path, "php: \"8.4\"\n")?;
        let mut database = Database::open(&paths)?;
        let project = database
            .link_project(LinkProjectInput {
                path: project_path.clone(),
                original_path: project_path,
                primary_hostname: "project.test".to_owned(),
                config_path: config_path.clone(),
                desired_php_track: Some("8.4".to_owned()),
                additional_hostnames: Vec::new(),
            })?
            .project;
        database.record_managed_resource_track_installed(
            "frankenphp",
            "8.4",
            "fake-frankenphp-pv1",
            &release,
        )?;
        drop(database);
        reconcile_gateway_runtimes_with_pf_state_for_test(
            &paths,
            Duration::from_secs(5),
            GatewayPfRoutingState::Inactive,
        )
        .await?;
        let supervisor = ProcessSupervisor::new(paths.clone());
        let worker = supervisor
            .adopt_recorded(
                &paths.worker_pid("8.4"),
                &paths.worker_runtime_metadata("8.4"),
            )?
            .ok_or_else(|| anyhow::anyhow!("worker not running after setup"))?;
        worker.stop(Duration::from_secs(1)).await?;
        state::fs::write_sensitive_file(&config_path, "php: [\n")?;
        let scope = ReconciliationScope::project(project.id.clone())?;
        let result = run_background_reconciliation_job(
            paths.clone(),
            ReconciliationQueue::new(),
            scope,
            None,
        )
        .await;
        let recovered = supervisor.adopt_recorded(
            &paths.worker_pid("8.4"),
            &paths.worker_runtime_metadata("8.4"),
        )?;
        let worker_recovered = recovered.is_some();
        drop(recovered);
        let database = Database::open(&paths)?;
        let job_statuses = database
            .recent_jobs()?
            .into_iter()
            .map(|job| job.status)
            .collect::<Vec<_>>();
        let project_status = database
            .project_env_observed_state(&project.id)?
            .map(|state| state.status);
        assert!(
            matches!(result, Err(DaemonError::Config(ConfigError::Parse { .. }))),
            "unexpected Project failure: {result:?}"
        );
        assert_debug_snapshot!((worker_recovered, project_status, job_statuses), @r"
        (
            true,
            Some(
                Failed,
            ),
            [
                Failed,
            ],
        )
        ");
        caddy_guard.cleanup().await?;
        assert!(!paths.worker_pid("8.4").exists());
        assert!(!paths.worker_runtime_metadata("8.4").exists());
        Ok(())
    }

    #[tokio::test]
    async fn project_and_gateway_recovery_failures_are_both_reported() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        seed_installed_caddy(&paths)?;
        let mut caddy_guard = SeededRuntimeGuard::with_gateway(paths.clone());
        let project_path = tempdir.path().join("project");
        let config_path = project_path.join("pv.yml");
        state::fs::write_sensitive_file(&config_path, "php: [\n")?;
        let project = Database::open(&paths)?
            .link_project(LinkProjectInput {
                path: project_path.clone(),
                original_path: project_path,
                primary_hostname: "project.test".to_owned(),
                config_path,
                desired_php_track: Some("8.4".to_owned()),
                additional_hostnames: Vec::new(),
            })?
            .project;
        let result = run_background_reconciliation_job(
            paths.clone(),
            ReconciliationQueue::new(),
            ReconciliationScope::project(project.id)?,
            None,
        )
        .await;
        let Err(DaemonError::SystemReconciliationFailures { failures }) = result else {
            anyhow::bail!("expected Project and Gateway failures, got {result:?}");
        };
        assert!(
            matches!(failures.as_slice(), [
            DaemonError::Config(ConfigError::Parse { .. }),
            DaemonError::UnexpectedProtocolResponse { reason },
        ] if reason == "FrankenPHP is not installed for PHP track `8.4`"),
            "unexpected failures: {failures:?}"
        );
        caddy_guard.cleanup().await?;
        Ok(())
    }

    #[tokio::test]
    async fn resource_only_project_without_active_route_excludes_gateway_coverage()
    -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let project_path = tempdir.path().join("resource-only");
        let config_path = project_path.join("pv.yml");
        let uncertain_path = tempdir.path().join("uncertain");
        let uncertain_config_path = uncertain_path.join("pv.yml");
        state::fs::write_sensitive_file(&config_path, "serve: false\n")?;
        state::fs::write_sensitive_file(&uncertain_config_path, "serve: false\n")?;
        let mut database = Database::open(&paths)?;
        let project = database
            .link_project_with_mode(
                LinkProjectInput {
                    path: project_path.clone(),
                    original_path: project_path,
                    primary_hostname: "ignored.test".to_owned(),
                    config_path,
                    desired_php_track: None,
                    additional_hostnames: Vec::new(),
                },
                ProjectMode::ResourceOnly,
            )?
            .project;
        drop(database);
        seed_installed_caddy(&paths)?;
        let mut caddy_guard = SeededRuntimeGuard::with_gateway(paths.clone());
        crate::gateway::reconcile_gateway_runtimes_with_pf_state_for_test(
            &paths,
            Duration::from_secs(5),
            crate::gateway::GatewayPfRoutingState::Inactive,
        )
        .await?;
        let mut database = Database::open(&paths)?;
        let uncertain = database
            .link_project(LinkProjectInput {
                path: uncertain_path.clone(),
                original_path: uncertain_path,
                primary_hostname: "uncertain.test".to_owned(),
                config_path: uncertain_config_path,
                desired_php_track: None,
                additional_hostnames: Vec::new(),
            })?
            .project;
        drop(database);
        let scope = ReconciliationScope::project(project.id.clone())?;
        let ReconciliationScope::Project { id } = &scope else {
            return Err(anyhow::anyhow!("expected Project scope"));
        };
        let phase_log = crate::structured_log::ReconciliationPhaseLog::new(
            &paths,
            "resource-only-project-test",
            "reconcile",
            &scope.to_string(),
        );

        let completed = complete_project_reconciliation_with_progress(
            &paths,
            id,
            None,
            super::DaemonDownloadProgress::disabled(),
            &phase_log,
            Some(crate::gateway::GatewayPfRoutingState::Inactive),
            &mut None,
        )
        .await?;

        assert_eq!(
            completed.coverage,
            [JobDiagnosticSubject::Project { id: project.id }]
        );
        assert!(paths.gateway_pid().exists());
        assert_eq!(
            Database::open(&paths)?
                .project_by_id(&uncertain.id)?
                .ok_or_else(|| anyhow::anyhow!("expected uncertain Project"))?
                .mode,
            ProjectMode::Served
        );
        let phase_events = reconciliation_phase_events(&paths, "resource-only-project-test")?;
        for phase in ["workers", "gateway"] {
            let event = phase_events
                .iter()
                .find(|event| event["phase"] == phase)
                .ok_or_else(|| anyhow::anyhow!("missing skipped {phase} phase"))?;
            assert_eq!(event["outcome"], "skipped");
            assert_eq!(event["elapsed_ms"], 0);
            assert_eq!(event["subject"], "target_project");
        }

        caddy_guard.cleanup().await?;
        Ok(())
    }

    #[tokio::test]
    async fn targeted_skip_refreshes_failed_gateway_observation() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let project_path = tempdir.path().join("resource-only");
        let config_path = project_path.join("pv.yml");
        state::fs::write_sensitive_file(&config_path, "serve: false\n")?;
        let mut database = Database::open(&paths)?;
        let project = database
            .link_project_with_mode(
                LinkProjectInput {
                    path: project_path.clone(),
                    original_path: project_path,
                    primary_hostname: "ignored.test".to_owned(),
                    config_path,
                    desired_php_track: None,
                    additional_hostnames: Vec::new(),
                },
                ProjectMode::ResourceOnly,
            )?
            .project;
        drop(database);
        seed_installed_caddy(&paths)?;
        let mut caddy_guard = SeededRuntimeGuard::with_gateway(paths.clone());
        crate::gateway::reconcile_gateway_runtimes_with_pf_state_for_test(
            &paths,
            Duration::from_secs(5),
            crate::gateway::GatewayPfRoutingState::Inactive,
        )
        .await?;
        let mut database = Database::open(&paths)?;
        database.record_runtime_observed_snapshot(
            RuntimeSubject::Gateway,
            RuntimeObservedStatus::Failed,
            Some("previous gateway failure"),
        )?;
        drop(database);

        let scope = ReconciliationScope::project(project.id.clone())?;
        let ReconciliationScope::Project { id } = &scope else {
            return Err(anyhow::anyhow!("expected Project scope"));
        };
        let phase_log = crate::structured_log::ReconciliationPhaseLog::new(
            &paths,
            "targeted-skip-gateway-test",
            "reconcile",
            &scope.to_string(),
        );
        let completed = complete_project_reconciliation_with_progress(
            &paths,
            id,
            None,
            super::DaemonDownloadProgress::disabled(),
            &phase_log,
            Some(crate::gateway::GatewayPfRoutingState::Inactive),
            &mut None,
        )
        .await?;

        assert_eq!(
            completed.coverage,
            [JobDiagnosticSubject::Project {
                id: project.id.clone()
            }]
        );
        let gateway_observation = Database::open(&paths)?
            .runtime_observed_states()?
            .into_iter()
            .find(|state| state.subject == RuntimeSubject::Gateway)
            .ok_or_else(|| anyhow::anyhow!("missing Gateway observation"))?;
        assert_eq!(gateway_observation.status, RuntimeObservedStatus::Degraded);
        assert_eq!(
            gateway_observation.message.as_deref(),
            Some(
                "Low-port routing is inactive; run `pv ports:install` to restore ports 80 and 443"
            )
        );

        stop_seeded_caddy(&paths).await?;
        let mut database = Database::open(&paths)?;
        database.record_runtime_observed_snapshot(
            RuntimeSubject::Gateway,
            RuntimeObservedStatus::Failed,
            Some("previous gateway failure"),
        )?;
        drop(database);
        let failed_probe_phase_log = crate::structured_log::ReconciliationPhaseLog::new(
            &paths,
            "targeted-skip-gateway-failed-probe",
            "reconcile",
            &scope.to_string(),
        );
        let outcome = crate::gateway::reconcile_project_gateway_runtimes_with_phase_log(
            &paths,
            &project.id,
            Some(crate::gateway::GatewayPfRoutingState::Inactive),
            &failed_probe_phase_log,
        )
        .await?;
        assert!(
            matches!(
                outcome,
                crate::gateway::ProjectGatewayReconciliationOutcome::PromoteSystem
            ),
            "expected failed probe to promote to System reconciliation"
        );
        let gateway_observation = Database::open(&paths)?
            .runtime_observed_states()?
            .into_iter()
            .find(|state| state.subject == RuntimeSubject::Gateway)
            .ok_or_else(|| anyhow::anyhow!("missing Gateway observation"))?;
        assert_eq!(gateway_observation.status, RuntimeObservedStatus::Failed);
        assert_eq!(
            gateway_observation.message.as_deref(),
            Some("previous gateway failure")
        );
        caddy_guard.cleanup().await?;

        Ok(())
    }

    #[tokio::test]
    async fn uncertain_project_gateway_plan_promotes_to_system_reconciliation() -> anyhow::Result<()>
    {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let target_path = tempdir.path().join("target");
        let target_config_path = target_path.join("pv.yml");
        let uncertain_path = tempdir.path().join("uncertain");
        let uncertain_config_path = uncertain_path.join("pv.yml");
        state::fs::write_sensitive_file(&target_config_path, "serve: false\n")?;
        state::fs::write_sensitive_file(&uncertain_config_path, "serve: false\n")?;
        let mut database = Database::open(&paths)?;
        let target = database
            .link_project_with_mode(
                LinkProjectInput {
                    path: target_path.clone(),
                    original_path: target_path,
                    primary_hostname: "ignored.test".to_owned(),
                    config_path: target_config_path,
                    desired_php_track: None,
                    additional_hostnames: Vec::new(),
                },
                ProjectMode::ResourceOnly,
            )?
            .project;
        let uncertain = database
            .link_project(LinkProjectInput {
                path: uncertain_path.clone(),
                original_path: uncertain_path,
                primary_hostname: "uncertain.test".to_owned(),
                config_path: uncertain_config_path,
                desired_php_track: None,
                additional_hostnames: Vec::new(),
            })?
            .project;
        drop(database);
        state::fs::write_sensitive_file(
            &paths
                .gateway_projects_config_dir()
                .join(format!("{}.Caddyfile", target.id)),
            "# stale active target route\n",
        )?;
        seed_installed_caddy(&paths)?;
        let mut caddy_guard = SeededRuntimeGuard::with_gateway(paths.clone());
        let scope = ReconciliationScope::project(target.id.clone())?;
        let ReconciliationScope::Project { id } = &scope else {
            return Err(anyhow::anyhow!("expected Project scope"));
        };
        let phase_log = crate::structured_log::ReconciliationPhaseLog::new(
            &paths,
            "project-system-promotion-test",
            "reconcile",
            &scope.to_string(),
        );

        let completed = complete_project_reconciliation_with_progress(
            &paths,
            id,
            None,
            super::DaemonDownloadProgress::disabled(),
            &phase_log,
            None,
            &mut None,
        )
        .await?;
        let database = Database::open(&paths)?;
        let uncertain = database
            .project_by_id(&uncertain.id)?
            .ok_or_else(|| anyhow::anyhow!("expected uncertain Project"))?;

        assert_eq!(uncertain.mode, ProjectMode::ResourceOnly);
        assert!(
            completed
                .coverage
                .contains(&JobDiagnosticSubject::SystemReconciliation)
        );
        assert!(
            completed
                .coverage
                .contains(&JobDiagnosticSubject::GatewayRuntime)
        );
        assert_eq!(
            completed
                .coverage
                .iter()
                .filter(|subject| matches!(subject, JobDiagnosticSubject::Project { .. }))
                .count(),
            2
        );

        caddy_guard.cleanup().await?;
        Ok(())
    }

    #[tokio::test]
    async fn promoted_project_failure_retains_system_diagnostic_subject() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let mut database = Database::open(&paths)?;
        let mut projects = Vec::new();
        for name in ["target", "peer"] {
            let project_path = tempdir.path().join(name);
            let config_path = project_path.join("pv.yml");
            state::fs::write_sensitive_file(&config_path, "serve: [\n")?;
            projects.push(
                database
                    .link_project_with_mode(
                        LinkProjectInput {
                            path: project_path.clone(),
                            original_path: project_path,
                            primary_hostname: format!("{name}.test"),
                            config_path,
                            desired_php_track: None,
                            additional_hostnames: Vec::new(),
                        },
                        ProjectMode::ResourceOnly,
                    )?
                    .project,
            );
        }
        database.record_managed_resource_track_removal_intent("mailpit", "1.0", false, true)?;
        state::fs::write_sensitive_file(
            &projects[1].config_path,
            "serve: false\nmailpit:\n  version: \"1.0\"\n",
        )?;
        drop(database);
        seed_installed_caddy(&paths)?;
        let mut caddy_guard = SeededRuntimeGuard::with_gateway(paths.clone());
        let target = &projects[0];
        let scope = ReconciliationScope::project(target.id.clone())?;
        let project_failure = start_reconciliation_job(&paths, &scope.to_string())?;
        let result = super::complete_reconciliation_job(
            &paths,
            &project_failure,
            &scope,
            None,
            ReconciliationJobTiming::immediate(),
            Some(crate::gateway::GatewayPfRoutingState::Inactive),
        )
        .await;
        assert!(matches!(result, Err(DaemonError::Config(_))));
        let unresolved = Database::open(&paths)?.unresolved_job_failures()?;
        assert_eq!(unresolved.len(), 1);
        assert_eq!(
            unresolved[0].subject,
            JobDiagnosticSubject::Project {
                id: target.id.clone()
            }
        );

        state::fs::write_sensitive_file(&target.config_path, "serve: false\n")?;
        let stale_fragment = paths.gateway_projects_config_dir().join("stale.Caddyfile");
        state::fs::write_sensitive_file(&stale_fragment, "# unverified routing snapshot\n")?;
        let promoted_failure = start_reconciliation_job(&paths, &scope.to_string())?;
        let result = super::complete_reconciliation_job(
            &paths,
            &promoted_failure,
            &scope,
            None,
            ReconciliationJobTiming::immediate(),
            Some(crate::gateway::GatewayPfRoutingState::Inactive),
        )
        .await;
        assert!(
            matches!(
                result,
                Err(DaemonError::SystemReconciliationFailures { .. })
            ),
            "{result:?}"
        );
        let unresolved = Database::open(&paths)?.unresolved_job_failures()?;
        let promoted = unresolved
            .iter()
            .find(|failure| failure.job.id == promoted_failure)
            .ok_or_else(|| anyhow::anyhow!("missing promoted failure"))?;
        assert_eq!(promoted.subject, JobDiagnosticSubject::SystemReconciliation);

        state::fs::remove_file_if_exists(&stale_fragment)?;
        let project_success = start_reconciliation_job(&paths, &scope.to_string())?;
        super::complete_reconciliation_job(
            &paths,
            &project_success,
            &scope,
            None,
            ReconciliationJobTiming::immediate(),
            Some(crate::gateway::GatewayPfRoutingState::Inactive),
        )
        .await?;
        let unresolved = Database::open(&paths)?.unresolved_job_failures()?;
        assert_eq!(unresolved.len(), 1);
        assert_eq!(unresolved[0].job.id, promoted_failure);
        assert_eq!(
            unresolved[0].subject,
            JobDiagnosticSubject::SystemReconciliation
        );

        state::fs::write_sensitive_file(&projects[1].config_path, "serve: false\n")?;
        let system_success = start_reconciliation_job(&paths, "system")?;
        super::complete_reconciliation_job(
            &paths,
            &system_success,
            &ReconciliationScope::System,
            None,
            ReconciliationJobTiming::immediate(),
            None,
        )
        .await?;
        assert!(
            Database::open(&paths)?
                .unresolved_job_failures()?
                .is_empty()
        );
        caddy_guard.cleanup().await?;
        Ok(())
    }

    #[tokio::test]
    async fn targeted_resource_scope_records_exact_partial_and_success_coverage()
    -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let mut database = Database::open(&paths)?;
        let project_config = r#"mailpit:
  version: "1.0"
  env:
    MAIL_HOST: "${smtp_host}"
"#;
        let mut projects = Vec::new();
        for name in ["acme", "beta", "unrelated"] {
            let project_path = tempdir.path().join(name);
            let config_path = project_path.join("pv.yml");
            state::fs::write_sensitive_file(&config_path, project_config)?;
            let linked = database.link_project(LinkProjectInput {
                path: project_path.clone(),
                original_path: project_path,
                primary_hostname: format!("{name}.test"),
                config_path,
                desired_php_track: None,
                additional_hostnames: Vec::new(),
            })?;
            projects.push(linked.project);
        }
        let desired_resource = ProjectManagedResourceInput {
            resource_name: "mailpit".to_owned(),
            track: MAILPIT_TEST_TRACK.to_owned(),
        };
        for project in &projects[..2] {
            database.replace_project_managed_resources(
                &project.id,
                std::slice::from_ref(&desired_resource),
            )?;
        }
        database.record_managed_resource_track_env_context(
            "mailpit",
            MAILPIT_TEST_TRACK,
            &BTreeMap::from([("smtp_host".to_owned(), "127.0.0.1".to_owned())]),
        )?;
        database.record_runtime_observed_snapshot(
            RuntimeSubject::Resource {
                name: "mailpit".to_owned(),
                track: MAILPIT_TEST_TRACK.to_owned(),
            },
            RuntimeObservedStatus::Running,
            Some("fixture mailpit readiness diagnostic"),
        )?;
        database.record_managed_resource_track_installed(
            "php",
            PHP_TEST_TRACK,
            PHP_TEST_ARTIFACT_VERSION,
            &paths
                .resources()
                .join("php/8.5/releases")
                .join(PHP_TEST_ARTIFACT_VERSION),
        )?;
        database.replace_project_php_runtime(
            &projects[1].id,
            Some(&ProjectPhpRuntimeInput {
                track: PHP_TEST_TRACK.to_owned(),
                requested_extensions: vec!["unsupported_fixture".to_owned()],
                loaded_extensions: Vec::new(),
                ignored_extensions: vec!["unsupported_fixture".to_owned()],
            }),
        )?;
        let persisted_php_runtime = database
            .project_by_id(&projects[1].id)?
            .ok_or_else(|| anyhow::anyhow!("expected Beta Project"))?
            .php_runtime;
        database.record_project_env_observed_snapshot(
            &projects[1].id,
            ProjectEnvObservedStatus::Warning,
            Some("Project runtime has warnings"),
            &[
                ProjectEnvObservedWarningInput {
                    kind: "ignored_php_extension".to_owned(),
                    message: "ignored unsupported PHP extension `unsupported_fixture`".to_owned(),
                },
                ProjectEnvObservedWarningInput {
                    kind: "duplicate_key".to_owned(),
                    message: "a previously duplicated env key".to_owned(),
                },
            ],
        )?;
        drop(database);

        state::fs::write_sensitive_file(
            &projects[1].config_path,
            r#"mailpit:
  version: "1.1"
  env:
    MAIL_HOST: "${smtp_host}"
"#,
        )?;
        let scope = ReconciliationScope::resource("mailpit", MAILPIT_TEST_TRACK)?;
        let ReconciliationScope::Resource { name, track } = &scope else {
            return Err(anyhow::anyhow!("expected resource reconciliation scope"));
        };
        let catalog =
            crate::managed_resources::ManagedResourceRuntimeCatalog::without_adapters_with_manifest_url(
                OFFLINE_TEST_MANIFEST_URL,
            )?;
        let phase_log = crate::structured_log::ReconciliationPhaseLog::new(
            &paths,
            "targeted-resource-partial",
            "reconcile",
            &scope.to_string(),
        );
        let partial = complete_managed_resource_reconciliation_with_progress(
            &paths,
            name,
            track,
            Some(&catalog),
            super::DaemonDownloadProgress::disabled(),
            &phase_log,
        )
        .await?;
        let database = Database::open(&paths)?;
        let partial_statuses = projects
            .iter()
            .map(|project| {
                database
                    .project_env_observed_state(&project.id)
                    .map(|observed| observed.map(|observed| observed.status))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let persisted_beta_demand = database
            .project_managed_resources(&projects[1].id)?
            .into_iter()
            .map(|resource| (resource.resource_name, resource.track))
            .collect::<Vec<_>>();
        drop(database);
        assert_eq!(
            partial.coverage,
            vec![
                JobDiagnosticSubject::Resource {
                    name: "mailpit".to_owned(),
                    track: MAILPIT_TEST_TRACK.to_owned()
                },
                JobDiagnosticSubject::Project {
                    id: projects[0].id.clone()
                },
            ]
        );
        let partial_snapshot = (
            partial.summary,
            partial.coverage,
            partial_statuses,
            persisted_beta_demand,
            state::fs::read_to_string(&projects[0].path.join(".env"))?,
            state::fs::path_entry_exists(&projects[1].path.join(".env"))?,
            state::fs::path_entry_exists(&projects[2].path.join(".env"))?,
        );

        state::fs::write_sensitive_file(
            &projects[1].config_path,
            &format!(
                "php:\n  version: \"8.5\"\n  extensions: [unsupported_fixture]\n{project_config}"
            ),
        )?;
        reconcile_project_env_from_persisted_state(
            &paths,
            &mut Database::open(&paths)?,
            &projects[1].id,
        )?;
        assert_eq!(
            Database::open(&paths)?
                .project_env_observed_state(&projects[1].id)?
                .map(|observed| observed.status),
            Some(ProjectEnvObservedStatus::Warning)
        );
        state::fs::write_sensitive_file(&projects[1].path.join(".env"), "EXISTING=beta\n")?;
        let allocation_report = reconcile_persisted_project_envs(
            &paths,
            &projects[..2],
            BTreeMap::from([(
                projects[1].id.clone(),
                DaemonError::UnexpectedProtocolResponse {
                    reason: "fixture rejected allocation `database`".to_owned(),
                },
            )]),
        )?;
        let mut allocation_coverage = vec![JobDiagnosticSubject::Resource {
            name: "mailpit".to_owned(),
            track: MAILPIT_TEST_TRACK.to_owned(),
        }];
        allocation_coverage.extend(allocation_report.successful_project_coverage());
        let database = Database::open(&paths)?;
        let allocation_statuses = projects[..2]
            .iter()
            .map(|project| {
                database
                    .project_env_observed_state(&project.id)
                    .map(|observed| observed.map(|observed| observed.status))
            })
            .collect::<Result<Vec<_>, _>>()?;
        drop(database);
        assert_eq!(
            allocation_coverage,
            vec![
                JobDiagnosticSubject::Resource {
                    name: "mailpit".to_owned(),
                    track: MAILPIT_TEST_TRACK.to_owned()
                },
                JobDiagnosticSubject::Project {
                    id: projects[0].id.clone()
                },
            ]
        );
        let allocation_partial_snapshot = (
            managed_resource_reconciliation_summary(
                "mailpit",
                MAILPIT_TEST_TRACK,
                &allocation_report,
            ),
            allocation_coverage,
            allocation_statuses,
            state::fs::read_to_string(&projects[1].path.join(".env"))?,
        );
        let phase_log = crate::structured_log::ReconciliationPhaseLog::new(
            &paths,
            "targeted-resource-success",
            "reconcile",
            &scope.to_string(),
        );
        let success = complete_managed_resource_reconciliation_with_progress(
            &paths,
            name,
            track,
            Some(&catalog),
            super::DaemonDownloadProgress::disabled(),
            &phase_log,
        )
        .await?;
        let database = Database::open(&paths)?;
        let success_statuses = projects
            .iter()
            .map(|project| {
                database
                    .project_env_observed_state(&project.id)
                    .map(|observed| observed.map(|observed| observed.status))
            })
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(
            success_statuses,
            [
                Some(ProjectEnvObservedStatus::Rendered),
                Some(ProjectEnvObservedStatus::Warning),
                None,
            ]
        );
        assert_eq!(
            success.coverage,
            vec![
                JobDiagnosticSubject::Resource {
                    name: "mailpit".to_owned(),
                    track: MAILPIT_TEST_TRACK.to_owned()
                },
                JobDiagnosticSubject::Project {
                    id: projects[0].id.clone()
                },
                JobDiagnosticSubject::Project {
                    id: projects[1].id.clone()
                },
            ]
        );
        assert_eq!(
            database
                .project_by_id(&projects[1].id)?
                .ok_or_else(|| anyhow::anyhow!("expected Beta Project"))?
                .php_runtime,
            persisted_php_runtime
        );
        let warnings = database
            .project_env_observed_state(&projects[1].id)?
            .ok_or_else(|| anyhow::anyhow!("expected recovered Beta observation"))?
            .warnings
            .into_iter()
            .map(|warning| (warning.kind, warning.message))
            .collect::<Vec<_>>();
        assert_eq!(
            warnings,
            vec![(
                "ignored_php_extension".to_owned(),
                "ignored unsupported PHP extension `unsupported_fixture`".to_owned()
            )]
        );
        let success_snapshot = (
            success.summary,
            success.coverage,
            success_statuses,
            state::fs::read_to_string(&projects[0].path.join(".env"))?,
            state::fs::read_to_string(&projects[1].path.join(".env"))?,
            state::fs::path_entry_exists(&projects[2].path.join(".env"))?,
        );

        let mut settings = Settings::clone_current();
        settings.add_filter(tempdir.path().as_str(), "<tempdir>");
        settings.add_filter(r#"id: "[a-z0-9]{10}""#, r#"id: "<project_id>""#);
        settings.add_filter(r"Project `[a-z0-9]{10}`", "Project `<project_id>`");
        settings.bind(|| {
            assert_debug_snapshot!("targeted_resource_scope_partial_failure", partial_snapshot);
            assert_debug_snapshot!(
                "targeted_resource_scope_allocation_partial_failure",
                allocation_partial_snapshot
            );
            assert_debug_snapshot!("targeted_resource_scope_success", success_snapshot);
        });

        Ok(())
    }

    #[tokio::test]
    async fn targeted_resource_track_failure_records_dependent_project_failures()
    -> anyhow::Result<()> {
        let mut outcomes = Vec::new();
        for reject_observation in [false, true] {
            let tempdir = tempdir()?;
            let paths = PvPaths::for_home(tempdir.path().join("home"));
            let mut runtime_guard =
                SeededRuntimeGuard::with_resource(paths.clone(), "mailpit", MAILPIT_TEST_TRACK);
            seed_installed_artifact(
                &paths,
                "mailpit",
                MAILPIT_TEST_TRACK,
                MAILPIT_TEST_ARTIFACT_VERSION,
                "bin/pv-fake-mailpit",
            )?;
            let executable = paths
                .resources()
                .join("mailpit/1.0/current/bin/pv-fake-mailpit");
            state::fs::write_sensitive_file(&executable, FAKE_MAILPIT_SCRIPT)?;
            set_executable(&executable)?;
            let mut database = Database::open(&paths)?;
            let mut projects = Vec::new();
            let previous_env =
                "USER_VALUE=kept\n# >>> PV MANAGED\nMAIL_HOST=previous\n# <<< PV MANAGED\n";
            for name in ["acme", "beta", "unrelated"] {
                let project_path = tempdir.path().join(name);
                let config_path = project_path.join("pv.yml");
                state::fs::write_sensitive_file(
                    &config_path,
                    "mailpit:\n  version: \"1.0\"\n  env:\n    MAIL_HOST: \"${smtp_host}\"\n",
                )?;
                state::fs::write_sensitive_file(&project_path.join(".env"), previous_env)?;
                let project = database
                    .link_project(LinkProjectInput {
                        path: project_path.clone(),
                        original_path: project_path,
                        primary_hostname: format!("{name}.test"),
                        config_path,
                        desired_php_track: None,
                        additional_hostnames: Vec::new(),
                    })?
                    .project;
                database.record_project_env_observed_snapshot(
                    &project.id,
                    ProjectEnvObservedStatus::Rendered,
                    Some("last valid env"),
                    &[],
                )?;
                if name != "unrelated" {
                    database.replace_project_managed_resources(
                        &project.id,
                        &[ProjectManagedResourceInput {
                            resource_name: "mailpit".to_owned(),
                            track: MAILPIT_TEST_TRACK.to_owned(),
                        }],
                    )?;
                }
                projects.push(project);
            }
            let unrelated_before = database.project_env_observed_state(&projects[2].id)?;
            if reject_observation {
                Connection::open(paths.db().as_std_path())?.execute_batch(
                    "CREATE TRIGGER reject_project_failure BEFORE INSERT ON observed_states
                     WHEN NEW.subject_kind = 'project_env'
                     BEGIN SELECT RAISE(FAIL, 'fixture rejected Project failure observation'); END;"
                )?;
            }
            let catalog =
                crate::managed_resources::fake_unready_runtime_catalog(OFFLINE_TEST_MANIFEST_URL)?;
            let scope = ReconciliationScope::resource("mailpit", MAILPIT_TEST_TRACK)?;
            let ReconciliationScope::Resource { name, track } = &scope else {
                return Err(anyhow::anyhow!("expected resource scope"));
            };
            let phase_log = crate::structured_log::ReconciliationPhaseLog::new(
                &paths,
                "fatal-track",
                "reconcile",
                &scope.to_string(),
            );
            let result = complete_managed_resource_reconciliation_with_progress(
                &paths,
                name,
                track,
                Some(&catalog),
                super::DaemonDownloadProgress::disabled(),
                &phase_log,
            )
            .await;
            let observations = projects
                .iter()
                .map(|project| database.project_env_observed_state(&project.id))
                .collect::<Result<Vec<_>, _>>()?;
            let files_preserved = projects
                .iter()
                .map(|project| state::fs::read_to_string(&project.path.join(".env")))
                .collect::<Result<Vec<_>, _>>()?
                .iter()
                .all(|content| content == previous_env);
            let runtime_failed = database.runtime_observed_states()?.iter().any(|observed| {
                observed.subject
                    == RuntimeSubject::Resource {
                        name: "mailpit".to_owned(),
                        track: MAILPIT_TEST_TRACK.to_owned(),
                    }
                    && observed.status == RuntimeObservedStatus::Failed
            });
            for project in &projects[..2] {
                database.replace_project_managed_resources(&project.id, &[])?;
            }
            drop(database);
            let resource_cleanup_result =
                stop_undemanded_system_resource_runtimes(&paths, Some(&catalog)).await;
            let correct_error = match (&result, reject_observation) {
                (
                    Err(DaemonError::ReadinessTimedOut {
                        timeout_ms: 100, ..
                    }),
                    false,
                ) => true,
                (
                    Err(DaemonError::ProjectEnvFailureRecordingFailed {
                        project_id,
                        reconciliation,
                        recording,
                    }),
                    true,
                ) => {
                    projects[..2]
                        .iter()
                        .any(|project| &project.id == project_id)
                        && matches!(
                            reconciliation.as_ref(),
                            DaemonError::ReadinessTimedOut {
                                timeout_ms: 100,
                                ..
                            }
                        )
                        && matches!(recording.as_ref(), DaemonError::State(StateError::Sqlite(SqliteError::SqliteFailure(_, Some(message)))) if message == "fixture rejected Project failure observation")
                }
                _ => false,
            };
            let dependents_failed = reject_observation
                || observations[..2].iter().all(|observed| {
                    observed.as_ref().is_some_and(|observed| {
                        observed.status == ProjectEnvObservedStatus::Failed
                            && observed.message == result.as_ref().err().map(ToString::to_string)
                    })
                });
            let outcome = (
                format!(
                    "reject_observation={reject_observation}, error={:?}",
                    result.as_ref().err()
                ),
                [
                    correct_error,
                    dependents_failed,
                    files_preserved,
                    runtime_failed,
                    observations[2] == unrelated_before,
                    resource_cleanup_result.is_ok(),
                    !state::fs::path_entry_exists(
                        &paths.resource_pid("mailpit", MAILPIT_TEST_TRACK),
                    )?,
                ],
            );
            let outcome_passed = outcome.1.iter().all(|passed| *passed);
            let guard_cleanup_result = runtime_guard.cleanup().await;
            if !outcome_passed {
                anyhow::bail!(
                    "fatal track outcome: {outcome:#?}; resource cleanup: {resource_cleanup_result:?}; guard cleanup: {guard_cleanup_result:?}"
                );
            }
            match (resource_cleanup_result, guard_cleanup_result) {
                (Ok(()), Ok(())) => {}
                (Err(resource_error), Ok(())) => return Err(resource_error.into()),
                (Ok(()), Err(guard_error)) => return Err(guard_error),
                (Err(resource_error), Err(guard_error)) => anyhow::bail!(
                    "resource cleanup failed: {resource_error}; guard cleanup failed: {guard_error}"
                ),
            }
            outcomes.push(outcome);
        }
        assert!(
            outcomes
                .iter()
                .all(|(_, checks)| checks.iter().all(|passed| *passed)),
            "fatal track outcomes: {outcomes:#?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn targeted_resource_job_fails_when_project_lookup_failure_cannot_be_recorded()
    -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let mut database = Database::open(&paths)?;
        let config = "mailpit:\n  version: '1.0'\n  env:\n    MAIL_HOST: '${smtp_host}'\n";
        let mut projects = Vec::new();
        for name in ["acme", "beta"] {
            let project_path = tempdir.path().join(name);
            let config_path = project_path.join("pv.yml");
            state::fs::write_sensitive_file(&config_path, config)?;
            let project = database
                .link_project(LinkProjectInput {
                    path: project_path.clone(),
                    original_path: project_path,
                    primary_hostname: format!("{name}.test"),
                    config_path,
                    desired_php_track: None,
                    additional_hostnames: Vec::new(),
                })?
                .project;
            database.replace_project_managed_resources(
                &project.id,
                &[ProjectManagedResourceInput {
                    resource_name: "mailpit".to_owned(),
                    track: MAILPIT_TEST_TRACK.to_owned(),
                }],
            )?;
            projects.push(project);
        }
        database.record_managed_resource_track_env_context(
            "mailpit",
            MAILPIT_TEST_TRACK,
            &BTreeMap::from([("smtp_host".to_owned(), "127.0.0.1".to_owned())]),
        )?;
        database.record_runtime_observed_snapshot(
            RuntimeSubject::Resource {
                name: "mailpit".to_owned(),
                track: MAILPIT_TEST_TRACK.to_owned(),
            },
            RuntimeObservedStatus::Running,
            Some("fixture mailpit readiness diagnostic"),
        )?;
        let catalog = crate::managed_resources::ManagedResourceRuntimeCatalog::without_adapters_with_manifest_url(OFFLINE_TEST_MANIFEST_URL)?;
        let scope = ReconciliationScope::resource("mailpit", MAILPIT_TEST_TRACK)?;
        run_background_reconciliation_job(
            paths.clone(),
            ReconciliationQueue::new(),
            scope.clone(),
            Some(&catalog),
        )
        .await?;
        let beta_before = database.project_env_observed_state(&projects[1].id)?;
        let beta_env = state::fs::read_to_string(&projects[1].path.join(".env"))?;
        let previous_ids = database
            .recent_jobs()?
            .into_iter()
            .map(|job| job.id)
            .collect::<BTreeSet<_>>();
        state::fs::write_sensitive_file(
            &projects[0].path.join(".env"),
            "USER_VALUE=kept\n# >>> PV MANAGED\nMAIL_HOST=previous\n# <<< PV MANAGED\n",
        )?;
        Connection::open(paths.db().as_std_path())?.execute_batch(&format!(
            "CREATE TRIGGER corrupt_later_project BEFORE INSERT ON observed_states
             WHEN NEW.subject_kind = 'project_env' AND NEW.subject_id = '{}' AND NEW.status = 'rendered'
             BEGIN UPDATE projects SET path = X'00' WHERE id = '{}'; END;
             CREATE TRIGGER reject_later_project_failure BEFORE INSERT ON observed_states
             WHEN NEW.subject_kind = 'project_env' AND NEW.subject_id = '{}' AND NEW.status = 'failed'
             BEGIN SELECT RAISE(FAIL, 'fixture rejected Project lookup failure observation'); END;",
            projects[0].id, projects[1].id, projects[1].id,
        ))?;
        let result = run_background_reconciliation_job(
            paths.clone(),
            ReconciliationQueue::new(),
            scope,
            Some(&catalog),
        )
        .await;
        let jobs = database
            .recent_jobs()?
            .into_iter()
            .filter(|job| !previous_ids.contains(&job.id))
            .collect::<Vec<_>>();
        let [job] = jobs.as_slice() else {
            return Err(anyhow::anyhow!(
                "expected one new reconciliation job: {jobs:#?}"
            ));
        };
        let coverage = state::testing::transaction(&mut database, |transaction| {
            let mut statement = transaction.prepare("SELECT subject_kind, subject_id FROM job_diagnostic_outcomes WHERE job_id = ?1 AND outcome = 'success' ORDER BY subject_kind, subject_id")?;
            statement
                .query_map([&job.id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<BTreeSet<_>>>()
        })?;
        let error_message = result.as_ref().err().map(ToString::to_string);
        let exact_error = matches!(&result,
            Err(DaemonError::ProjectEnvFailureRecordingFailed { project_id, reconciliation, recording })
            if project_id == &projects[1].id
                && matches!(reconciliation.as_ref(), DaemonError::State(StateError::Sqlite(SqliteError::InvalidColumnType(1, column, _))) if column == "path")
                && matches!(recording.as_ref(), DaemonError::State(StateError::Sqlite(error)) if error.to_string() == "fixture rejected Project lookup failure observation")
        );
        let checks = [
            (
                "initial real reconciliation verifies Beta",
                beta_before
                    .as_ref()
                    .is_some_and(|observed| observed.status == ProjectEnvObservedStatus::Rendered),
            ),
            ("lookup and recording errors are both typed", exact_error),
            (
                "actual reconciliation job fails with the returned error",
                job.kind == "reconcile"
                    && job.scope == "resource:mailpit:1.0"
                    && job.status == JobStatus::Failed
                    && job.error == error_message,
            ),
            (
                "fatal job records no successful coverage",
                coverage.is_empty(),
            ),
            (
                "unrecordable Project keeps its prior observation",
                database.project_env_observed_state(&projects[1].id)? == beta_before,
            ),
            (
                "unrecordable Project keeps its env",
                state::fs::read_to_string(&projects[1].path.join(".env"))? == beta_env,
            ),
            (
                "earlier Project completes its env work",
                state::fs::read_to_string(&projects[0].path.join(".env"))?
                    == "USER_VALUE=kept\n# >>> PV MANAGED\nMAIL_HOST=127.0.0.1\n# <<< PV MANAGED\n",
            ),
        ];
        assert!(
            checks.iter().all(|(_, passed)| *passed),
            "result={result:#?}, job={job:#?}, coverage={coverage:#?}, checks={checks:#?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn targeted_resource_scope_refuses_unapplied_php_identity() -> anyhow::Result<()> {
        for (php_config, applied) in [
            ("php: \"8.5\"\n", true),
            ("php: latest\n", true),
            ("php: \"8.4\"\n", false),
            ("php:\n  version: \"8.5\"\n  extensions: [redis]\n", false),
            ("", false),
        ] {
            let tempdir = tempdir()?;
            let paths = PvPaths::for_home(tempdir.path().join("home"));
            let project_path = tempdir.path().join("project");
            let config_path = project_path.join("pv.yml");
            state::fs::write_sensitive_file(
                &config_path,
                &format!(
                    "serve: false\n{php_config}mailpit:\n  version: \"1.0\"\n  env:\n    MAIL_HOST: \"${{smtp_host}}\"\n"
                ),
            )?;
            let mut database = Database::open(&paths)?;
            let project = database
                .link_project_with_mode(
                    LinkProjectInput {
                        path: project_path.clone(),
                        original_path: project_path,
                        primary_hostname: "project.test".to_owned(),
                        config_path,
                        desired_php_track: Some("8.5".to_owned()),
                        additional_hostnames: Vec::new(),
                    },
                    ProjectMode::ResourceOnly,
                )?
                .project;
            database.replace_project_managed_resources(
                &project.id,
                &[ProjectManagedResourceInput {
                    resource_name: "mailpit".to_owned(),
                    track: "1.0".to_owned(),
                }],
            )?;
            database.record_managed_resource_track_env_context(
                "mailpit",
                "1.0",
                &BTreeMap::from([("smtp_host".to_owned(), "127.0.0.1".to_owned())]),
            )?;
            database.record_runtime_observed_snapshot(
                RuntimeSubject::Resource {
                    name: "mailpit".to_owned(),
                    track: "1.0".to_owned(),
                },
                RuntimeObservedStatus::Running,
                Some("fixture mailpit readiness diagnostic"),
            )?;
            database.record_project_env_observed_snapshot(
                &project.id,
                ProjectEnvObservedStatus::Failed,
                Some("previous PHP application failed"),
                &[],
            )?;
            let previous_env = "USER_VALUE=kept\n";
            state::fs::write_sensitive_file(&project.path.join(".env"), previous_env)?;
            let result =
                reconcile_project_env_from_persisted_state(&paths, &mut database, &project.id);
            if applied {
                assert!(result.is_ok(), "{result:?}");
            } else {
                assert!(
                    matches!(
                        result,
                        Err(DaemonError::ProjectEnvDependenciesNotApplied { .. })
                    ),
                    "{result:?}"
                );
            }
            let catalog = crate::managed_resources::ManagedResourceRuntimeCatalog::without_adapters_with_manifest_url(OFFLINE_TEST_MANIFEST_URL)?;
            let scope = ReconciliationScope::resource("mailpit", "1.0")?;
            let ReconciliationScope::Resource { name, track } = &scope else {
                return Err(anyhow::anyhow!("expected resource scope"));
            };
            let phase_log = crate::structured_log::ReconciliationPhaseLog::new(
                &paths,
                "php-dependency",
                "reconcile",
                &scope.to_string(),
            );
            let completion = complete_managed_resource_reconciliation_with_progress(
                &paths,
                name,
                track,
                Some(&catalog),
                super::DaemonDownloadProgress::disabled(),
                &phase_log,
            )
            .await?;
            assert_eq!(
                completion
                    .coverage
                    .contains(&JobDiagnosticSubject::Project {
                        id: project.id.clone()
                    }),
                applied
            );
            let observed = database
                .project_env_observed_state(&project.id)?
                .ok_or_else(|| anyhow::anyhow!("expected Project observation"))?;
            assert_eq!(
                observed.status,
                if applied {
                    ProjectEnvObservedStatus::Rendered
                } else {
                    ProjectEnvObservedStatus::Failed
                }
            );
            assert_eq!(
                database
                    .project_by_id(&project.id)?
                    .map(|project| project.php_runtime),
                Some(project.php_runtime)
            );
            if !applied {
                assert_eq!(
                    state::fs::read_to_string(&project.path.join(".env"))?,
                    previous_env
                );
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn targeted_resource_scope_refuses_served_project_without_installed_php()
    -> anyhow::Result<()> {
        for installed in [false, true] {
            let tempdir = tempdir()?;
            let paths = PvPaths::for_home(tempdir.path().join("home"));
            let project_path = tempdir.path().join("project");
            let config_path = project_path.join("pv.yml");
            state::fs::write_sensitive_file(
                &config_path,
                "php: \"8.5\"\nmailpit:\n  version: \"1.0\"\n  env:\n    MAIL_HOST: \"${smtp_host}\"\n",
            )?;
            let mut database = Database::open(&paths)?;
            let project = database
                .link_project(LinkProjectInput {
                    path: project_path.clone(),
                    original_path: project_path.clone(),
                    primary_hostname: "project.test".to_owned(),
                    config_path,
                    desired_php_track: Some(PHP_TEST_TRACK.to_owned()),
                    additional_hostnames: Vec::new(),
                })?
                .project;
            database.replace_project_managed_resources(
                &project.id,
                &[ProjectManagedResourceInput {
                    resource_name: "mailpit".to_owned(),
                    track: MAILPIT_TEST_TRACK.to_owned(),
                }],
            )?;
            database.replace_project_php_runtime(
                &project.id,
                Some(&ProjectPhpRuntimeInput {
                    track: PHP_TEST_TRACK.to_owned(),
                    requested_extensions: Vec::new(),
                    loaded_extensions: Vec::new(),
                    ignored_extensions: Vec::new(),
                }),
            )?;
            if installed {
                database.record_managed_resource_track_installed(
                    "php",
                    PHP_TEST_TRACK,
                    PHP_TEST_ARTIFACT_VERSION,
                    &paths
                        .resources()
                        .join("php/8.5/releases")
                        .join(PHP_TEST_ARTIFACT_VERSION),
                )?;
            }
            database.record_managed_resource_track_env_context(
                "mailpit",
                MAILPIT_TEST_TRACK,
                &BTreeMap::from([("smtp_host".to_owned(), "127.0.0.1".to_owned())]),
            )?;
            database.record_runtime_observed_snapshot(
                RuntimeSubject::Resource {
                    name: "mailpit".to_owned(),
                    track: MAILPIT_TEST_TRACK.to_owned(),
                },
                RuntimeObservedStatus::Running,
                Some("fixture mailpit readiness diagnostic"),
            )?;
            let previous_env = "USER_VALUE=kept\n";
            state::fs::write_sensitive_file(&project.path.join(".env"), previous_env)?;

            let result =
                reconcile_project_env_from_persisted_state(&paths, &mut database, &project.id);
            if installed {
                assert!(result.is_ok(), "{result:?}");
            } else {
                assert!(
                    matches!(
                        result,
                        Err(DaemonError::ProjectEnvDependenciesNotApplied { .. })
                    ),
                    "{result:?}"
                );
            }

            let catalog = crate::managed_resources::ManagedResourceRuntimeCatalog::without_adapters_with_manifest_url(OFFLINE_TEST_MANIFEST_URL)?;
            let scope = ReconciliationScope::resource("mailpit", MAILPIT_TEST_TRACK)?;
            let ReconciliationScope::Resource { name, track } = &scope else {
                return Err(anyhow::anyhow!("expected resource scope"));
            };
            let phase_log = crate::structured_log::ReconciliationPhaseLog::new(
                &paths,
                "installed-php",
                "reconcile",
                &scope.to_string(),
            );
            let completion = complete_managed_resource_reconciliation_with_progress(
                &paths,
                name,
                track,
                Some(&catalog),
                super::DaemonDownloadProgress::disabled(),
                &phase_log,
            )
            .await?;
            assert_eq!(
                completion
                    .coverage
                    .contains(&JobDiagnosticSubject::Project {
                        id: project.id.clone()
                    }),
                installed
            );
            assert_eq!(
                database
                    .project_env_observed_state(&project.id)?
                    .map(|observed| observed.status),
                Some(if installed {
                    ProjectEnvObservedStatus::Rendered
                } else {
                    ProjectEnvObservedStatus::Failed
                })
            );
            assert_eq!(
                state::fs::read_to_string(&project.path.join(".env"))?,
                if installed {
                    "USER_VALUE=kept\n# >>> PV MANAGED\nMAIL_HOST=127.0.0.1\n# <<< PV MANAGED\n"
                } else {
                    previous_env
                }
            );
        }

        Ok(())
    }

    #[tokio::test]
    async fn targeted_resource_scope_refuses_unready_sibling_dependencies() -> anyhow::Result<()> {
        let mut checks = Vec::new();
        for with_mappings in [true, false] {
            let tempdir = tempdir()?;
            let paths = PvPaths::for_home(tempdir.path().join("home"));
            let project_path = tempdir.path().join("project");
            let config_path = project_path.join("pv.yml");
            let mappings = if with_mappings {
                "  env:\n    MAIL_HOST: \"${smtp_host}\"\n"
            } else {
                ""
            };
            let allocations = if with_mappings {
                "  allocations:\n    app: {}\n"
            } else {
                ""
            };
            state::fs::write_sensitive_file(
                &config_path,
                &format!(
                    "mailpit:\n  version: \"1.0\"\n{mappings}mysql:\n  version: \"8.0\"\n{allocations}"
                ),
            )?;
            let mut database = Database::open(&paths)?;
            let project = database
                .link_project(LinkProjectInput {
                    path: project_path.clone(),
                    original_path: project_path.clone(),
                    primary_hostname: "project.test".to_owned(),
                    config_path,
                    desired_php_track: None,
                    additional_hostnames: Vec::new(),
                })?
                .project;
            database.replace_project_managed_resources(
                &project.id,
                &[
                    ProjectManagedResourceInput {
                        resource_name: "mailpit".to_owned(),
                        track: "1.0".to_owned(),
                    },
                    ProjectManagedResourceInput {
                        resource_name: "mysql".to_owned(),
                        track: "8.0".to_owned(),
                    },
                ],
            )?;
            for (resource, track) in [("mailpit", "1.0"), ("mysql", "8.0")] {
                database.record_managed_resource_track_env_context(
                    resource,
                    track,
                    &BTreeMap::from([("smtp_host".to_owned(), "127.0.0.1".to_owned())]),
                )?;
            }
            database.record_runtime_observed_snapshot(
                RuntimeSubject::Resource {
                    name: "mailpit".to_owned(),
                    track: "1.0".to_owned(),
                },
                RuntimeObservedStatus::Running,
                Some("fixture mailpit readiness diagnostic"),
            )?;
            if with_mappings {
                let generated =
                    resources::generated_allocation_name("mysql", &project.slug, "app")?;
                database.replace_project_resource_allocations(
                    &project.id,
                    "mysql",
                    "8.0",
                    &[ResourceAllocationInput {
                        allocation_name: "app".to_owned(),
                        generated_name: generated.generated_name().to_owned(),
                    }],
                )?;
            }
            let catalog = crate::managed_resources::ManagedResourceRuntimeCatalog::without_adapters_with_manifest_url(OFFLINE_TEST_MANIFEST_URL)?;
            let scope = ReconciliationScope::resource("mailpit", "1.0")?;
            let ReconciliationScope::Resource { name, track } = &scope else {
                return Err(anyhow::anyhow!("expected resource scope"));
            };
            let phase_log = crate::structured_log::ReconciliationPhaseLog::new(
                &paths,
                "dependency-readiness",
                "reconcile",
                &scope.to_string(),
            );
            let previous_env =
                "USER_VALUE=kept\n# >>> PV MANAGED\nMAIL_HOST=previous\n# <<< PV MANAGED\n";
            for (status, status_name) in [
                (None, "missing"),
                (Some(RuntimeObservedStatus::Failed), "failed"),
                (Some(RuntimeObservedStatus::Degraded), "degraded"),
                (Some(RuntimeObservedStatus::Stopped), "stopped"),
                (Some(RuntimeObservedStatus::Pending), "pending"),
                (Some(RuntimeObservedStatus::Running), "running"),
            ] {
                if with_mappings && status == Some(RuntimeObservedStatus::Failed) {
                    database.mark_resource_allocation_ready(
                        &project.id,
                        "mysql",
                        "8.0",
                        "app",
                        &BTreeMap::new(),
                    )?;
                }
                if let Some(status) = status {
                    database.record_runtime_observed_snapshot(
                        RuntimeSubject::Resource {
                            name: "mysql".to_owned(),
                            track: "8.0".to_owned(),
                        },
                        status,
                        Some("fixture mysql readiness diagnostic"),
                    )?;
                }
                let resource_observations = database.runtime_observed_states()?;
                state::fs::write_sensitive_file(&project_path.join(".env"), previous_env)?;
                let (prior_status, prior_message) = if status.is_none() {
                    (
                        ProjectEnvObservedStatus::Failed,
                        "Project config error: previous config failure",
                    )
                } else {
                    (
                        ProjectEnvObservedStatus::Failed,
                        "previous dependency failure",
                    )
                };
                database.record_project_env_observed_snapshot(
                    &project.id,
                    prior_status,
                    Some(prior_message),
                    &[],
                )?;
                let result = reconcile_project_env_from_persisted_state(
                    &paths,
                    &mut Database::open(&paths)?,
                    &project.id,
                );
                let completion = complete_managed_resource_reconciliation_with_progress(
                    &paths,
                    name,
                    track,
                    Some(&catalog),
                    super::DaemonDownloadProgress::disabled(),
                    &phase_log,
                )
                .await?;
                let healthy = status == Some(RuntimeObservedStatus::Running);
                let correct_result = match (&result, status) {
                    (
                        Err(DaemonError::Config(ConfigError::MissingAllocationEnvContext {
                            resource,
                            allocation,
                        })),
                        None,
                    ) if with_mappings => resource == "mysql" && allocation == "app",
                    (
                        Err(DaemonError::ProjectEnvDependenciesNotApplied { project_id, reason }),
                        None,
                    ) => {
                        project_id == &project.id
                            && reason == "required resource mysql track 8.0 has no observed state"
                    }
                    (
                        Err(DaemonError::ProjectEnvDependenciesNotApplied { project_id, .. }),
                        Some(
                            RuntimeObservedStatus::Failed
                            | RuntimeObservedStatus::Degraded
                            | RuntimeObservedStatus::Stopped
                            | RuntimeObservedStatus::Pending,
                        ),
                    ) => {
                        project_id == &project.id
                            && result.as_ref().err().map(ToString::to_string)
                                == Some(format!(
                                    "Project `{}` env dependencies cannot be refreshed: required resource mysql track 8.0 is {status_name}: fixture mysql readiness diagnostic",
                                    project.id
                                ))
                    }
                    (Ok(_), Some(RuntimeObservedStatus::Running)) => true,
                    _ => false,
                };
                let mut expected_coverage = vec![JobDiagnosticSubject::Resource {
                    name: "mailpit".to_owned(),
                    track: "1.0".to_owned(),
                }];
                if healthy {
                    expected_coverage.push(JobDiagnosticSubject::Project {
                        id: project.id.clone(),
                    });
                }
                let expected_env = if healthy && with_mappings {
                    previous_env.replace("MAIL_HOST=previous", "MAIL_HOST=127.0.0.1")
                } else {
                    previous_env.to_owned()
                };
                let observed = database
                    .project_env_observed_state(&project.id)?
                    .ok_or_else(|| anyhow::anyhow!("expected Project observation"))?;
                checks.push((
                    format!("mappings={with_mappings}, status={status:?}, result={result:?}"),
                    [
                        correct_result,
                        completion.coverage == expected_coverage,
                        observed.status
                            == if healthy {
                                ProjectEnvObservedStatus::Rendered
                            } else {
                                ProjectEnvObservedStatus::Failed
                            },
                        state::fs::read_to_string(&project_path.join(".env"))? == expected_env,
                        database.runtime_observed_states()? == resource_observations,
                        status.is_some()
                            || observed.message
                                == Some(if with_mappings {
                                    DaemonError::Config(
                                        ConfigError::MissingAllocationEnvContext {
                                            resource: "mysql".to_owned(),
                                            allocation: "app".to_owned(),
                                        },
                                    )
                                    .to_string()
                                } else {
                                    DaemonError::ProjectEnvDependenciesNotApplied {
                                        project_id: project.id.clone(),
                                        reason: "required resource mysql track 8.0 has no observed state"
                                            .to_owned(),
                                    }
                                    .to_string()
                                }),
                    ],
                ));
            }
        }
        assert!(
            checks
                .iter()
                .all(|(_, checks)| checks.iter().all(|passed| *passed)),
            "dependency outcomes: {checks:#?}"
        );
        Ok(())
    }

    #[test]
    fn unready_resource_projects_ignores_historical_inactive_allocations() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let mut database = Database::open(&paths)?;
        let project_path = tempdir.path().join("project");
        let config_path = project_path.join("pv.yml");
        state::fs::write_sensitive_file(
            &config_path,
            "mailpit:\n  version: \"1.0\"\nmysql:\n  version: \"8.0\"\nrustfs:\n  version: \"1.0\"\n",
        )?;
        let project = database
            .link_project(LinkProjectInput {
                path: project_path.clone(),
                original_path: project_path,
                primary_hostname: "project.test".to_owned(),
                config_path,
                desired_php_track: None,
                additional_hostnames: Vec::new(),
            })?
            .project;
        database.replace_project_managed_resources(
            &project.id,
            &[
                ProjectManagedResourceInput {
                    resource_name: "mailpit".to_owned(),
                    track: "1.0".to_owned(),
                },
                ProjectManagedResourceInput {
                    resource_name: "mysql".to_owned(),
                    track: "8.0".to_owned(),
                },
                ProjectManagedResourceInput {
                    resource_name: "rustfs".to_owned(),
                    track: "1.0".to_owned(),
                },
            ],
        )?;
        let mysql_allocation = resources::generated_allocation_name("mysql", &project.slug, "app")?;
        database.replace_project_resource_allocations(
            &project.id,
            "mysql",
            "8.0",
            &[ResourceAllocationInput {
                allocation_name: "app".to_owned(),
                generated_name: mysql_allocation.generated_name().to_owned(),
            }],
        )?;
        database.replace_project_resource_allocations(&project.id, "mysql", "8.0", &[])?;
        let retired_allocation =
            resources::generated_allocation_name("rustfs", &project.slug, "retired")?;
        database.replace_project_resource_allocations(
            &project.id,
            "rustfs",
            "1.0",
            &[ResourceAllocationInput {
                allocation_name: "retired".to_owned(),
                generated_name: retired_allocation.generated_name().to_owned(),
            }],
        )?;
        let uploads_allocation =
            resources::generated_allocation_name("rustfs", &project.slug, "uploads")?;
        database.replace_project_resource_allocations(
            &project.id,
            "rustfs",
            "1.0",
            &[ResourceAllocationInput {
                allocation_name: "uploads".to_owned(),
                generated_name: uploads_allocation.generated_name().to_owned(),
            }],
        )?;
        database.mark_resource_allocation_ready(
            &project.id,
            "rustfs",
            "1.0",
            "uploads",
            &BTreeMap::new(),
        )?;
        database.record_runtime_observed_snapshot(
            RuntimeSubject::Resource {
                name: "rustfs".to_owned(),
                track: "1.0".to_owned(),
            },
            RuntimeObservedStatus::Running,
            Some("fixture rustfs readiness diagnostic"),
        )?;

        let failures = unready_established_resource_projects(
            &paths,
            std::slice::from_ref(&project),
            "mailpit",
            "1.0",
        )?;

        assert!(failures.is_empty(), "{failures:#?}");
        Ok(())
    }

    #[test]
    fn targeted_resource_scope_requires_applied_hostnames_and_tls_artifacts() -> anyhow::Result<()>
    {
        for check_tls in [false, true] {
            let tempdir = tempdir()?;
            let paths = PvPaths::for_home(tempdir.path().join("home"));
            let project_path = tempdir.path().join("project");
            let config_path = project_path.join("pv.yml");
            let dependency = if check_tls {
                "env:\n  CERTIFICATE: \"${tls_cert}\"\n"
            } else {
                "hostnames:\n  - api.project.test\n"
            };
            state::fs::write_sensitive_file(
                &config_path,
                &format!(
                    "{dependency}mailpit:\n  version: \"1.0\"\n  env:\n    MAIL_HOST: \"${{smtp_host}}\"\n"
                ),
            )?;
            let mut database = Database::open(&paths)?;
            let project = database
                .link_project(LinkProjectInput {
                    path: project_path.clone(),
                    original_path: project_path.clone(),
                    primary_hostname: "project.test".to_owned(),
                    config_path,
                    desired_php_track: None,
                    additional_hostnames: Vec::new(),
                })?
                .project;
            database.replace_project_managed_resources(
                &project.id,
                &[ProjectManagedResourceInput {
                    resource_name: "mailpit".to_owned(),
                    track: "1.0".to_owned(),
                }],
            )?;
            database.record_managed_resource_track_env_context(
                "mailpit",
                "1.0",
                &BTreeMap::from([("smtp_host".to_owned(), "127.0.0.1".to_owned())]),
            )?;
            database.record_runtime_observed_snapshot(
                RuntimeSubject::Resource {
                    name: "mailpit".to_owned(),
                    track: "1.0".to_owned(),
                },
                RuntimeObservedStatus::Running,
                Some("fixture mailpit is ready"),
            )?;

            let result =
                reconcile_project_env_from_persisted_state(&paths, &mut database, &project.id);

            if check_tls {
                assert!(matches!(
                    result,
                    Err(DaemonError::State(StateError::Filesystem { ref path, .. }))
                        if path == &paths.ca_certificate()
                ));
            } else {
                assert!(matches!(
                    result,
                    Err(DaemonError::ProjectEnvDependenciesNotApplied { ref reason, .. })
                        if reason.contains("hostnames")
                ));
            }
            assert_eq!(
                database
                    .project_env_observed_state(&project.id)?
                    .map(|observed| observed.status),
                Some(ProjectEnvObservedStatus::Failed)
            );
            assert!(!state::fs::path_entry_exists(&project.path.join(".env"))?);
        }

        Ok(())
    }

    #[test]
    fn unsafe_resource_scopes_are_promoted_exactly() -> anyhow::Result<()> {
        let scopes = [
            ReconciliationScope::resource("php", "8.4")?,
            ReconciliationScope::resource("frankenphp", "8.4")?,
            ReconciliationScope::resource("caddy", "2")?,
            ReconciliationScope::resource("mailpit", "1.0")?,
        ];
        let effective_scopes = scopes
            .iter()
            .map(|scope| (scope.clone(), scope.effective()))
            .collect::<Vec<_>>();

        assert_debug_snapshot!(effective_scopes);

        Ok(())
    }

    #[test]
    fn system_coverage_excludes_failed_projects_and_their_resources() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let mut database = Database::open(&paths)?;
        let successful_path = tempdir.path().join("successful");
        let successful = database.link_project(LinkProjectInput {
            path: successful_path.clone(),
            original_path: successful_path.clone(),
            primary_hostname: "successful.test".to_owned(),
            config_path: successful_path.join("pv.yml"),
            desired_php_track: None,
            additional_hostnames: Vec::new(),
        })?;
        database.replace_project_managed_resources(
            &successful.project.id,
            &[ProjectManagedResourceInput {
                resource_name: "redis".to_owned(),
                track: "8.0".to_owned(),
            }],
        )?;
        let failed_path = tempdir.path().join("failed");
        let failed = database.link_project(LinkProjectInput {
            path: failed_path.clone(),
            original_path: failed_path.clone(),
            primary_hostname: "failed.test".to_owned(),
            config_path: failed_path.join("pv.yml"),
            desired_php_track: None,
            additional_hostnames: Vec::new(),
        })?;
        database.replace_project_managed_resources(
            &failed.project.id,
            &[ProjectManagedResourceInput {
                resource_name: "mailpit".to_owned(),
                track: "1.0".to_owned(),
            }],
        )?;
        drop(database);

        let report = SystemProjectReconciliationReport {
            total: 2,
            succeeded: 1,
            successful_project_ids: vec![successful.project.id],
            summaries: vec!["Project env current".to_owned()],
            failures: vec![DaemonError::ProjectReconciliation {
                project_label: "failed.test".to_owned(),
                source: Box::new(
                    StateError::ProjectNotFound {
                        target: failed.project.id,
                    }
                    .into(),
                ),
            }],
        };

        let coverage = completed_system_reconciliation_coverage(&paths, &report)?;
        let mut settings = Settings::clone_current();
        settings.add_filter(r#"id: "[a-z0-9]{10}""#, r#"id: "<project_id>""#);
        settings.bind(|| {
            assert_debug_snapshot!(coverage);
        });

        Ok(())
    }

    #[tokio::test]
    async fn system_reconciliation_verifies_php_for_project_linked_after_discovery()
    -> anyhow::Result<()> {
        let mut mixed_errors = Vec::new();
        for (update_path, fail_install, fail_env) in [
            (true, false, false),
            (false, false, false),
            (true, true, false),
            (false, true, false),
            (true, true, true),
            (false, true, true),
        ] {
            let tempdir = tempdir()?;
            let paths = PvPaths::for_home(tempdir.path().join("home"));
            seed_cached_php_pair(&paths, tempdir.path())?;
            let mut caddy_guard = SeededRuntimeGuard::with_gateway(paths.clone());
            if !update_path {
                seed_gateway_ports(&mut Database::open(&paths)?)?;
                let certified_key =
                    generate_simple_self_signed(vec!["pv-gateway.localhost".to_owned()])?;
                state::fs::write_sensitive_file(
                    &paths.ca_certificate(),
                    &certified_key.cert.pem(),
                )?;
                state::fs::write_sensitive_file(
                    &paths.ca_private_key(),
                    &certified_key.signing_key.serialize_pem(),
                )?;
            }
            if fail_install {
                let sha256 = sha256_file(&tempdir.path().join(PHP_TEST_ARCHIVE_FILE_NAME))?;
                state::fs::remove_file(
                    &paths
                        .downloads()
                        .join(format!("{sha256}-{PHP_TEST_ARCHIVE_FILE_NAME}")),
                )?;
            }
            let project_path = tempdir.path().join("project");
            let config_path = project_path.join("pv.yml");
            state::fs::write_sensitive_file(
                &config_path,
                "serve: false\nphp: \"8.5\"\nenv:\n  APP_NAME: late\n",
            )?;
            let malformed_env = "USER_VALUE=kept\n# >>> PV MANAGED\nAPP_NAME=old\n";
            let env_project = if fail_env {
                let env_project_path = tempdir.path().join("invalid-env");
                let env_config_path = env_project_path.join("pv.yml");
                state::fs::write_sensitive_file(
                    &env_config_path,
                    "serve: false\nenv:\n  APP_NAME: project\n",
                )?;
                state::fs::write_sensitive_file(&env_project_path.join(".env"), malformed_env)?;
                Some(
                    Database::open(&paths)?
                        .link_project_with_mode(
                            LinkProjectInput {
                                path: env_project_path.clone(),
                                original_path: env_project_path,
                                primary_hostname: "invalid-env.test".to_owned(),
                                config_path: env_config_path,
                                desired_php_track: None,
                                additional_hostnames: Vec::new(),
                            },
                            ProjectMode::ResourceOnly,
                        )?
                        .project,
                )
            } else {
                None
            };
            let catalog = crate::managed_resources::ManagedResourceRuntimeCatalog::without_adapters_with_manifest_client(
                OFFLINE_TEST_MANIFEST_URL,
                LinkingProjectArtifactClient {
                    inner: MultiArtifactClient {
                        manifest: state::fs::read_to_string(&paths.downloads().join("manifest.json"))?,
                        archives: BTreeMap::new(),
                    },
                    paths: paths.clone(),
                    project: LinkProjectInput {
                        path: project_path.clone(), original_path: project_path.clone(),
                        primary_hostname: "late.test".to_owned(), config_path,
                        desired_php_track: None, additional_hostnames: Vec::new(),
                    },
                },
            )?;
            assert!(
                Database::open(&paths)?
                    .projects()?
                    .iter()
                    .all(|project| project.path != project_path)
            );

            let job_id = if !update_path && fail_env {
                start_reconciliation_job(&paths, "system")?
            } else {
                "system-late-project-test".to_owned()
            };
            let phase_log = crate::structured_log::ReconciliationPhaseLog::new(
                &paths,
                &job_id,
                "reconcile",
                "system",
            );
            let progress = super::DaemonDownloadProgress::disabled();
            let mut streamed_error = None;
            let result = if update_path {
                reconcile_system_projects_and_resources_with_progress(
                    &paths,
                    Some(&catalog),
                    progress,
                    &phase_log,
                )
                .await
                .map(|report| {
                    if !fail_install {
                        assert_eq!((report.total, report.succeeded), (1, 1));
                        assert!(report.failures.is_empty());
                    }
                })
            } else if fail_env {
                let (client, daemon) = duplex(64 * 1024);
                stream_started_reconciliation_job(
                    paths.clone(),
                    protocol::transport(daemon),
                    true,
                    &job_id,
                    ReconciliationScope::System,
                    Some(&catalog),
                    ReconciliationJobTiming::immediate(),
                )
                .await?;
                let mut reader = protocol::transport(client);
                while let Some(line) = reader.next().await {
                    let event = serde_json::from_str::<serde_json::Value>(&line?)?;
                    if event["type"] == "job_failed" {
                        streamed_error = event["error"].as_str().map(str::to_owned);
                    }
                }
                let job = Database::open(&paths)?
                    .recent_jobs()?
                    .into_iter()
                    .find(|job| job.id == job_id)
                    .ok_or_else(|| anyhow::anyhow!("expected failed job"))?;
                assert_eq!(job.status, JobStatus::Failed);
                assert!(streamed_error.is_some());
                assert_eq!(streamed_error, job.error);
                Ok(())
            } else {
                complete_system_reconciliation_with_progress(
                    &paths,
                    Some(&catalog),
                    progress,
                    &phase_log,
                    None,
                    None,
                    None,
                )
                .await
                .map(|_| ())
            };
            let database = Database::open(&paths)?;
            assert!(
                database
                    .managed_resource_track("caddy", CADDY_TEST_TRACK)?
                    .current_artifact_path
                    .is_some()
            );
            let project = database
                .projects()?
                .into_iter()
                .find(|project| project.path == project_path)
                .ok_or_else(|| anyhow::anyhow!("expected late Project"))?;
            if !update_path {
                let phases = reconciliation_phase_events(&paths, &job_id)?;
                let expected = if fail_install {
                    [
                        "resources/desired_resources/succeeded",
                        "resources/desired_resources/failed",
                        "project_apply/linked_projects/failed",
                    ]
                } else {
                    [
                        "resources/desired_resources/succeeded",
                        "resources/desired_resources/succeeded",
                        "project_apply/linked_projects/succeeded",
                    ]
                };
                assert_eq!(
                    project_resource_phases(&phases, &project.id),
                    expected,
                    "phases were {:?}",
                    job_phase_outcomes(&phases)
                );
            }
            if fail_install {
                if let Some(env_project) = env_project {
                    let message = if let Some(message) = streamed_error {
                        message
                    } else {
                        let error = result
                            .err()
                            .ok_or_else(|| anyhow::anyhow!("expected mixed failure"))?;
                        let DaemonError::SystemReconciliationFailures { failures } = &error else {
                            anyhow::bail!("expected mixed system failure: {error}");
                        };
                        let [
                            DaemonError::ManagedResourceDefaultInstallFailures { .. },
                            DaemonError::ProjectReconciliation {
                                project_label: env_label,
                                source: env_source,
                            },
                            DaemonError::ProjectReconciliation {
                                project_label: install_label,
                                source: install_source,
                            },
                        ] = failures.as_slice()
                        else {
                            anyhow::bail!(
                                "expected resource, env, and Project installation failures: {failures:?}"
                            );
                        };
                        assert_eq!(env_label, "invalid-env");
                        assert!(matches!(
                            env_source.as_ref(),
                            DaemonError::Config(ConfigError::MalformedManagedEnvBlock { .. })
                        ));
                        assert_eq!(install_label, "late.test");
                        let DaemonError::ProjectResourceInstallation { source } =
                            install_source.as_ref()
                        else {
                            anyhow::bail!("expected Project installation source: {install_source}");
                        };
                        assert!(matches!(
                            source.as_ref(),
                            DaemonError::ManagedResourceArtifactMissing { resource, track }
                                if resource == "php" && track == PHP_TEST_TRACK
                        ));
                        error.to_string()
                    };
                    mixed_errors.push((update_path, message));
                    assert_eq!(
                        state::fs::read_to_string(&env_project.path.join(".env"))?,
                        malformed_env
                    );
                    assert_eq!(
                        database
                            .project_env_observed_state(&env_project.id)?
                            .map(|state| state.status),
                        Some(ProjectEnvObservedStatus::Failed)
                    );
                } else {
                    let error = result.err().ok_or_else(|| {
                        anyhow::anyhow!("expected installation and Project failure")
                    })?;
                    let error_message = error.to_string();
                    let DaemonError::SystemReconciliationFailures { failures: errors } = error
                    else {
                        anyhow::bail!("expected system failure: {error}");
                    };
                    let [
                        DaemonError::ManagedResourceDefaultInstallFailures { failures },
                        DaemonError::ProjectReconciliation {
                            project_label,
                            source,
                        },
                    ] = errors.as_slice()
                    else {
                        anyhow::bail!("expected resource and Project failures: {errors:?}");
                    };
                    assert_eq!(project_label, "late.test");
                    let DaemonError::ProjectResourceInstallation { source } = source.as_ref()
                    else {
                        anyhow::bail!("expected Project installation source: {source}");
                    };
                    assert!(matches!(
                        source.as_ref(),
                        DaemonError::ManagedResourceArtifactMissing { resource, track }
                            if resource == "php" && track == PHP_TEST_TRACK
                    ));
                    allow_duplicates! {
                        assert_snapshot!(error_message, @r#"System reconciliation failed: Managed Resource default installs failed: php/frankenphp 8.5: php 8.5: Managed Resource command failed: HTTP request failed for `https://artifacts.example.test/php-8.5.0-pv1-any.tar.gz`: missing scripted archive; late.test: Project application stopped after resource installation failed"#);
                    }
                    allow_duplicates! {
                        assert_debug_snapshot!(failures, @r#"
                        [
                            "php/frankenphp 8.5: php 8.5: Managed Resource command failed: HTTP request failed for `https://artifacts.example.test/php-8.5.0-pv1-any.tar.gz`: missing scripted archive",
                        ]
                        "#);
                    }
                }
                assert!(database.managed_resource_tracks()?.iter().all(|track| {
                    track.resource_name != "php" || track.current_artifact_path.is_none()
                }));
                assert_eq!(
                    database
                        .project_env_observed_state(&project.id)?
                        .map(|state| state.status),
                    Some(ProjectEnvObservedStatus::Failed)
                );
                assert!(!state::fs::path_exists(&project_path.join(".env")));
                if !update_path {
                    let phases = reconciliation_phase_events(&paths, &job_id)?;
                    assert!(
                        phases.iter().any(|phase| phase["phase"] == "resources"
                            && phase["outcome"] == "succeeded")
                    );
                    assert!(
                        phases.iter().any(|phase| phase["phase"] == "project_apply"
                            && phase["outcome"] == "failed")
                    );
                }
                caddy_guard.cleanup().await?;
                continue;
            }
            result?;
            for resource in ["php", "frankenphp"] {
                assert!(
                    database
                        .managed_resource_track(resource, PHP_TEST_TRACK)?
                        .current_artifact_path
                        .is_some(),
                    "late Project must install {resource}"
                );
            }
            assert_eq!(
                database
                    .project_env_observed_state(&project.id)?
                    .map(|state| state.status),
                Some(ProjectEnvObservedStatus::Rendered)
            );
            let rendered_env = state::fs::read_to_string(&project_path.join(".env"))?;
            allow_duplicates! {
                assert_snapshot!(rendered_env, @r"
                # >>> PV MANAGED
                APP_NAME=late
                # <<< PV MANAGED
                ");
            }
            caddy_guard.cleanup().await?;
        }
        assert_debug_snapshot!(mixed_errors, @r#"
        [
            (
                true,
                "System reconciliation failed: Managed Resource default installs failed: php/frankenphp 8.5: php 8.5: Managed Resource command failed: HTTP request failed for `https://artifacts.example.test/php-8.5.0-pv1-any.tar.gz`: missing scripted archive; invalid-env: Project config error: malformed PV-managed .env block: start marker without end marker; late.test: Project application stopped after resource installation failed",
            ),
            (
                false,
                "System reconciliation failed: Managed Resource default installs failed: php/frankenphp 8.5: php 8.5: Managed Resource command failed: HTTP request failed for `https://artifacts.example.test/php-8.5.0-pv1-any.tar.gz`: missing scripted archive; invalid-env: Project config error: malformed PV-managed .env block: start marker without end marker; late.test: Project application stopped after resource installation failed",
            ),
        ]
        "#);
        Ok(())
    }

    #[tokio::test]
    async fn system_reconciliation_retains_partial_project_env_failure() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        seed_installed_caddy(&paths)?;
        let project_path = tempdir.path().join("project");
        let config_path = project_path.join("pv.yml");
        state::fs::write_sensitive_file(&config_path, "serve: false\nenv:\n  APP_NAME: project\n")?;
        let env_path = project_path.join(".env");
        let env_before = "USER_VALUE=kept\n# >>> PV MANAGED\nAPP_NAME=old\n";
        state::fs::write_sensitive_file(&env_path, env_before)?;
        let linked = Database::open(&paths)?.link_project(LinkProjectInput {
            path: project_path.clone(),
            original_path: project_path,
            primary_hostname: "project.test".to_owned(),
            config_path,
            desired_php_track: None,
            additional_hostnames: Vec::new(),
        })?;
        let catalog = crate::managed_resources::fake_runtime_catalog(OFFLINE_TEST_MANIFEST_URL)?;

        let phase_log = crate::structured_log::ReconciliationPhaseLog::new(
            &paths,
            "system-partial-project-env-test",
            "reconcile",
            "system",
        );

        let report = reconcile_system_projects_and_resources_with_progress(
            &paths,
            Some(&catalog),
            super::DaemonDownloadProgress::disabled(),
            &phase_log,
        )
        .await?;

        assert_eq!((report.total, report.succeeded), (1, 0));
        assert_debug_snapshot!(system_project_summary(&report), @r#"
        Some(
            "Project env reconciled for 0 of 1 Projects; failures: project.test: Project config error: malformed PV-managed .env block: start marker without end marker",
        )
        "#);
        assert_eq!(state::fs::read_to_string(&env_path)?, env_before);
        assert_eq!(
            Database::open(&paths)?
                .project_env_observed_state(&linked.project.id)?
                .map(|state| state.status),
            Some(ProjectEnvObservedStatus::Failed)
        );
        Ok(())
    }

    #[tokio::test]
    async fn system_reconciliation_pins_discovered_php_track_across_manifest_refresh()
    -> anyhow::Result<()> {
        for (version, applied_version, initially_served) in [
            ("  version: latest\n", "  version: latest\n", false),
            ("  version: latest\n", "  version: latest\n", true),
            ("", "", true),
            ("", "  version: latest\n", true),
            ("  version: latest\n", "", true),
        ] {
            let tempdir = tempdir()?;
            let paths = PvPaths::for_home(tempdir.path().join("home"));
            let project_path = tempdir.path().join("project");
            let config_path = project_path.join("pv.yml");

            let mailpit_config = if version.is_empty() {
                "mailpit:\n  version: \"1.0\"\n"
            } else {
                ""
            };
            state::fs::write_sensitive_file(
                &config_path,
                &format!(
                    "serve: {initially_served}\nphp:\n{version}  extensions: [redis]\nenv:\n  APP_NAME: project\n{mailpit_config}"
                ),
            )?;
            seed_cached_php_pair(&paths, tempdir.path())?;
            let cached_manifest_path = paths.downloads().join("manifest.json");
            let cached_manifest = state::fs::read_to_string(&cached_manifest_path)?;
            let mut refreshed_manifest =
                serde_json::from_str::<serde_json::Value>(&cached_manifest)?;
            let resources = refreshed_manifest
                .get_mut("resources")
                .and_then(serde_json::Value::as_array_mut)
                .ok_or_else(|| anyhow::anyhow!("expected manifest resources"))?;
            for resource in resources {
                let resource = resource
                    .as_object_mut()
                    .ok_or_else(|| anyhow::anyhow!("expected manifest resource object"))?;
                let resource_name = resource
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("expected manifest resource name"))?;
                if !matches!(resource_name, "php" | "frankenphp") {
                    continue;
                }
                let tracks = resource
                    .get_mut("tracks")
                    .and_then(serde_json::Value::as_array_mut)
                    .ok_or_else(|| anyhow::anyhow!("expected manifest tracks"))?;
                let mut previous_track = tracks
                    .first()
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("expected manifest track"))?;
                let track_name = previous_track
                    .get_mut("name")
                    .ok_or_else(|| anyhow::anyhow!("expected manifest track name"))?;
                *track_name = json!("8.4");
                tracks.push(previous_track);
                resource.insert("default_track".to_owned(), json!("8.4"));
            }
            let refreshed_manifest = serde_json::to_string_pretty(&refreshed_manifest)?;
            seed_installed_caddy(&paths)?;
            let mut caddy_guard = SeededRuntimeGuard::with_gateway(paths.clone());
            caddy_guard.register_worker(
                "8.5+redis",
                &paths.resources().join("frankenphp").join(PHP_TEST_TRACK),
            );
            let mut database = Database::open(&paths)?;
            database.link_project(LinkProjectInput {
                path: project_path.clone(),
                original_path: project_path,
                primary_hostname: "project.test".to_owned(),
                config_path: config_path.clone(),
                desired_php_track: None,
                additional_hostnames: Vec::new(),
            })?;
            drop(database);
            let write_counter = Connection::open(paths.db().as_std_path())?;
            write_counter.execute_batch(
                "CREATE TABLE test_project_env_writes (count INTEGER NOT NULL);
             INSERT INTO test_project_env_writes (count) VALUES (0);
             CREATE TRIGGER test_project_env_insert
             AFTER INSERT ON observed_states
             WHEN NEW.subject_kind = 'project_env'
             BEGIN
                 UPDATE test_project_env_writes SET count = count + 1;
             END;
             CREATE TRIGGER test_project_env_update
             AFTER UPDATE ON observed_states
             WHEN NEW.subject_kind = 'project_env'
             BEGIN
                 UPDATE test_project_env_writes SET count = count + 1;
             END;",
            )?;
            let catalog = crate::managed_resources::ManagedResourceRuntimeCatalog::without_adapters_with_manifest_client(
            OFFLINE_TEST_MANIFEST_URL,
            ReconfiguringProjectArtifactClient {
                inner: MultiArtifactClient {
                    manifest: refreshed_manifest.clone(),
                    archives: BTreeMap::new(),
                },
                config_path,
                config: format!("serve: false\nphp:\n{applied_version}  extensions: [redis]\nenv:\n  APP_NAME: project\n"),
            },
        )?;

            let phase_log = crate::structured_log::ReconciliationPhaseLog::new(
                &paths,
                "system-demand-discovery-test",
                "reconcile",
                "system",
            );
            complete_system_reconciliation_with_progress(
                &paths,
                Some(&catalog),
                super::DaemonDownloadProgress::disabled(),
                &phase_log,
                None,
                None,
                None,
            )
            .await?;

            let database = Database::open(&paths)?;
            let project = database
                .projects()?
                .into_iter()
                .next()
                .ok_or_else(|| anyhow::anyhow!("expected linked project"))?;

            assert_eq!(project.php_runtime.track.as_deref(), Some(PHP_TEST_TRACK));
            assert_eq!(project.mode, ProjectMode::ResourceOnly);
            let installed_php_tracks = database
                .managed_resource_tracks()?
                .into_iter()
                .filter(|track| {
                    matches!(track.resource_name.as_str(), "php" | "frankenphp")
                        && track.current_artifact_path.is_some()
                })
                .map(|track| (track.resource_name, track.track))
                .collect::<Vec<_>>();
            assert_eq!(
                installed_php_tracks,
                [
                    ("frankenphp".to_owned(), PHP_TEST_TRACK.to_owned()),
                    ("php".to_owned(), PHP_TEST_TRACK.to_owned()),
                ]
            );
            assert_eq!(project.php_runtime.requested_extensions, ["redis"]);
            assert_eq!(project.php_runtime.loaded_extensions, ["redis"]);
            assert!(project.php_runtime.ignored_extensions.is_empty());
            assert_eq!(
                state::fs::read_to_string(&cached_manifest_path)?,
                refreshed_manifest
            );
            assert!(
                state::fs::read_to_string(&project.path.join(".env"))?.contains("APP_NAME=project")
            );
            let observed_writes = write_counter.query_row(
                "SELECT count FROM test_project_env_writes",
                [],
                |row| row.get::<_, i64>(0),
            )?;
            assert_eq!(observed_writes, 1);
            caddy_guard.cleanup().await?;
        }

        Ok(())
    }

    #[tokio::test]
    async fn system_install_failure_preserves_unrelated_php_project_application()
    -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        seed_cached_php_pair(&paths, tempdir.path())?;
        seed_installed_caddy(&paths)?;
        let mut caddy_guard = SeededRuntimeGuard::with_gateway(paths.clone());
        let manifest = state::fs::read_to_string(&paths.downloads().join("manifest.json"))?;
        let mut database = Database::open(&paths)?;
        let mut projects = Vec::new();
        for (name, config) in [
            (
                "ready",
                "serve: false\nphp:\n  version: \"8.5\"\nenv:\n  APP_NAME: applied\n",
            ),
            ("failed", "serve: false\nmailpit:\n  version: \"1.0\"\n"),
        ] {
            let project_path = tempdir.path().join(name);
            let config_path = project_path.join("pv.yml");
            state::fs::write_sensitive_file(&config_path, config)?;
            let linked = database.link_project(LinkProjectInput {
                path: project_path.clone(),
                original_path: project_path,
                primary_hostname: format!("{name}.test"),
                config_path,
                desired_php_track: None,
                additional_hostnames: Vec::new(),
            })?;
            projects.push(linked.project);
        }
        database.record_managed_resource_track_desired(
            "mailpit",
            MAILPIT_TEST_TRACK,
            ManagedResourceDesiredState::Installed,
        )?;
        drop(database);
        let catalog = crate::managed_resources::fake_runtime_catalog_with_manifest_client(
            OFFLINE_TEST_MANIFEST_URL,
            MultiArtifactClient {
                manifest,
                archives: BTreeMap::new(),
            },
        )?;
        let phase_log = crate::structured_log::ReconciliationPhaseLog::new(
            &paths,
            "system-install-failure-test",
            "reconcile",
            "system",
        );

        let result = complete_system_reconciliation_with_progress(
            &paths,
            Some(&catalog),
            super::DaemonDownloadProgress::disabled(),
            &phase_log,
            None,
            None,
            None,
        )
        .await;

        let error = result
            .err()
            .ok_or_else(|| anyhow::anyhow!("expected installation and Project failure"))?;
        let error_message = error.to_string();
        let DaemonError::SystemReconciliationFailures { failures: errors } = error else {
            anyhow::bail!("expected system failure: {error}");
        };
        let [
            DaemonError::ManagedResourceDefaultInstallFailures { failures },
            DaemonError::ProjectReconciliation {
                project_label,
                source,
            },
        ] = errors.as_slice()
        else {
            anyhow::bail!("expected resource and Project failures: {errors:?}");
        };
        assert_eq!(project_label, "failed.test");
        let DaemonError::ProjectResourceInstallation { source } = source.as_ref() else {
            anyhow::bail!("expected Project installation source: {source}");
        };
        assert!(matches!(
            source.as_ref(),
            DaemonError::ManagedResourceArtifactMissing { resource, track }
                if resource == "mailpit" && track == MAILPIT_TEST_TRACK
        ));
        allow_duplicates! {
            assert_snapshot!(error_message, @r#"System reconciliation failed: Managed Resource default installs failed: mailpit 1.0: Managed Resource command failed: artifact manifest does not include Managed Resource `mailpit`; failed.test: Project application stopped after resource installation failed"#);
        }
        assert_debug_snapshot!(failures, @r#"
        [
            "mailpit 1.0: Managed Resource command failed: artifact manifest does not include Managed Resource `mailpit`",
        ]
        "#);
        let database = Database::open(&paths)?;
        for (project, expected_status) in projects.iter().zip([
            ProjectEnvObservedStatus::Rendered,
            ProjectEnvObservedStatus::Failed,
        ]) {
            let state = database.project_env_observed_state(&project.id)?;
            assert_eq!(state.map(|state| state.status), Some(expected_status));
        }
        assert_snapshot!(
            state::fs::read_to_string(&projects[0].path.join(".env"))?,
            @r"
        # >>> PV MANAGED
        APP_NAME=applied
        # <<< PV MANAGED
        "
        );

        caddy_guard.cleanup().await?;
        Ok(())
    }

    #[tokio::test]
    async fn system_install_failure_preserves_previous_project_runtime() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        seed_installed_caddy(&paths)?;
        let mut caddy_guard = SeededRuntimeGuard::with_gateway(paths.clone());
        caddy_guard.register_resource("mailpit", MAILPIT_TEST_TRACK);
        seed_installed_artifact(
            &paths,
            "mailpit",
            MAILPIT_TEST_TRACK,
            MAILPIT_TEST_ARTIFACT_VERSION,
            "bin/pv-fake-mailpit",
        )?;
        state::fs::write_sensitive_file(
            &paths
                .resources()
                .join("mailpit/1.0/current/bin/pv-fake-mailpit"),
            FAKE_MAILPIT_SCRIPT,
        )?;
        set_executable(
            &paths
                .resources()
                .join("mailpit/1.0/current/bin/pv-fake-mailpit"),
        )?;
        let (client, _) = scripted_artifact_client(
            tempdir.path(),
            "mailpit",
            "1.1",
            "1.1.0-pv1",
            "mailpit-1.1.0-pv1-any.tar.gz",
            "bin/pv-fake-mailpit",
        )?;
        state::fs::write_sensitive_file(
            &paths.downloads().join("manifest.json"),
            &client.manifest,
        )?;
        let catalog = crate::managed_resources::fake_runtime_catalog_with_manifest_client(
            OFFLINE_TEST_MANIFEST_URL,
            MultiArtifactClient {
                manifest: client.manifest,
                archives: BTreeMap::new(),
            },
        )?;
        let project_path = tempdir.path().join("project");
        let config_path = project_path.join("pv.yml");
        let config =
            "serve: false\nmailpit:\n  version: \"1.0\"\n  env:\n    MAIL_PORT: \"${smtp_port}\"\n";
        state::fs::write_sensitive_file(&config_path, config)?;
        let mut database = Database::open(&paths)?;
        let linked = database.link_project(LinkProjectInput {
            path: project_path.clone(),
            original_path: project_path.clone(),
            primary_hostname: "project.test".to_owned(),
            config_path: config_path.clone(),
            desired_php_track: None,
            additional_hostnames: Vec::new(),
        })?;
        let mut port_guards = Vec::new();
        for port_name in ["smtp", "dashboard"] {
            let listener = TcpListener::bind(("127.0.0.1", 0))?;
            let port = listener.local_addr()?.port();
            database.assign_port(
                PortRequest::resource_port(
                    "mailpit",
                    MAILPIT_TEST_TRACK,
                    port_name,
                    port,
                    port,
                    port,
                ),
                |candidate| candidate == port,
            )?;
            port_guards.push(listener);
        }
        let smtp_address = port_guards[0].local_addr()?;
        drop(port_guards);
        drop(database);
        reconcile_project_env_with_runtime_catalog_and_progress(
            &paths,
            &linked.project.id,
            Some(&catalog),
            None,
            &BTreeSet::new(),
            super::DaemonDownloadProgress::disabled(),
            ProjectApplyOptions::new(ProjectApplyStage::CompleteApply, None),
        )
        .await?;
        let verification = async {
            let previous_env = state::fs::read_to_string(&project_path.join(".env"))?;
            let previous_pid =
                state::fs::read_to_string(&paths.resource_pid("mailpit", MAILPIT_TEST_TRACK))?;
            let previous_metadata = state::fs::read_to_string(
                &paths.resource_runtime_metadata("mailpit", MAILPIT_TEST_TRACK),
            )?;
            let ready_path = tempdir.path().join("ready");
            let ready_config = ready_path.join("pv.yml");
            state::fs::write_sensitive_file(
                &ready_config,
                "serve: false\nenv:\n  APP_NAME: independent\n",
            )?;
            Database::open(&paths)?.link_project(LinkProjectInput {
                path: ready_path.clone(),
                original_path: ready_path.clone(),
                primary_hostname: "ready.test".to_owned(),
                config_path: ready_config,
                desired_php_track: None,
                additional_hostnames: Vec::new(),
            })?;
            state::fs::write_sensitive_file(&config_path, &config.replace("1.0", "1.1"))?;
            let phase_log = crate::structured_log::ReconciliationPhaseLog::new(
                &paths,
                "system-replacement-failure-test",
                "reconcile",
                "system",
            );
            let result = complete_system_reconciliation_with_progress(
                &paths,
                Some(&catalog),
                super::DaemonDownloadProgress::disabled(),
                &phase_log,
                None,
                None,
                None,
            )
            .await;
            let database = Database::open(&paths)?;
            let resources = database.project_managed_resources(&linked.project.id)?;
            let snapshot = (
                resources
                    .into_iter()
                    .map(|resource| (resource.resource_name, resource.track))
                    .collect::<Vec<_>>(),
                database
                    .managed_resource_track("mailpit", MAILPIT_TEST_TRACK)?
                    .usage_count,
                database.managed_resource_tracks()?.iter().all(|track| {
                    track.resource_name != "mailpit"
                        || track.track != "1.1"
                        || track.current_artifact_path.is_none()
                }),
                state::fs::read_to_string(&project_path.join(".env"))? == previous_env,
                state::fs::read_to_string(&paths.resource_pid("mailpit", MAILPIT_TEST_TRACK))
                    .ok()
                    .as_deref()
                    == Some(&previous_pid),
                state::fs::read_to_string(
                    &paths.resource_runtime_metadata("mailpit", MAILPIT_TEST_TRACK),
                )
                .ok()
                .as_deref()
                    == Some(&previous_metadata),
                TcpStream::connect(smtp_address).is_ok(),
                database
                    .project_env_observed_state(&linked.project.id)?
                    .map(|state| state.status),
                state::fs::read_to_string(&ready_path.join(".env"))?,
            );
            Ok::<_, anyhow::Error>((result, snapshot))
        }
        .await;
        Database::open(&paths)?.replace_project_managed_resources(&linked.project.id, &[])?;
        let (result, snapshot) = verification?;
        let error = result
            .err()
            .ok_or_else(|| anyhow::anyhow!("expected installation and Project failure"))?;
        let error_message = error.to_string();
        let DaemonError::SystemReconciliationFailures { failures: errors } = error else {
            anyhow::bail!("expected system failure: {error}");
        };
        let [
            DaemonError::ManagedResourceDefaultInstallFailures { failures },
            DaemonError::ProjectReconciliation {
                project_label,
                source,
            },
        ] = errors.as_slice()
        else {
            anyhow::bail!("expected resource and Project failures: {errors:?}");
        };
        assert_eq!(project_label, "project.test");
        let DaemonError::ProjectResourceInstallation { source } = source.as_ref() else {
            anyhow::bail!("expected Project installation source: {source}");
        };
        assert!(matches!(
            source.as_ref(),
            DaemonError::ManagedResourceArtifactMissing { resource, track }
                if resource == "mailpit" && track == "1.1"
        ));
        allow_duplicates! {
            assert_snapshot!(error_message, @r#"System reconciliation failed: Managed Resource default installs failed: mailpit 1.1: Managed Resource command failed: HTTP request failed for `https://artifacts.example.test/mailpit-1.1.0-pv1-any.tar.gz`: missing scripted archive; project.test: Project application stopped after resource installation failed"#);
        }
        assert_debug_snapshot!(failures, @r#"
        [
            "mailpit 1.1: Managed Resource command failed: HTTP request failed for `https://artifacts.example.test/mailpit-1.1.0-pv1-any.tar.gz`: missing scripted archive",
        ]
        "#);
        assert_debug_snapshot!(snapshot, @r##"
        (
            [
                (
                    "mailpit",
                    "1.0",
                ),
            ],
            1,
            true,
            true,
            true,
            true,
            true,
            Some(
                Failed,
            ),
            "# >>> PV MANAGED\nAPP_NAME=independent\n# <<< PV MANAGED\n",
        )
        "##);
        caddy_guard.cleanup().await?;
        assert!(!paths.resource_pid("mailpit", MAILPIT_TEST_TRACK).exists());
        assert!(
            !paths
                .resource_runtime_metadata("mailpit", MAILPIT_TEST_TRACK)
                .exists()
        );
        assert!(TcpStream::connect(smtp_address).is_err());
        Ok(())
    }

    #[tokio::test]
    async fn system_reconciliation_recovers_initial_artifact_download_failure() -> anyhow::Result<()>
    {
        for (update_path, download_failures) in [(false, 1), (true, 1), (false, 2), (true, 2)] {
            let tempdir = tempdir()?;
            let paths = PvPaths::for_home(tempdir.path().join("home"));
            seed_cached_php_pair(&paths, tempdir.path())?;
            seed_installed_caddy(&paths)?;
            let mut caddy_guard = SeededRuntimeGuard::with_gateway(paths.clone());
            let archive_path = tempdir.path().join(PHP_TEST_ARCHIVE_FILE_NAME);
            let sha256 = sha256_file(&archive_path)?;
            state::fs::remove_file(
                &paths
                    .downloads()
                    .join(format!("{sha256}-{PHP_TEST_ARCHIVE_FILE_NAME}")),
            )?;
            let manifest = state::fs::read_to_string(&paths.downloads().join("manifest.json"))?;
            let project_path = tempdir.path().join("project");
            let config_path = project_path.join("pv.yml");
            state::fs::write_sensitive_file(
                &config_path,
                "serve: false\nphp:\n  version: \"8.5\"\nenv:\n  APP_NAME: recovered\n",
            )?;
            let linked = Database::open(&paths)?.link_project(LinkProjectInput {
                path: project_path.clone(),
                original_path: project_path,
                primary_hostname: "project.test".to_owned(),
                config_path,
                desired_php_track: None,
                additional_hostnames: Vec::new(),
            })?;
            let download_attempts = Arc::new(AtomicUsize::new(0));
            let manifest_requests = Arc::new(AtomicUsize::new(0));
            let catalog = crate::managed_resources::ManagedResourceRuntimeCatalog::without_adapters_with_manifest_client(
                OFFLINE_TEST_MANIFEST_URL,
                FailingArtifactClient {
                    inner: ScriptedArtifactClient { manifest, archive: read_file(&archive_path)? },
                    download_failures,
                    download_attempts: Arc::clone(&download_attempts),
                    manifest_requests: Arc::clone(&manifest_requests),
                },
            )?;
            let job_id = "system-recovered-download-test";
            let phase_log = crate::structured_log::ReconciliationPhaseLog::new(
                &paths,
                job_id,
                "reconcile",
                "system",
            );
            let progress = super::DaemonDownloadProgress::disabled();
            let result = if update_path {
                reconcile_system_projects_and_resources_with_progress(
                    &paths,
                    Some(&catalog),
                    progress,
                    &phase_log,
                )
                .await
                .map(|report| {
                    assert!(report.failures.is_empty());
                    assert_eq!(report.succeeded, 1);
                })
            } else {
                let result = complete_system_reconciliation_with_progress(
                    &paths,
                    Some(&catalog),
                    progress,
                    &phase_log,
                    None,
                    None,
                    None,
                )
                .await
                .map(|_| ());
                if download_failures == 1 {
                    // The retry is a second timed Resources phase that runs before Project Apply,
                    // and it suppresses nested operation records so artifact work is counted only
                    // by its owning phase.
                    let events = reconciliation_phase_events(&paths, job_id)?;
                    assert_eq!(
                        project_resource_phases(&events, &linked.project.id),
                        [
                            "resources/desired_resources/failed",
                            "resources/desired_resources/succeeded",
                            "project_apply/linked_projects/succeeded",
                        ],
                        "phases were {:?}",
                        job_phase_outcomes(&events)
                    );
                }
                result
            };
            assert_eq!(download_attempts.load(Ordering::SeqCst), 2);
            assert_eq!(manifest_requests.load(Ordering::SeqCst), 1);
            let database = Database::open(&paths)?;
            if download_failures == 2 {
                let error = result
                    .err()
                    .ok_or_else(|| anyhow::anyhow!("expected installation and Project failure"))?;
                let error_message = error.to_string();
                let DaemonError::SystemReconciliationFailures { failures: errors } = error else {
                    anyhow::bail!("expected system failure: {error}");
                };
                let [
                    DaemonError::ManagedResourceDefaultInstallFailures { failures },
                    DaemonError::ProjectReconciliation {
                        project_label,
                        source,
                    },
                ] = errors.as_slice()
                else {
                    anyhow::bail!("expected resource and Project failures: {errors:?}");
                };
                assert_eq!(project_label, "project.test");
                let DaemonError::ProjectResourceInstallation { source } = source.as_ref() else {
                    anyhow::bail!("expected Project installation source: {source}");
                };
                assert!(matches!(
                    source.as_ref(),
                    DaemonError::ManagedResourceArtifactMissing { resource, track }
                        if resource == "php" && track == PHP_TEST_TRACK
                ));
                allow_duplicates! {
                    assert_snapshot!(error_message, @r#"System reconciliation failed: Managed Resource default installs failed: php/frankenphp 8.5: php 8.5: Managed Resource command failed: HTTP status 410 for `https://artifacts.example.test/php-8.5.0-pv1-any.tar.gz`; project.test: Project application stopped after resource installation failed"#);
                }
                allow_duplicates! {
                    assert_debug_snapshot!(failures, @r#"
                    [
                        "php/frankenphp 8.5: php 8.5: Managed Resource command failed: HTTP status 410 for `https://artifacts.example.test/php-8.5.0-pv1-any.tar.gz`",
                    ]
                    "#);
                }
                assert!(database.managed_resource_tracks()?.iter().all(|track| {
                    track.resource_name != "php" || track.current_artifact_path.is_none()
                }));
                assert_eq!(
                    database
                        .project_env_observed_state(&linked.project.id)?
                        .map(|state| state.status),
                    Some(ProjectEnvObservedStatus::Failed)
                );
                assert!(!state::fs::path_exists(&linked.project.path.join(".env")));
                caddy_guard.cleanup().await?;
                continue;
            }
            result?;
            assert!(
                database
                    .managed_resource_track("php", PHP_TEST_TRACK)?
                    .current_artifact_path
                    .is_some()
            );
            assert_eq!(
                database
                    .project_env_observed_state(&linked.project.id)?
                    .map(|state| state.status),
                Some(ProjectEnvObservedStatus::Rendered)
            );
            let env = state::fs::read_to_string(&linked.project.path.join(".env"))?;
            allow_duplicates! {
                assert_snapshot!(env, @r"
            # >>> PV MANAGED
            APP_NAME=recovered
            # <<< PV MANAGED
            ");
            }
            caddy_guard.cleanup().await?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn system_reconciliation_retains_artifact_layout_failure() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let (client, _) = scripted_artifact_client(
            tempdir.path(),
            "caddy",
            CADDY_TEST_TRACK,
            CADDY_TEST_ARTIFACT_VERSION,
            CADDY_TEST_ARCHIVE_FILE_NAME,
            "bin/not-caddy",
        )?;
        let catalog = crate::managed_resources::ManagedResourceRuntimeCatalog::without_adapters_with_manifest_client(OFFLINE_TEST_MANIFEST_URL, client)?;

        let phase_log = crate::structured_log::ReconciliationPhaseLog::new(
            &paths,
            "system-artifact-layout-test",
            "reconcile",
            "system",
        );

        let result = reconcile_system_projects_and_resources_with_progress(
            &paths,
            Some(&catalog),
            super::DaemonDownloadProgress::disabled(),
            &phase_log,
        )
        .await;

        let Err(DaemonError::ManagedResourceDefaultInstallFailures { failures }) = result else {
            anyhow::bail!("expected invalid Caddy layout: {result:?}");
        };
        assert_debug_snapshot!(failures, @r#"
        [
            "caddy 2: Managed Resource command failed: invalid artifact layout for `caddy`: missing executable `bin/caddy`",
        ]
        "#);
        assert!(
            Database::open(&paths)?
                .managed_resource_track("caddy", CADDY_TEST_TRACK)?
                .current_artifact_path
                .is_none()
        );
        Ok(())
    }

    #[tokio::test]
    async fn system_recovery_uses_applied_project_demand() -> anyhow::Result<()> {
        for (update_path, replacement) in [(false, false), (true, false), (false, true)] {
            let tempdir = tempdir()?;
            let paths = PvPaths::for_home(tempdir.path().join("home"));
            seed_installed_caddy(&paths)?;
            let mut caddy_guard = SeededRuntimeGuard::with_gateway(paths.clone());
            let (client, _) = scripted_artifact_client(
                tempdir.path(),
                "mailpit",
                MAILPIT_TEST_TRACK,
                MAILPIT_TEST_ARTIFACT_VERSION,
                MAILPIT_TEST_ARCHIVE_FILE_NAME,
                "bin/pv-fake-mailpit",
            )?;
            state::fs::write_sensitive_file(
                &paths.downloads().join("manifest.json"),
                &client.manifest,
            )?;
            let project_path = tempdir.path().join("project");
            let config_path = project_path.join("pv.yml");
            state::fs::write_sensitive_file(
                &config_path,
                "serve: false\nmailpit:\n  version: \"1.0\"\n",
            )?;
            let linked = Database::open(&paths)?.link_project(LinkProjectInput {
                path: project_path.clone(),
                original_path: project_path,
                primary_hostname: "project.test".to_owned(),
                config_path: config_path.clone(),
                desired_php_track: None,
                additional_hostnames: Vec::new(),
            })?;
            let catalog = crate::managed_resources::fake_runtime_catalog_with_manifest_client(
                OFFLINE_TEST_MANIFEST_URL,
                ReconfiguringProjectArtifactClient {
                    inner: MultiArtifactClient {
                        manifest: client.manifest,
                        archives: BTreeMap::new(),
                    },
                    config_path,
                    config: if replacement {
                        "serve: false\nmailpit:\n  version: \"1.1\"\n".to_owned()
                    } else {
                        "serve: false\nenv:\n  APP_NAME: current\n".to_owned()
                    },
                },
            )?;
            let phase_log = crate::structured_log::ReconciliationPhaseLog::new(
                &paths,
                "system-current-demand-test",
                "reconcile",
                "system",
            );
            let progress = super::DaemonDownloadProgress::disabled();
            let result = if update_path {
                reconcile_system_projects_and_resources_with_progress(
                    &paths,
                    Some(&catalog),
                    progress,
                    &phase_log,
                )
                .await
                .map(|_| ())
            } else {
                complete_system_reconciliation_with_progress(
                    &paths,
                    Some(&catalog),
                    progress,
                    &phase_log,
                    None,
                    None,
                    None,
                )
                .await
                .map(|_| ())
            };
            let database = Database::open(&paths)?;
            assert!(
                database
                    .project_managed_resources(&linked.project.id)?
                    .is_empty()
            );
            assert!(database.managed_resource_tracks()?.iter().all(|track| {
                track.resource_name != "mailpit" || track.current_artifact_path.is_none()
            }));
            if replacement {
                let error = result
                    .err()
                    .ok_or_else(|| anyhow::anyhow!("expected installation and Project failure"))?;
                let error_message = error.to_string();
                let DaemonError::SystemReconciliationFailures { failures: errors } = error else {
                    anyhow::bail!("expected system failure: {error}");
                };
                let [
                    DaemonError::ManagedResourceDefaultInstallFailures { failures },
                    DaemonError::ProjectReconciliation {
                        project_label,
                        source,
                    },
                ] = errors.as_slice()
                else {
                    anyhow::bail!("expected resource and Project failures: {errors:?}");
                };
                assert_eq!(project_label, "project.test");
                let DaemonError::ProjectResourceInstallation { source } = source.as_ref() else {
                    anyhow::bail!("expected Project installation source: {source}");
                };
                assert!(matches!(
                    source.as_ref(),
                    DaemonError::ManagedResourceArtifactMissing { resource, track }
                        if resource == "mailpit" && track == "1.1"
                ));
                allow_duplicates! {
                    assert_snapshot!(error_message, @r#"System reconciliation failed: Managed Resource default installs failed: mailpit 1.1: installation is still pending; project.test: Project application stopped after resource installation failed"#);
                }
                assert_debug_snapshot!(failures, @r#"
                [
                    "mailpit 1.1: installation is still pending",
                ]
                "#);
                assert_eq!(
                    database
                        .project_env_observed_state(&linked.project.id)?
                        .map(|state| state.status),
                    Some(ProjectEnvObservedStatus::Failed)
                );
                assert!(!state::fs::path_exists(&linked.project.path.join(".env")));
                caddy_guard.cleanup().await?;
                continue;
            }
            result?;
            assert_eq!(
                database
                    .project_env_observed_state(&linked.project.id)?
                    .map(|state| state.status),
                Some(ProjectEnvObservedStatus::Rendered)
            );
            let env = state::fs::read_to_string(&linked.project.path.join(".env"))?;
            allow_duplicates! {
                assert_snapshot!(env, @r"
                # >>> PV MANAGED
                APP_NAME=current
                # <<< PV MANAGED
                ");
            }
            caddy_guard.cleanup().await?;
        }
        Ok(())
    }

    #[test]
    fn system_installation_verification_rejects_missing_active_pointer() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        seed_installed_caddy(&paths)?;
        state::fs::remove_file(&paths.resources().join("caddy/2/current"))?;
        let catalog = crate::managed_resources::ManagedResourceRuntimeCatalog::without_adapters()?;

        let result = super::verify_system_resources_after_project_application(
            &paths,
            Some(&catalog),
            None,
            &super::DaemonDownloadProgress::disabled(),
        );

        assert!(
            matches!(result, Err(DaemonError::ManagedResourceCommand(ManagedResourceCommandError::Resources(ResourcesError::InvalidArtifactLayout { resource, .. }))) if resource == "caddy")
        );
        assert!(!state::fs::path_exists(
            &paths.resources().join("caddy/2/current")
        ));
        Ok(())
    }

    #[tokio::test]
    async fn system_cleanup_failure_still_reconciles_gateway_and_preserves_errors()
    -> anyhow::Result<()> {
        for fail_gateway in [false, true] {
            let tempdir = tempdir()?;
            let paths = PvPaths::for_home(tempdir.path().join("home"));
            seed_cached_php_pair(&paths, tempdir.path())?;
            seed_installed_caddy(&paths)?;
            let mut caddy_guard = SeededRuntimeGuard::with_gateway(paths.clone());
            caddy_guard.register_worker(
                PHP_TEST_TRACK,
                &paths.resources().join("frankenphp").join(PHP_TEST_TRACK),
            );
            seed_installed_artifact(
                &paths,
                "mailpit",
                MAILPIT_TEST_TRACK,
                MAILPIT_TEST_ARTIFACT_VERSION,
                "bin/pv-fake-mailpit",
            )?;
            if fail_gateway {
                let caddy_path = paths.resources().join("caddy/2/current/bin/caddy");
                state::fs::write_sensitive_file(&caddy_path, "#!/bin/sh\nexit 1\n")?;
                set_executable(&caddy_path)?;
            }
            let manifest = state::fs::read_to_string(&paths.downloads().join("manifest.json"))?;
            let project_path = tempdir.path().join("project");
            let config_path = project_path.join("pv.yml");
            state::fs::write_sensitive_file(
                &config_path,
                "php:\n  version: \"8.5\"\nmailpit:\n  version: \"1.0\"\n",
            )?;
            let mut database = Database::open(&paths)?;
            let linked = database.link_project(LinkProjectInput {
                path: project_path.clone(),
                original_path: project_path,
                primary_hostname: "project.test".to_owned(),
                config_path: config_path.clone(),
                desired_php_track: None,
                additional_hostnames: Vec::new(),
            })?;
            drop(database);
            state::fs::write_sensitive_file(
                &paths.resource_pid("mailpit", MAILPIT_TEST_TRACK),
                "2147483647",
            )?;
            state::fs::write_sensitive_file(
                &paths.resource_runtime_metadata("mailpit", MAILPIT_TEST_TRACK),
                "not JSON",
            )?;
            let catalog = crate::managed_resources::fake_runtime_catalog_with_manifest_client(
                OFFLINE_TEST_MANIFEST_URL,
                ReconfiguringProjectArtifactClient {
                    inner: MultiArtifactClient {
                        manifest,
                        archives: BTreeMap::new(),
                    },
                    config_path,
                    config: "serve: false\nenv:\n  APP_NAME: applied\n".to_owned(),
                },
            )?;
            let job_id = "system-cleanup-failure-test";
            let phase_log = crate::structured_log::ReconciliationPhaseLog::new(
                &paths,
                job_id,
                "reconcile",
                "system",
            );

            let result = complete_system_reconciliation_with_progress(
                &paths,
                Some(&catalog),
                super::DaemonDownloadProgress::disabled(),
                &phase_log,
                None,
                None,
                None,
            )
            .await;

            let error = result
                .err()
                .ok_or_else(|| anyhow::anyhow!("expected cleanup failure"))?;
            if fail_gateway {
                let DaemonError::SystemReconciliationFailures { failures } = error else {
                    anyhow::bail!("expected both cleanup and Gateway errors: {error}");
                };
                assert!(matches!(
                    failures.as_slice(),
                    [
                        DaemonError::Json(_),
                        DaemonError::UnexpectedProtocolResponse { .. }
                    ]
                ));
            } else {
                assert!(matches!(error, DaemonError::Json(_)));
            }
            let database = Database::open(&paths)?;
            let project = database
                .project_by_id(&linked.project.id)?
                .ok_or_else(|| anyhow::anyhow!("expected linked project"))?;
            assert_eq!(project.mode, ProjectMode::ResourceOnly);
            assert!(database.project_managed_resources(&project.id)?.is_empty());
            let phases = reconciliation_phase_events(&paths, job_id)?;
            let expected_outcome = if fail_gateway { "failed" } else { "succeeded" };
            assert!(
                phases.iter().any(
                    |phase| phase["phase"] == "gateway" && phase["outcome"] == expected_outcome
                )
            );
            caddy_guard.cleanup().await?;
        }

        Ok(())
    }

    #[test]
    fn demand_discovery_is_read_only_and_deduplicates_resource_only_projects() -> anyhow::Result<()>
    {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let mut database = Database::open(&paths)?;
        let mut project_ids = Vec::new();

        for name in ["first", "second"] {
            let project_path = tempdir.path().join(name);
            let config_path = project_path.join("pv.yml");
            state::fs::write_sensitive_file(
                &config_path,
                "serve: false\nmailpit:\n  version: \"1.0\"\n",
            )?;
            let linked = database.link_project(LinkProjectInput {
                path: project_path.clone(),
                original_path: project_path,
                primary_hostname: format!("{name}.test"),
                config_path,
                desired_php_track: None,
                additional_hostnames: Vec::new(),
            })?;
            project_ids.push(linked.project.id);
        }
        let projects_before = database.projects()?;
        let tracks_before = database.managed_resource_tracks()?;
        drop(database);

        let demand = discover_system_project_demand(&paths)?;

        assert_eq!(demand.project_count, 2);
        assert_eq!(demand.fallback_count, 0);
        assert_eq!(
            demand.resource_tracks,
            BTreeSet::from([super::DemandedResourceTrack::new(
                "mailpit",
                MAILPIT_TEST_TRACK,
            )])
        );
        for project_id in &project_ids {
            let project_demand = demand
                .project_demands
                .get(project_id)
                .ok_or_else(|| anyhow::anyhow!("expected discovered project demand"))?;
            assert_eq!(project_demand.resource_tracks, demand.resource_tracks);
            assert!(project_demand.php_track.is_none());
            assert!(!project_demand.used_persisted_state);
        }
        let database = Database::open(&paths)?;
        assert_eq!(database.projects()?, projects_before);
        assert_eq!(database.managed_resource_tracks()?, tracks_before);
        for project_id in project_ids {
            assert!(database.project_managed_resources(&project_id)?.is_empty());
            assert!(database.project_env_observed_state(&project_id)?.is_none());
        }

        Ok(())
    }

    #[test]
    fn invalid_config_discovery_uses_persisted_last_valid_demand() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let project_path = tempdir.path().join("project");
        let config_path = project_path.join("pv.yml");
        state::fs::write_sensitive_file(&config_path, "php: [\n")?;
        let mut database = Database::open(&paths)?;
        let linked = database.link_project(LinkProjectInput {
            path: project_path.clone(),
            original_path: project_path,
            primary_hostname: "project.test".to_owned(),
            config_path,
            desired_php_track: None,
            additional_hostnames: Vec::new(),
        })?;
        database.replace_project_managed_resources(
            &linked.project.id,
            &[ProjectManagedResourceInput {
                resource_name: "redis".to_owned(),
                track: "8.0".to_owned(),
            }],
        )?;
        database.replace_project_php_runtime(
            &linked.project.id,
            Some(&ProjectPhpRuntimeInput {
                track: "8.5".to_owned(),
                requested_extensions: vec!["redis".to_owned()],
                loaded_extensions: vec!["redis".to_owned()],
                ignored_extensions: Vec::new(),
            }),
        )?;
        let projects_before = database.projects()?;
        let tracks_before = database.managed_resource_tracks()?;
        let resources_before = database.project_managed_resources(&linked.project.id)?;
        drop(database);

        let demand = discover_system_project_demand(&paths)?;

        assert_eq!(
            demand.resource_tracks,
            BTreeSet::from([
                super::DemandedResourceTrack::new("frankenphp", "8.5"),
                super::DemandedResourceTrack::new("php", "8.5"),
                super::DemandedResourceTrack::new("redis", "8.0"),
            ])
        );
        assert_eq!(demand.fallback_count, 1);
        let database = Database::open(&paths)?;
        assert_eq!(database.projects()?, projects_before);
        assert_eq!(database.managed_resource_tracks()?, tracks_before);
        assert_eq!(
            database.project_managed_resources(&linked.project.id)?,
            resources_before
        );
        assert!(
            database
                .project_env_observed_state(&linked.project.id)?
                .is_none()
        );

        Ok(())
    }

    #[tokio::test]
    async fn project_application_rereads_config_after_discovery() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let project_path = tempdir.path().join("project");
        let config_path = project_path.join("pv.yml");
        state::fs::write_sensitive_file(
            &config_path,
            "serve: false\nmailpit:\n  version: \"1.0\"\nenv:\n  APP_NAME: discovered\n",
        )?;
        seed_cached_php_pair(&paths, tempdir.path())?;
        let mut database = Database::open(&paths)?;
        let linked = database.link_project(LinkProjectInput {
            path: project_path.clone(),
            original_path: project_path.clone(),
            primary_hostname: "project.test".to_owned(),
            config_path: config_path.clone(),
            desired_php_track: None,
            additional_hostnames: Vec::new(),
        })?;
        database.record_managed_resource_track_installed(
            "mailpit",
            MAILPIT_TEST_TRACK,
            MAILPIT_TEST_ARTIFACT_VERSION,
            &paths.resources().join("mailpit/1.0/releases/1.0.0-pv1"),
        )?;
        drop(database);
        let demand = discover_system_project_demand(&paths)?;
        assert!(!project_path.join(".env").exists());
        // Mailpit leaves the config and nothing new is demanded, so the reread is proved by the
        // rendered env rather than by an artifact the Resources phase was never asked to install.
        state::fs::write_sensitive_file(&config_path, "serve: false\nenv:\n  APP_NAME: applied\n")?;
        let catalog = crate::managed_resources::fake_runtime_catalog(OFFLINE_TEST_MANIFEST_URL)?;
        let progress = super::DaemonDownloadProgress::disabled();

        reconcile_system_resources_with_runtime_catalog_and_progress(
            &paths,
            Some(&catalog),
            &demand.resource_tracks,
            progress.clone(),
        )
        .await?;
        let report = reconcile_system_projects_with_progress(
            &paths,
            Some(&catalog),
            &demand.resource_tracks,
            &demand.project_demands,
            &progress,
            ProjectApplyStage::CompleteStagedApply,
            &linked_projects(&paths)?,
            &BTreeSet::new(),
            None,
        )
        .await?;
        stop_undemanded_system_resource_runtimes(&paths, Some(&catalog)).await?;

        assert!(report.failures.is_empty(), "got {:?}", report.failures);
        assert_eq!(report.succeeded, 1);
        let env = state::fs::read_to_string(&project_path.join(".env"))?;
        assert!(env.contains("APP_NAME=applied"));
        assert!(!env.contains("APP_NAME=discovered"));
        let database = Database::open(&paths)?;
        let project = database
            .project_by_id(&linked.project.id)?
            .ok_or_else(|| anyhow::anyhow!("expected linked project"))?;
        assert_eq!(project.mode, ProjectMode::ResourceOnly);
        // The Project Apply installs nothing, so a pair the Resources phase never demanded stays
        // uninstalled even though its archives are cached.
        assert!(
            database
                .managed_resource_tracks()?
                .iter()
                .filter(|track| track.resource_name == "php" || track.resource_name == "frankenphp")
                .all(|track| track.current_artifact_path.is_none()),
            "neither PHP side may gain an installed artifact path, got {:?}",
            database.managed_resource_tracks()?
        );
        assert!(
            database
                .runtime_observed_states()?
                .into_iter()
                .any(|state| {
                    state.subject
                        == (RuntimeSubject::Resource {
                            name: "mailpit".to_owned(),
                            track: MAILPIT_TEST_TRACK.to_owned(),
                        })
                        && state.status == RuntimeObservedStatus::Stopped
                })
        );

        Ok(())
    }

    #[tokio::test]
    async fn project_php_demand_preserves_removed_pair_members() -> anyhow::Result<()> {
        for removed_resource in ["php", "frankenphp"] {
            let tempdir = tempdir()?;
            let paths = PvPaths::for_home(tempdir.path().join("home"));
            seed_cached_php_pair(&paths, tempdir.path())?;
            for resource in ["php", "frankenphp"] {
                seed_installed_artifact(
                    &paths,
                    resource,
                    "8.5",
                    "8.5.0-pv1",
                    &format!("bin/{resource}"),
                )?;
            }
            let project_path = tempdir.path().join("project");
            let config_path = project_path.join("pv.yml");
            state::fs::write_sensitive_file(&config_path, "serve: false\nphp: \"8.5\"\n")?;
            let mut database = Database::open(&paths)?;
            let linked = database.link_project(LinkProjectInput {
                path: project_path.clone(),
                original_path: project_path,
                primary_hostname: "project.test".to_owned(),
                config_path,
                desired_php_track: None,
                additional_hostnames: Vec::new(),
            })?;
            let before = database.record_managed_resource_track_removal_intent(
                removed_resource,
                "8.5",
                false,
                true,
            )?;
            let catalog =
                crate::managed_resources::fake_runtime_catalog(OFFLINE_TEST_MANIFEST_URL)?;
            let result = crate::project_env::reconcile_project_env_with_catalog(
                &paths,
                &mut database,
                &linked.project.id,
                &catalog,
            )
            .await;

            assert!(
                matches!(result, Err(DaemonError::ManagedResourceTrackRemoved { resource, track })
                if resource == removed_resource && track == "8.5")
            );
            assert_eq!(
                database.managed_resource_track(removed_resource, "8.5")?,
                before
            );
            assert_eq!(
                database
                    .project_env_observed_state(&linked.project.id)?
                    .map(|state| state.status),
                Some(ProjectEnvObservedStatus::Failed)
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn project_application_rereads_global_php_after_discovery() -> anyhow::Result<()> {
        let mut applied_tracks = Vec::new();
        for (name, config, global_track) in [
            ("served", "document_root: .\n", Some("8.5")),
            (
                "mapping",
                "serve: false\nphp:\n  extensions: []\n",
                Some("8.5"),
            ),
            ("new-global", "document_root: .\n", None),
            ("latest", "serve: false\nphp: latest\n", Some("8.5")),
            (
                "latest-to-global",
                "serve: false\nphp: latest\n",
                Some("8.5"),
            ),
            (
                "global-to-latest",
                "serve: false\nphp:\n  extensions: []\n",
                Some("8.5"),
            ),
        ] {
            let tempdir = tempdir()?;
            let paths = PvPaths::for_home(tempdir.path().join("home"));
            seed_cached_php_pair(&paths, tempdir.path())?;
            for (track, artifact_version) in [("8.5", "8.5.0-pv1"), ("8.4", "8.4.0-pv1")] {
                for resource in ["php", "frankenphp"] {
                    seed_installed_artifact(
                        &paths,
                        resource,
                        track,
                        artifact_version,
                        &format!("bin/{resource}"),
                    )?;
                }
            }
            let project_path = tempdir.path().join("project");
            let config_path = project_path.join("pv.yml");
            state::fs::write_sensitive_file(&config_path, config)?;
            let mut database = Database::open(&paths)?;
            let linked = database.link_project(LinkProjectInput {
                path: project_path.clone(),
                original_path: project_path,
                primary_hostname: "project.test".to_owned(),
                config_path: config_path.clone(),
                desired_php_track: None,
                additional_hostnames: Vec::new(),
            })?;
            if let Some(global_track) = global_track {
                database.record_global_php_default_track(global_track)?;
            }
            let demand = discover_system_project_demand(&paths)?;
            assert_eq!(
                demand.resource_tracks,
                BTreeSet::from([
                    super::DemandedResourceTrack::new("php", "8.5"),
                    super::DemandedResourceTrack::new("frankenphp", "8.5"),
                ])
            );

            database.record_global_php_default_track("8.4")?;
            if name == "latest-to-global" {
                state::fs::write_sensitive_file(
                    &config_path,
                    "serve: false\nphp:\n  extensions: []\n",
                )?;
            } else if name == "global-to-latest" {
                state::fs::write_sensitive_file(&config_path, "serve: false\nphp: latest\n")?;
            }
            if matches!(name, "latest" | "global-to-latest") {
                let manifest_path = paths.downloads().join("manifest.json");
                let manifest = state::fs::read_to_string(&manifest_path)?;
                state::fs::write_sensitive_file(
                    &manifest_path,
                    &manifest.replace("\"8.5\"", "\"8.4\""),
                )?;
            }
            drop(database);
            let catalog =
                crate::managed_resources::fake_runtime_catalog(OFFLINE_TEST_MANIFEST_URL)?;
            let report = reconcile_system_projects_with_progress(
                &paths,
                Some(&catalog),
                &demand.resource_tracks,
                &demand.project_demands,
                &super::DaemonDownloadProgress::disabled(),
                ProjectApplyStage::CompleteStagedApply,
                &linked_projects(&paths)?,
                &BTreeSet::new(),
                None,
            )
            .await?;

            assert_eq!((report.total, report.succeeded), (1, 1));
            assert!(report.failures.is_empty());
            let database = Database::open(&paths)?;
            assert_eq!(database.global_php_default_track()?.as_deref(), Some("8.4"));
            let project = database
                .project_by_id(&linked.project.id)?
                .ok_or_else(|| anyhow::anyhow!("expected linked Project"))?;
            applied_tracks.push((name, project.desired_php_track, project.php_runtime.track));
        }
        assert_debug_snapshot!(applied_tracks, @r#"
        [
            (
                "served",
                Some(
                    "8.4",
                ),
                Some(
                    "8.4",
                ),
            ),
            (
                "mapping",
                Some(
                    "8.4",
                ),
                Some(
                    "8.4",
                ),
            ),
            (
                "new-global",
                Some(
                    "8.4",
                ),
                Some(
                    "8.4",
                ),
            ),
            (
                "latest",
                Some(
                    "8.5",
                ),
                Some(
                    "8.5",
                ),
            ),
            (
                "latest-to-global",
                Some(
                    "8.4",
                ),
                Some(
                    "8.4",
                ),
            ),
            (
                "global-to-latest",
                Some(
                    "8.4",
                ),
                Some(
                    "8.4",
                ),
            ),
        ]
        "#);
        Ok(())
    }

    #[tokio::test]
    async fn project_reconciliation_refreshes_php_extensions_after_missing_php_install()
    -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let project_path = tempdir.path().join("project");
        let config_path = project_path.join("pv.yml");

        state::fs::write_sensitive_file(
            &config_path,
            "php:\n  version: \"8.5\"\n  extensions: [redis]\n",
        )?;
        seed_cached_php_pair(&paths, tempdir.path())?;
        let mut database = Database::open(&paths)?;
        let linked = database.link_project(LinkProjectInput {
            path: project_path.clone(),
            original_path: project_path,
            primary_hostname: "project.test".to_owned(),
            config_path,
            desired_php_track: None,
            additional_hostnames: Vec::new(),
        })?;
        database.finalize_project_reconciliation(ProjectReconciliationStateInput {
            project_id: linked.project.id.clone(),
            link: LinkProjectInput {
                path: linked.project.path.clone(),
                original_path: linked.project.original_path.clone(),
                primary_hostname: "project.test".to_owned(),
                config_path: linked.project.config_path.clone(),
                desired_php_track: None,
                additional_hostnames: Vec::new(),
            },
            mode: ProjectMode::ResourceOnly,
            php_runtime: None,
            env_status: ProjectEnvObservedStatus::Rendered,
            env_message: Some("fixture state".to_owned()),
            env_warnings: Vec::new(),
        })?;
        drop(database);
        let catalog =
            crate::managed_resources::ManagedResourceRuntimeCatalog::without_adapters_with_manifest_url(
                OFFLINE_TEST_MANIFEST_URL,
            )?;

        reconcile_project_env_and_missing_resources(&paths, &linked.project.id, Some(&catalog))
            .await?;

        let database = Database::open(&paths)?;
        let project = database
            .project_by_id(&linked.project.id)?
            .ok_or_else(|| anyhow::anyhow!("expected linked project"))?;

        assert_eq!(project.php_runtime.track.as_deref(), Some(PHP_TEST_TRACK));
        assert_eq!(project.php_runtime.requested_extensions, ["redis"]);
        assert_eq!(project.php_runtime.loaded_extensions, ["redis"]);
        assert!(project.php_runtime.ignored_extensions.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn project_reconciliation_reports_disjoint_project_and_resource_phases()
    -> anyhow::Result<()> {
        for (scenario, label, expected_phases) in [
            (
                ProjectPhaseScenario::DeclaredBackingResource,
                "declared backing resource",
                vec![
                    "project_apply/project/succeeded",
                    "resources/desired_resources/skipped",
                    "project_apply/project/failed",
                ],
            ),
            (
                ProjectPhaseScenario::InvalidProjectConfig,
                "invalid project config",
                vec!["project_apply/project/failed"],
            ),
            (
                ProjectPhaseScenario::MissingPhpArtifact,
                "missing PHP artifact",
                vec![
                    "project_apply/project/succeeded",
                    "resources/desired_resources/failed",
                    "project_apply/project/succeeded",
                ],
            ),
            (
                ProjectPhaseScenario::UnreadablePhpExtensions,
                "unreadable PHP extensions",
                vec![
                    "project_apply/project/succeeded",
                    "resources/desired_resources/succeeded",
                    "project_apply/project/failed",
                ],
            ),
            (
                ProjectPhaseScenario::DeclaredBackingResourceInstallFailure,
                "declared backing resource install failure",
                vec![
                    "project_apply/project/succeeded",
                    "resources/desired_resources/failed",
                ],
            ),
        ] {
            let scenario_tempdir = tempdir()?;
            let scenario_paths = PvPaths::for_home(scenario_tempdir.path().join("home"));
            let (scenario_events, scenario_project_id, scenario_succeeded) =
                run_project_php_extension_reconciliation(
                    &scenario_paths,
                    scenario_tempdir.path(),
                    scenario,
                )
                .await?;

            assert!(!scenario_succeeded, "expected {label} to fail the job");
            assert_eq!(
                project_resource_phases(&scenario_events, &scenario_project_id),
                expected_phases,
                "unexpected phases for {label}; all phases were {:?}",
                job_phase_outcomes(&scenario_events)
            );
            assert_phase_time_within_execution(&scenario_events)?;
        }

        let success_tempdir = tempdir()?;
        let success_paths = PvPaths::for_home(success_tempdir.path().join("home"));
        let (events, project_id, _succeeded) = run_project_php_extension_reconciliation(
            &success_paths,
            success_tempdir.path(),
            ProjectPhaseScenario::Success,
        )
        .await?;

        // The stub FrankenPHP artifact cannot serve, so the job still fails at the worker
        // stage. What matters here is that nothing before Gateway work is blamed for it.
        assert_eq!(
            failed_phases(&events),
            ["workers/php_workers", "finalization/job"],
            "resource installation must not be blamed for a worker failure"
        );
        // This cold fixture has no active Gateway snapshot, so targeted proof safely promotes
        // to System after the Project-scoped resource stages complete.
        assert_eq!(
            project_resource_phases(&events, &project_id),
            [
                "project_apply/project/succeeded",
                "resources/desired_resources/succeeded",
                "project_apply/project/succeeded",
                "resources/desired_resources/succeeded",
                "project_apply/linked_projects/succeeded",
            ],
            "installing a missing artifact must be owned by Resources"
        );
        assert_phase_time_within_execution(&events)?;

        // This scenario's manifest URL is unreachable and its cached copy is seeded, so the
        // install falls back. Suppressing the operation phase must not suppress that
        // diagnostic, and the diagnostic must not itself be a phase record.
        let fallback = daemon_log_events(&success_paths)?
            .into_iter()
            .find(|event| event["event"] == "artifact_manifest_fallback")
            .ok_or_else(|| anyhow::anyhow!("missing artifact_manifest_fallback event"))?;
        assert_eq!(fallback["level"], "warn");
        assert_eq!(fallback["scope"], format!("project:{project_id}"));
        assert_eq!(fallback["manifest_source"], "cached");
        assert!(
            fallback["fallback_reason"]
                .as_str()
                .is_some_and(|reason| !reason.is_empty()),
            "fallback reason must be recorded, got {fallback:?}"
        );
        assert!(
            fallback.get("phase").is_none() && fallback.get("elapsed_ms").is_none(),
            "the fallback diagnostic must not be a phase record, got {fallback:?}"
        );

        let (settled_events, _settled_succeeded) = run_scenario_reconciliation_job(
            &success_paths,
            format!("project:{project_id}").parse::<ReconciliationScope>()?,
        )
        .await?;

        assert_eq!(
            project_resource_phases(&settled_events, &project_id),
            [
                "project_apply/project/succeeded",
                "resources/desired_resources/skipped",
                "project_apply/project/succeeded",
                "resources/desired_resources/succeeded",
                "project_apply/linked_projects/succeeded",
            ],
            "a settled project must still report Resources and complete the apply"
        );
        assert_phase_time_within_execution(&settled_events)?;

        // A Project that demands no artifact work must not be charged with installing
        // unrelated desired tracks, even though they are missing.
        let idle_tempdir = tempdir()?;
        let idle_paths = PvPaths::for_home(idle_tempdir.path().join("home"));
        let (idle_events, idle_project_id, idle_succeeded) =
            run_project_php_extension_reconciliation(
                &idle_paths,
                idle_tempdir.path(),
                ProjectPhaseScenario::NoProjectArtifactDemand,
            )
            .await?;

        assert!(
            idle_succeeded,
            "a Project with no artifact demand must not fail; phases were {:?}",
            job_phase_outcomes(&idle_events)
        );
        assert_eq!(
            project_resource_phases(&idle_events, &idle_project_id),
            [
                "project_apply/project/succeeded",
                "resources/desired_resources/skipped",
                "project_apply/project/succeeded",
            ],
            "Resources must not install tracks the Project never demanded"
        );
        assert_phase_time_within_execution(&idle_events)?;

        // A declared track with no adapter but seeded env context has no artifact to install,
        // so Resources must leave it alone and the apply must still tolerate it.
        let seeded_tempdir = tempdir()?;
        let seeded_paths = PvPaths::for_home(seeded_tempdir.path().join("home"));
        let (seeded_events, seeded_project_id, seeded_succeeded) =
            run_project_php_extension_reconciliation(
                &seeded_paths,
                seeded_tempdir.path(),
                ProjectPhaseScenario::SeededResourceWithoutAdapter,
            )
            .await?;

        assert!(
            seeded_succeeded,
            "a seeded adapterless track must not fail the job; phases were {:?}",
            job_phase_outcomes(&seeded_events)
        );
        assert_eq!(
            project_resource_phases(&seeded_events, &seeded_project_id),
            [
                "project_apply/project/succeeded",
                "resources/desired_resources/skipped",
                "project_apply/project/succeeded",
            ],
            "Resources must skip a declared track that has no artifact to install"
        );
        assert_phase_time_within_execution(&seeded_events)?;

        // When the repair pass and the apply both fail, the apply is primary but the repair
        // failure that preceded it is preserved alongside it.
        let both_tempdir = tempdir()?;
        let both_paths = PvPaths::for_home(both_tempdir.path().join("home"));
        let (both_events, both_project_id, both_succeeded) =
            run_project_php_extension_reconciliation(
                &both_paths,
                both_tempdir.path(),
                ProjectPhaseScenario::RepairAndApplyFailure,
            )
            .await?;

        assert!(!both_succeeded, "expected the combined failure to fail");
        assert_eq!(
            project_resource_phases(&both_events, &both_project_id),
            [
                "project_apply/project/succeeded",
                "resources/desired_resources/failed",
                "project_apply/project/failed",
            ],
            "a deferred repair failure must still let the apply run and be reported"
        );
        assert_phase_time_within_execution(&both_events)?;

        let failure = Database::open(&both_paths)?
            .recent_jobs()?
            .into_iter()
            .find(|job| job.scope == format!("project:{both_project_id}"))
            .and_then(|job| job.error)
            .ok_or_else(|| anyhow::anyhow!("missing persisted job failure"))?;

        assert!(
            failure.contains("Project apply failed with"),
            "apply must be the primary failure, got {failure}"
        );
        assert!(
            failure.contains("Managed Resource runtime `mailpit` is not supported yet"),
            "the apply failure must be the primary cause, got {failure}"
        );
        assert!(
            failure.contains("resource repair also failed"),
            "repair failure must be preserved, got {failure}"
        );
        assert!(
            failure.contains("php 8.5"),
            "the repair failure must name the PHP track it could not install, got {failure}"
        );
        assert!(
            failure.contains("frankenphp 8.5"),
            "the repair failure must name the FrankenPHP track it could not install, got {failure}"
        );
        assert!(
            failure.contains("HTTP request failed for"),
            "the repair failure detail must be preserved, got {failure}"
        );

        Ok(())
    }

    /// A Managed Resource scope reconciles linked Projects too, so it stages them around its
    /// own Resources phase rather than applying them in one pass.
    #[tokio::test]
    async fn resource_scoped_reconciliation_stages_project_apply_around_resources()
    -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let project_path = tempdir.path().join("project");
        let config_path = project_path.join("pv.yml");

        state::fs::write_sensitive_file(&config_path, "serve: false\n")?;
        let mut database = Database::open(&paths)?;
        // Desired but missing, and unrelated to the scope, so it must not be installed here.
        database.record_managed_resource_track_desired(
            "redis",
            "8.8",
            state::ManagedResourceDesiredState::Installed,
        )?;
        let linked = database.link_project(LinkProjectInput {
            path: project_path.clone(),
            original_path: project_path,
            primary_hostname: "project.test".to_owned(),
            config_path,
            desired_php_track: None,
            additional_hostnames: Vec::new(),
        })?;
        database.replace_project_managed_resources(
            &linked.project.id,
            &[ProjectManagedResourceInput {
                resource_name: "mailpit".to_owned(),
                track: MAILPIT_TEST_TRACK.to_owned(),
            }],
        )?;
        database.record_managed_resource_track_env_context(
            "mailpit",
            MAILPIT_TEST_TRACK,
            &BTreeMap::from([("smtp_host".to_owned(), "127.0.0.1".to_owned())]),
        )?;
        database.record_runtime_observed_snapshot(
            RuntimeSubject::Resource {
                name: "mailpit".to_owned(),
                track: MAILPIT_TEST_TRACK.to_owned(),
            },
            RuntimeObservedStatus::Running,
            Some("fixture mailpit readiness diagnostic"),
        )?;
        database.finalize_project_reconciliation(ProjectReconciliationStateInput {
            project_id: linked.project.id.clone(),
            link: LinkProjectInput {
                path: linked.project.path.clone(),
                original_path: linked.project.original_path.clone(),
                primary_hostname: "project.test".to_owned(),
                config_path: linked.project.config_path.clone(),
                desired_php_track: None,
                additional_hostnames: Vec::new(),
            },
            mode: ProjectMode::ResourceOnly,
            php_runtime: None,
            env_status: ProjectEnvObservedStatus::Rendered,
            env_message: Some("fixture state".to_owned()),
            env_warnings: Vec::new(),
        })?;
        drop(database);

        let (events, succeeded) = run_scenario_reconciliation_job(
            &paths,
            format!("resource:mailpit:{MAILPIT_TEST_TRACK}").parse::<ReconciliationScope>()?,
        )
        .await?;

        assert!(
            succeeded,
            "expected the resource scoped job to succeed; phases were {:?}",
            job_phase_outcomes(&events)
        );
        assert_eq!(
            project_resource_phases(&events, &linked.project.id),
            [
                "project_apply/linked_projects/succeeded",
                format!("resources/mailpit:{MAILPIT_TEST_TRACK}/skipped").as_str(),
                "project_apply/linked_projects/succeeded",
            ],
            "a Managed Resource scope must stage Project Apply around its own Resources phase"
        );
        assert_phase_time_within_execution(&events)?;

        Ok(())
    }

    /// Whole-system reconciliation records per-Project failures instead of stopping, so
    /// Resources and the final Project Apply still run.
    #[tokio::test]
    async fn system_reconciliation_continues_after_a_project_failure() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let project_path = tempdir.path().join("project");
        let config_path = project_path.join("pv.yml");

        state::fs::write_sensitive_file(&config_path, "bogus_key: true\n")?;
        // The catalog is offline, so Resources needs a cached manifest to fall back to;
        // without one it fails before the final Project Apply could run at all.
        seed_cached_php_pair(&paths, tempdir.path())?;
        let mut database = Database::open(&paths)?;
        let linked = database.link_project(LinkProjectInput {
            path: project_path.clone(),
            original_path: project_path,
            primary_hostname: "project.test".to_owned(),
            config_path,
            desired_php_track: None,
            additional_hostnames: Vec::new(),
        })?;
        drop(database);

        let (events, succeeded) =
            run_scenario_reconciliation_job(&paths, ReconciliationScope::System).await?;

        assert_eq!(
            project_resource_phases(&events, &linked.project.id),
            [
                "resources/desired_resources/succeeded",
                "project_apply/linked_projects/failed",
            ],
            "Resources must run before the failed Project Apply"
        );
        // Demand Discovery does not fail on the invalid config; it falls back to the Project's
        // last applied demand and reports that fallback.
        let discovery = events
            .iter()
            .find(|event| event["phase"] == "demand_discovery")
            .ok_or_else(|| anyhow::anyhow!("expected a demand discovery phase, got {events:?}"))?;

        assert_eq!(discovery["outcome"], "fallback", "in {discovery:?}");
        assert_eq!(discovery["project_count"], 1, "in {discovery:?}");
        assert_eq!(discovery["fallback_count"], 1, "in {discovery:?}");
        // The Project failure never stops the job; the seeded Caddy is a stub that cannot
        // serve, so the job fails later at the Gateway instead.
        assert_eq!(
            failed_phases(&events),
            [
                "project_apply/linked_projects",
                "gateway/gateway",
                "finalization/job",
            ],
            "only the Gateway may end the job; phases were {:?}",
            job_phase_outcomes(&events)
        );
        assert!(!succeeded, "the stub Gateway must fail the job");
        for event in events
            .iter()
            .filter(|event| event["phase"] == "project_apply")
        {
            assert_eq!(event["project_count"], 1, "in {event:?}");
            assert_eq!(event["succeeded_count"], 0, "in {event:?}");
            assert_eq!(event["failed_count"], 1, "in {event:?}");
        }
        let finalization = events
            .iter()
            .find(|event| event["phase"] == "finalization")
            .ok_or_else(|| anyhow::anyhow!("missing finalization phase"))?;
        assert_eq!(finalization["outcome"], "failed");
        assert_phase_time_within_execution(&events)?;

        Ok(())
    }

    /// A staged apply refuses an artifact that is missing at the time it runs, rather than
    /// doing artifact work the Resources phase never covered.
    #[tokio::test]
    async fn staged_project_apply_refuses_to_install_a_missing_artifact() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let project_path = tempdir.path().join("project");
        let config_path = project_path.join("pv.yml");

        state::fs::write_sensitive_file(
            &config_path,
            "mailpit:\n  version: \"1.0\"\n  env:\n    MAIL_HOST: \"${smtp_host}\"\n",
        )?;
        // An unusable manifest, so any attempt to discover or download the artifact fails with
        // a manifest error instead of the refusal this guard expects.
        let resource_client = ScriptedArtifactClient {
            manifest: "not valid manifest JSON".to_owned(),
            archive: Vec::new(),
        };
        let mut database = Database::open(&paths)?;
        let linked = database.link_project(LinkProjectInput {
            path: project_path.clone(),
            original_path: project_path,
            primary_hostname: "project.test".to_owned(),
            config_path,
            desired_php_track: None,
            additional_hostnames: Vec::new(),
        })?;
        drop(database);
        let catalog = crate::managed_resources::fake_runtime_catalog_with_manifest_client(
            OFFLINE_TEST_MANIFEST_URL,
            resource_client,
        )?;

        let staged = reconcile_project_env_with_runtime_catalog_and_progress(
            &paths,
            &linked.project.id,
            Some(&catalog),
            None,
            &BTreeSet::new(),
            DaemonDownloadProgress::disabled(),
            ProjectApplyOptions::new(ProjectApplyStage::CompleteStagedApply, None),
        )
        .await;

        let Err(DaemonError::ProjectResourceInstallation { source }) = staged else {
            anyhow::bail!("staged apply must refuse a missing artifact, got {staged:?}");
        };
        assert!(
            matches!(
                source.as_ref(),
                DaemonError::ManagedResourceArtifactMissing {
                    resource,
                    track,
                } if resource == "mailpit" && track == MAILPIT_TEST_TRACK
            ),
            "staged apply must retain the missing-artifact cause, got {source:?}"
        );
        assert!(
            !state::fs::path_exists(&paths.resources().join("mailpit").join(MAILPIT_TEST_TRACK)),
            "staged apply must not install the missing artifact"
        );

        Ok(())
    }

    #[tokio::test]
    async fn stream_write_error_is_returned_after_job_completion_is_persisted() -> anyhow::Result<()>
    {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        seed_installed_caddy(&paths)?;
        let mut caddy_guard = SeededRuntimeGuard::with_gateway(paths.clone());
        let job_id = start_reconciliation_job(&paths, "system")?;
        let (client, server) = duplex(64);
        drop(client);

        let result = stream_started_reconciliation_job(
            paths.clone(),
            protocol::transport(server),
            true,
            &job_id,
            ReconciliationScope::System,
            None,
            ReconciliationJobTiming::immediate(),
        )
        .await;

        assert!(result.is_err());
        let database = Database::open(&paths)?;
        let job = database
            .recent_jobs()?
            .into_iter()
            .find(|job| job.id == job_id)
            .ok_or_else(|| anyhow::anyhow!("missing job {job_id}"))?;
        assert_eq!(
            job.status,
            JobStatus::Succeeded,
            "unexpected persisted job error: {:?}",
            job.error
        );

        caddy_guard.cleanup().await?;
        Ok(())
    }

    #[tokio::test]
    async fn foreground_update_records_actual_queue_wait() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        Database::open(&paths)?;
        let queue = ReconciliationQueue::new();
        let blocker = queued(enqueue_reconciliation_job(
            &paths,
            &queue,
            ReconciliationScope::System,
        )?)?;
        let blocker = blocker.wait_for_turn().await;
        let update = queued(enqueue_update_job(&paths, &queue)?)?;

        tokio::time::sleep(Duration::from_millis(10)).await;
        blocker.finish();
        let running = timeout(Duration::from_secs(1), update.wait_for_turn()).await?;
        let job_id = running.job_id().to_owned();
        let catalog = crate::managed_resources::ManagedResourceRuntimeCatalog::without_adapters_with_manifest_client(
            OFFLINE_TEST_MANIFEST_URL,
            ScriptedArtifactClient {
                manifest: serde_json::to_string(&json!({
                    "schema_version": 1,
                    "minimum_pv_version": "0.1.0",
                    "resources": [],
                }))?,
                archive: Vec::new(),
            },
        )?;
        let (_client, daemon) = duplex(64);

        stream_started_update_job(
            paths.clone(),
            protocol::transport(daemon),
            false,
            &job_id,
            Some(&catalog),
            running.timing(),
            None,
        )
        .await
        .into_result()?;
        running.finish();

        let phases = reconciliation_phase_events(&paths, &job_id)?;
        let queue_phase = phases
            .first()
            .ok_or_else(|| anyhow::anyhow!("missing update queue phase"))?;
        assert_eq!(queue_phase["phase"], "queue");
        assert!(
            queue_phase["elapsed_ms"]
                .as_u64()
                .is_some_and(|elapsed| elapsed >= 10)
        );

        Ok(())
    }

    #[tokio::test]
    async fn no_op_update_preserves_prior_reconciliation_failure_coverage() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        seed_cached_php_pair(&paths, tempdir.path())?;
        seed_installed_caddy(&paths)?;

        let (prior_system_failure_id, prior_gateway_failure_id) = {
            let mut database = Database::open(&paths)?;
            let system_failure = database.start_job("reconcile", "system")?;
            database.fail_job_with_subject(
                &system_failure.id,
                "prior system reconciliation failure",
                &state::JobDiagnosticSubject::SystemReconciliation,
            )?;
            let gateway_failure = database.start_job("reconcile", "gateway")?;
            database.fail_job_with_subject(
                &gateway_failure.id,
                "prior gateway reconciliation failure",
                &state::JobDiagnosticSubject::GatewayRuntime,
            )?;
            database.record_managed_resource_track_desired(
                "php",
                PHP_TEST_TRACK,
                state::ManagedResourceDesiredState::Installed,
            )?;

            (system_failure.id, gateway_failure.id)
        };
        let manifest = state::fs::read_to_string(&paths.downloads().join("manifest.json"))?;
        let catalog = crate::managed_resources::ManagedResourceRuntimeCatalog::without_adapters_with_manifest_client(
            OFFLINE_TEST_MANIFEST_URL,
            MultiArtifactClient {
                manifest,
                archives: BTreeMap::new(),
            },
        )?;
        let job_id = start_update_job(&paths)?;

        let summary = complete_update_job(&paths, &job_id, Some(&catalog)).await?;

        assert_eq!(summary, "current");
        let database = Database::open(&paths)?;
        let unresolved = database.unresolved_job_failures()?;
        assert_eq!(unresolved.len(), 2);
        assert!(unresolved.iter().any(|failure| {
            failure.job.id == prior_system_failure_id
                && failure.subject == state::JobDiagnosticSubject::SystemReconciliation
        }));
        assert!(unresolved.iter().any(|failure| {
            failure.job.id == prior_gateway_failure_id
                && failure.subject == state::JobDiagnosticSubject::GatewayRuntime
        }));
        let php_track = database
            .managed_resource_tracks()?
            .into_iter()
            .find(|record| record.resource_name == "php" && record.track == PHP_TEST_TRACK)
            .ok_or_else(|| anyhow::anyhow!("missing desired PHP track"))?;
        assert_eq!(
            php_track.desired_state,
            state::ManagedResourceDesiredState::Installed
        );
        assert!(php_track.current_artifact_path.is_none());
        assert!(!state::fs::path_entry_exists(&paths.gateway_pid())?);

        Ok(())
    }

    #[tokio::test]
    async fn partial_update_reconciliation_failure_reports_completed_artifacts()
    -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));

        let composer_update_version = "2.8.1-pv1";
        let composer_update_archive_path = tempdir.path().join("composer-2.8.1-pv1-any.tar.gz");
        seed_artifact_archive(
            tempdir.path(),
            &composer_update_archive_path,
            "composer",
            composer_update_version,
            "composer.phar",
        )?;
        let composer_update_archive = read_file(&composer_update_archive_path)?;
        let composer_update_url = "https://artifacts.example.test/composer-2.8.1-pv1-any.tar.gz";
        let caddy_update_url = "https://artifacts.example.test/caddy-2.11.5-pv1-any.tar.gz";
        let manifest = serde_json::to_string(&json!({
            "schema_version": 1,
            "minimum_pv_version": "0.1.0",
            "resources": [
                {
                    "name": "composer",
                    "default_track": COMPOSER_TEST_TRACK,
                    "tracks": [{
                        "name": COMPOSER_TEST_TRACK,
                        "artifacts": [manifest_artifact(
                            composer_update_version,
                            "2.8.1",
                            composer_update_url,
                            &sha256_file(&composer_update_archive_path)?,
                            composer_update_archive.len() as u64,
                        )],
                    }],
                },
                {
                    "name": "caddy",
                    "default_track": CADDY_TEST_TRACK,
                    "tracks": [{
                        "name": CADDY_TEST_TRACK,
                        "artifacts": [manifest_artifact(
                            "2.11.5-pv1",
                            "2.11.5",
                            caddy_update_url,
                            &"0".repeat(64),
                            1,
                        )],
                    }],
                },
            ],
        }))?;
        seed_installed_artifact(
            &paths,
            "composer",
            COMPOSER_TEST_TRACK,
            COMPOSER_TEST_ARTIFACT_VERSION,
            "composer.phar",
        )?;
        seed_installed_artifact(
            &paths,
            "caddy",
            CADDY_TEST_TRACK,
            CADDY_TEST_ARTIFACT_VERSION,
            "bin/caddy",
        )?;

        let catalog = crate::managed_resources::ManagedResourceRuntimeCatalog::without_adapters_with_manifest_client(
            OFFLINE_TEST_MANIFEST_URL,
            MultiArtifactClient {
                manifest,
                archives: BTreeMap::from([(
                    composer_update_url.to_string(),
                    composer_update_archive,
                )]),
            },
        )?;
        Database::open(&paths)?.record_managed_resource_track_desired(
            "composer",
            "3",
            ManagedResourceDesiredState::Installed,
        )?;
        let job_id = start_update_job(&paths)?;
        let events = update_events(paths.clone(), &job_id, &catalog).await?;
        let streamed_error = events
            .iter()
            .find(|event| {
                event.get("type").and_then(serde_json::Value::as_str) == Some("job_failed")
            })
            .and_then(|event| event.get("error"))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing streamed update failure"))?;
        let database = Database::open(&paths)?;
        let job = database
            .recent_jobs()?
            .into_iter()
            .find(|job| job.id == job_id)
            .ok_or_else(|| anyhow::anyhow!("missing update job {job_id}"))?;
        let persisted_error = job
            .error
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("missing persisted update failure"))?;
        assert_eq!(streamed_error, persisted_error);
        let failure = database
            .unresolved_job_failures()?
            .into_iter()
            .find(|failure| failure.job.id == job_id)
            .ok_or_else(|| anyhow::anyhow!("missing unresolved update failure {job_id}"))?;
        let composer_track = database.managed_resource_track("composer", COMPOSER_TEST_TRACK)?;
        let composer_current = state::fs::read_link(
            &paths
                .resources()
                .join("composer")
                .join(COMPOSER_TEST_TRACK)
                .join("current"),
        )?;

        let mut settings = Settings::clone_current();
        settings.add_filter(r"pid \d+", "pid <pid>");
        settings.bind(|| {
            assert_debug_snapshot!(
                "partial_update_reconciliation_and_reporting",
                (
                    streamed_error,
                    job.status,
                    failure.subject,
                    composer_track.installed_version,
                    composer_current,
                )
            );
        });

        Ok(())
    }

    #[tokio::test]
    async fn caddy_update_rolls_back_artifact_and_recovers_gateway_after_failure()
    -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        seed_installed_caddy(&paths)?;
        let mut caddy_guard = SeededRuntimeGuard::with_gateway(paths.clone());
        let (client, _total_bytes) = scripted_artifact_client(
            tempdir.path(),
            "caddy",
            CADDY_TEST_TRACK,
            "2.11.5-pv1",
            "caddy-2.11.5-pv1-any.tar.gz",
            "bin/caddy",
        )?;
        let catalog = crate::managed_resources::ManagedResourceRuntimeCatalog::without_adapters_with_manifest_client(
            OFFLINE_TEST_MANIFEST_URL,
            client,
        )?;
        let job_id = start_update_job(&paths)?;

        let update_error = complete_update_job(&paths, &job_id, Some(&catalog))
            .await
            .err()
            .ok_or_else(|| anyhow::anyhow!("Caddy update unexpectedly succeeded"))?;
        assert!(
            !matches!(
                &update_error,
                DaemonError::CaddyUpdateCompensationFailed { .. }
            ),
            "Caddy compensation unexpectedly failed: {update_error}"
        );

        let old_release = paths
            .resources()
            .join("caddy")
            .join(CADDY_TEST_TRACK)
            .join("releases")
            .join(CADDY_TEST_ARTIFACT_VERSION);
        let new_release = paths
            .resources()
            .join("caddy")
            .join(CADDY_TEST_TRACK)
            .join("releases")
            .join("2.11.5-pv1");
        let current_path = paths
            .resources()
            .join("caddy")
            .join(CADDY_TEST_TRACK)
            .join("current");
        assert_eq!(
            state::fs::read_link(&current_path)?,
            Utf8PathBuf::from(format!("releases/{CADDY_TEST_ARTIFACT_VERSION}")),
        );
        assert!(state::fs::path_entry_exists(&old_release)?);
        assert!(state::fs::path_entry_exists(&new_release)?);

        let database = Database::open(&paths)?;
        let caddy_track = database
            .managed_resource_tracks()?
            .into_iter()
            .find(|record| record.resource_name == "caddy" && record.track == CADDY_TEST_TRACK)
            .ok_or_else(|| anyhow::anyhow!("missing Caddy track"))?;
        assert_eq!(
            caddy_track.installed_version.as_deref(),
            Some(CADDY_TEST_ARTIFACT_VERSION)
        );
        assert_eq!(caddy_track.current_artifact_path, Some(old_release));
        assert!(
            state::fs::path_entry_exists(&paths.gateway_pid())?,
            "Gateway PID missing after expected update failure: {update_error}"
        );

        let job = database
            .recent_jobs()?
            .into_iter()
            .find(|job| job.id == job_id)
            .ok_or_else(|| anyhow::anyhow!("missing update job {job_id}"))?;
        assert_eq!(job.status, JobStatus::Failed);
        let failure = database
            .unresolved_job_failures()?
            .into_iter()
            .find(|failure| failure.job.id == job_id)
            .ok_or_else(|| anyhow::anyhow!("missing update failure {job_id}"))?;
        assert_eq!(failure.subject, JobDiagnosticSubject::UpdateAssessment);

        caddy_guard.cleanup().await?;
        Ok(())
    }

    #[test]
    fn caddy_rollback_restores_new_pointer_when_database_update_fails() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        seed_installed_caddy(&paths)?;
        let (client, _total_bytes) = scripted_artifact_client(
            tempdir.path(),
            "caddy",
            CADDY_TEST_TRACK,
            "2.11.5-pv1",
            "caddy-2.11.5-pv1-any.tar.gz",
            "bin/caddy",
        )?;
        let catalog = crate::managed_resources::ManagedResourceRuntimeCatalog::without_adapters_with_manifest_client(
            OFFLINE_TEST_MANIFEST_URL,
            client,
        )?;
        let report = crate::managed_resources::update_installed_with_progress(
            paths.clone(),
            Some(&catalog),
            &super::DaemonDownloadProgress::disabled(),
        )?;
        let database_lock = Connection::open(paths.db().as_std_path())?;
        database_lock.execute_batch("BEGIN EXCLUSIVE")?;

        let rollback_error = report
            .rollback_caddy(&paths)
            .err()
            .ok_or_else(|| anyhow::anyhow!("Caddy rollback unexpectedly succeeded"))?;

        database_lock.execute_batch("ROLLBACK")?;
        assert!(matches!(
            rollback_error,
            crate::DaemonError::ManagedResourceCommand(
                resources::ManagedResourceCommandError::State(_)
            )
        ));
        let new_release = paths
            .resources()
            .join("caddy")
            .join(CADDY_TEST_TRACK)
            .join("releases")
            .join("2.11.5-pv1");
        let current_path = paths
            .resources()
            .join("caddy")
            .join(CADDY_TEST_TRACK)
            .join("current");
        assert_eq!(
            state::fs::read_link(&current_path)?,
            Utf8PathBuf::from("releases/2.11.5-pv1"),
        );
        assert!(state::fs::path_entry_exists(&new_release)?);
        let caddy_track =
            Database::open(&paths)?.managed_resource_track("caddy", CADDY_TEST_TRACK)?;
        assert_eq!(caddy_track.installed_version.as_deref(), Some("2.11.5-pv1"));
        assert_eq!(caddy_track.current_artifact_path, Some(new_release));

        Ok(())
    }

    #[tokio::test]
    async fn streamed_reconciliation_returns_after_progress_write_times_out() -> anyhow::Result<()>
    {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        seed_installed_caddy(&paths)?;
        let mut caddy_guard = SeededRuntimeGuard::with_gateway(paths.clone());
        let (client, _total_bytes) = scripted_artifact_client(
            tempdir.path(),
            "composer",
            COMPOSER_TEST_TRACK,
            COMPOSER_TEST_ARTIFACT_VERSION,
            COMPOSER_TEST_ARCHIVE_FILE_NAME,
            "composer.phar",
        )?;
        let mut database = Database::open(&paths)?;
        database.record_managed_resource_track_desired(
            "composer",
            COMPOSER_TEST_TRACK,
            ManagedResourceDesiredState::Installed,
        )?;
        drop(database);
        let job_id = start_reconciliation_job(&paths, "system")?;
        let catalog = Arc::new(
            crate::managed_resources::ManagedResourceRuntimeCatalog::without_adapters_with_manifest_client(
                OFFLINE_TEST_MANIFEST_URL,
                DelayedScriptedArtifactClient::new(client, Duration::from_millis(200)),
            )?,
        );
        let (blocked_write_sender, blocked_write_receiver) = oneshot::channel();
        let task_catalog = Arc::clone(&catalog);
        let task_paths = paths.clone();
        let task_job_id = job_id.clone();
        let mut task = tokio::spawn(async move {
            stream_started_reconciliation_job(
                task_paths,
                protocol::transport(InitiallyWritableStream::with_blocked_write_signal(
                    2,
                    blocked_write_sender,
                )),
                true,
                &task_job_id,
                ReconciliationScope::System,
                Some(task_catalog.as_ref()),
                ReconciliationJobTiming::immediate(),
            )
            .await
        });

        let blocked_write_result = timeout(
            STREAMED_RECONCILIATION_PROGRESS_SETUP_TIMEOUT,
            blocked_write_receiver,
        )
        .await;
        let completion_result =
            timeout(STREAMED_RECONCILIATION_COMPLETION_TIMEOUT, &mut task).await;
        let task_result = match completion_result {
            Ok(result) => result,
            Err(_error) => {
                task.abort();
                let cleanup_result = task.await;
                return Err(anyhow::anyhow!(
                    "streamed reconciliation exceeded the progress-write assertion budget; completion cleanup result: {cleanup_result:?}"
                ));
            }
        };
        task_result??;
        let blocked_write_started_at = blocked_write_result
            .map_err(|_error| {
                anyhow::anyhow!(
                    "streamed reconciliation did not reach the blocked progress write during setup"
                )
            })?
            .map_err(|_error| anyhow::anyhow!("streamed reconciliation task dropped early"))?;
        let progress_write_elapsed = blocked_write_started_at.elapsed();
        assert!(
            progress_write_elapsed >= FOREGROUND_JOB_STREAM_WRITE_TIMEOUT,
            "progress write returned before its timeout budget: {progress_write_elapsed:?}"
        );
        let database = Database::open(&paths)?;
        let job = database
            .recent_jobs()?
            .into_iter()
            .find(|job| job.id == job_id)
            .ok_or_else(|| anyhow::anyhow!("missing job {job_id}"))?;
        assert_eq!(job.status, JobStatus::Succeeded);

        caddy_guard.cleanup().await?;
        Ok(())
    }

    #[tokio::test]
    async fn foreground_system_reconciliation_streams_setup_download_progress() -> anyhow::Result<()>
    {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let (resource_client, total_bytes) = scripted_artifact_client(
            tempdir.path(),
            "composer",
            COMPOSER_TEST_TRACK,
            COMPOSER_TEST_ARTIFACT_VERSION,
            COMPOSER_TEST_ARCHIVE_FILE_NAME,
            "bin/composer",
        )?;

        let mut database = Database::open(&paths)?;
        database.record_managed_resource_track_desired(
            "composer",
            COMPOSER_TEST_TRACK,
            state::ManagedResourceDesiredState::Installed,
        )?;
        drop(database);
        let job_id = start_reconciliation_job(&paths, "system")?;
        let catalog = crate::managed_resources::ManagedResourceRuntimeCatalog::without_adapters_with_manifest_client(
            OFFLINE_TEST_MANIFEST_URL,
            resource_client,
        )?;
        let events = reconciliation_events(
            paths.clone(),
            &job_id,
            ReconciliationScope::System,
            &catalog,
        )
        .await?;
        let download_progress = events
            .iter()
            .filter(|event| event["type"] == "download_progress")
            .cloned()
            .collect::<Vec<_>>();

        assert_eq!(
            download_progress.first(),
            Some(&json!({
                "type": "download_progress",
                "job_id": job_id,
                "resource": "composer",
                "track": COMPOSER_TEST_TRACK,
                "artifact_version": COMPOSER_TEST_ARTIFACT_VERSION,
                "downloaded_bytes": 0,
                "total_bytes": total_bytes,
            }))
        );
        // Downloading still streams progress to the foreground command, but the artifact work
        // is timed once by the Resources phase that owns it rather than by records nested
        // inside that phase.
        let phases = reconciliation_phase_events(&paths, &job_id)?;
        let resources = phases
            .iter()
            .find(|event| event["phase"] == "resources")
            .ok_or_else(|| anyhow::anyhow!("missing resources phase event"))?;
        // This fixture seeds only the Composer artifact, so the pass fails after downloading
        // it. What matters here is that Resources owns and times that work.
        assert_eq!(
            resources["outcome"],
            "failed",
            "phases were {:?}",
            job_phase_outcomes(&phases)
        );
        assert!(resources["elapsed_ms"].as_u64().is_some());
        for phase in ["manifest", "download", "install"] {
            assert!(
                !phases.iter().any(|event| event["phase"] == phase),
                "{phase} must not be timed inside the Resources phase"
            );
        }
        let live_phases = live_phase_names(&events);
        assert_eq!(
            live_phases,
            [
                "demand_discovery",
                "resources",
                "manifest",
                "download",
                "install",
                "resources",
                "download",
                "install",
                "project_apply",
                "workers",
                "gateway",
                "finalization",
            ]
        );

        Ok(())
    }

    #[tokio::test]
    async fn foreground_reconciliation_streams_manifest_before_held_fetch_finishes()
    -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let (resource_client, _total_bytes) = scripted_artifact_client(
            tempdir.path(),
            "composer",
            COMPOSER_TEST_TRACK,
            COMPOSER_TEST_ARTIFACT_VERSION,
            COMPOSER_TEST_ARCHIVE_FILE_NAME,
            "bin/composer",
        )?;
        let (release_sender, release_receiver) = mpsc::channel();
        let held_client = HeldManifestArtifactClient {
            inner: resource_client,
            release_receiver: Mutex::new(release_receiver),
            started: None,
        };
        let mut database = Database::open(&paths)?;
        database.record_managed_resource_track_desired(
            "composer",
            COMPOSER_TEST_TRACK,
            ManagedResourceDesiredState::Installed,
        )?;
        drop(database);
        let job_id = start_reconciliation_job(&paths, "system")?;
        let catalog = Arc::new(
            crate::managed_resources::ManagedResourceRuntimeCatalog::without_adapters_with_manifest_client(
                OFFLINE_TEST_MANIFEST_URL,
                held_client,
            )?,
        );
        let (client, daemon) = duplex(64 * 1024);
        let task_paths = paths.clone();
        let task_job_id = job_id.clone();
        let task_catalog = Arc::clone(&catalog);
        let mut task = tokio::spawn(async move {
            stream_started_reconciliation_job(
                task_paths,
                protocol::transport(daemon),
                true,
                &task_job_id,
                ReconciliationScope::System,
                Some(task_catalog.as_ref()),
                ReconciliationJobTiming::immediate(),
            )
            .await
        });
        let mut reader = protocol::transport(client);
        let phase_result = timeout(Duration::from_secs(1), async {
            while let Some(line) = reader.next().await {
                let event = serde_json::from_str::<serde_json::Value>(&line?)?;
                if event["type"] == "progress" && event["message"] == "manifest" {
                    return Ok::<serde_json::Value, anyhow::Error>(event);
                }
            }

            Err(anyhow::anyhow!(
                "job stream ended before the manifest phase"
            ))
        })
        .await;
        let operation_was_held = !task.is_finished();
        let release_result = release_sender
            .send(())
            .map_err(|_error| anyhow::anyhow!("held manifest request dropped"));
        let completion_result = timeout(Duration::from_secs(10), &mut task).await;

        release_result?;
        let phase = phase_result
            .map_err(|_error| anyhow::anyhow!("manifest phase arrived only after held work"))??;
        assert!(operation_was_held);
        assert_eq!(
            phase,
            json!({
                "type": "progress",
                "job_id": job_id,
                "message": "manifest",
            })
        );
        completion_result
            .map_err(|_error| anyhow::anyhow!("held reconciliation did not finish"))???;

        Ok(())
    }

    #[tokio::test]
    async fn failed_system_reconciliation_streams_and_persists_failure_phases() -> anyhow::Result<()>
    {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let mut database = Database::open(&paths)?;
        database.record_managed_resource_track_desired(
            "composer",
            COMPOSER_TEST_TRACK,
            state::ManagedResourceDesiredState::Installed,
        )?;
        drop(database);
        let job_id = start_reconciliation_job(&paths, "system")?;
        let catalog = crate::managed_resources::ManagedResourceRuntimeCatalog::without_adapters_with_manifest_client(
            OFFLINE_TEST_MANIFEST_URL,
            ScriptedArtifactClient {
                manifest: "not valid manifest JSON".to_owned(),
                archive: Vec::new(),
            },
        )?;
        let (client, daemon) = duplex(64 * 1024);

        stream_started_reconciliation_job(
            paths.clone(),
            protocol::transport(daemon),
            true,
            &job_id,
            ReconciliationScope::System,
            Some(&catalog),
            ReconciliationJobTiming::immediate(),
        )
        .await?;

        let mut reader = protocol::transport(client);
        let mut events = Vec::new();
        while let Some(line) = reader.next().await {
            events.push(serde_json::from_str::<serde_json::Value>(&line?)?);
        }
        assert!(
            events
                .iter()
                .any(|event| { event["type"] == "job_failed" && event["job_id"] == job_id })
        );
        let live_phases = live_phase_names(&events);
        assert_eq!(
            live_phases,
            [
                "demand_discovery",
                "resources",
                "manifest",
                "resources",
                "manifest",
                "project_apply",
                "workers",
                "gateway",
                "finalization",
            ]
        );
        let database = Database::open(&paths)?;
        let job = database
            .recent_jobs()?
            .into_iter()
            .find(|job| job.id == job_id)
            .ok_or_else(|| anyhow::anyhow!("missing job {job_id}"))?;
        assert_eq!(job.status, JobStatus::Failed);
        let failed_event = events
            .iter()
            .find(|event| event["type"] == "job_failed")
            .ok_or_else(|| anyhow::anyhow!("expected streamed failure"))?;
        assert_eq!(failed_event["error"].as_str(), job.error.as_deref());
        assert_snapshot!(job.error.as_deref().ok_or_else(|| anyhow::anyhow!("expected persisted failure"))?, @r"Managed Resource default installs failed: caddy 2: Managed Resource command failed: invalid artifact manifest: expected ident at line 1 column 2; composer 2: Managed Resource command failed: invalid artifact manifest: expected ident at line 1 column 2");
        let phases = reconciliation_phase_events(&paths, &job_id)?;
        // The manifest failure is reported by the Resources phase that owns it; timing it
        // separately inside that phase would nest one timer inside another. A persistent failure
        // is retried once before Project Apply, so two Resources phases own it.
        for (phase, expected_count) in [("manifest", 0), ("resources", 2), ("finalization", 1)] {
            let matching = phases
                .iter()
                .filter(|event| event["phase"] == phase && event["outcome"] == "failed")
                .count();
            assert_eq!(
                matching, expected_count,
                "unexpected {phase} failure phase count"
            );
        }

        Ok(())
    }

    #[tokio::test]
    async fn foreground_project_reconciliation_streams_download_progress() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let project_path = tempdir.path().join("project");
        let config_path = project_path.join("pv.yml");

        state::fs::write_sensitive_file(
            &config_path,
            "mailpit:\n  version: \"1.0\"\n  env:\n    MAIL_HOST: \"${smtp_host}\"\n",
        )?;
        let (resource_client, total_bytes) = scripted_artifact_client(
            tempdir.path(),
            "mailpit",
            MAILPIT_TEST_TRACK,
            MAILPIT_TEST_ARTIFACT_VERSION,
            MAILPIT_TEST_ARCHIVE_FILE_NAME,
            "bin/pv-fake-mailpit",
        )?;
        let mut database = Database::open(&paths)?;
        let linked = database.link_project(LinkProjectInput {
            path: project_path.clone(),
            original_path: project_path,
            primary_hostname: "project.test".to_owned(),
            config_path,
            desired_php_track: None,
            additional_hostnames: Vec::new(),
        })?;
        drop(database);
        let scope = format!("project:{}", linked.project.id).parse::<ReconciliationScope>()?;
        let job_id = start_reconciliation_job(&paths, &scope.to_string())?;
        let catalog = crate::managed_resources::fake_runtime_catalog_with_manifest_client(
            OFFLINE_TEST_MANIFEST_URL,
            resource_client,
        )?;
        let events = reconciliation_events(paths, &job_id, scope, &catalog).await?;
        let download_progress = events
            .iter()
            .filter(|event| event["type"] == "download_progress")
            .cloned()
            .collect::<Vec<_>>();

        assert_eq!(
            download_progress.first(),
            Some(&json!({
                "type": "download_progress",
                "job_id": job_id,
                "resource": "mailpit",
                "track": MAILPIT_TEST_TRACK,
                "artifact_version": MAILPIT_TEST_ARTIFACT_VERSION,
                "downloaded_bytes": 0,
                "total_bytes": total_bytes,
            }))
        );
        let live_phases = live_phase_names(&events);
        assert_eq!(
            live_phases,
            [
                "project_apply",
                "resources",
                "manifest",
                "download",
                "install",
                "project_apply",
                "workers",
                "gateway",
                "finalization",
            ]
        );

        Ok(())
    }

    #[tokio::test]
    async fn foreground_update_streams_progress_for_follow_up_reconciliation_downloads()
    -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let mailpit_archive_path = tempdir.path().join("mailpit-1.0.1-pv1-any.tar.gz");
        let composer_archive_path = tempdir.path().join("composer-2.8.1-pv1-any.tar.gz");

        seed_artifact_archive(
            tempdir.path(),
            &mailpit_archive_path,
            "mailpit",
            "1.0.1-pv1",
            "bin/pv-fake-mailpit",
        )?;
        seed_artifact_archive(
            tempdir.path(),
            &composer_archive_path,
            "composer",
            "2.8.1-pv1",
            "composer.phar",
        )?;

        let mailpit_archive = read_file(&mailpit_archive_path)?;
        let composer_archive = read_file(&composer_archive_path)?;
        let mailpit_size = mailpit_archive.len() as u64;
        let composer_size = composer_archive.len() as u64;
        let mailpit_url = "https://artifacts.example.test/mailpit-1.0.1-pv1-any.tar.gz";
        let composer_url = "https://artifacts.example.test/composer-2.8.1-pv1-any.tar.gz";
        let manifest = serde_json::to_string(&json!({
            "schema_version": 1,
            "minimum_pv_version": "0.1.0",
            "resources": [
                {
                    "name": "mailpit",
                    "default_track": "1.0",
                    "tracks": [{
                        "name": "1.0",
                        "artifacts": [manifest_artifact(
                            "1.0.1-pv1",
                            "1.0.1",
                            mailpit_url,
                            &sha256_file(&mailpit_archive_path)?,
                            mailpit_size,
                        )],
                    }],
                },
                {
                    "name": "composer",
                    "default_track": "2",
                    "tracks": [{
                        "name": "2",
                        "artifacts": [manifest_artifact(
                            "2.8.1-pv1",
                            "2.8.1",
                            composer_url,
                            &sha256_file(&composer_archive_path)?,
                            composer_size,
                        )],
                    }],
                },
            ],
        }))?;
        let manifest_requests = Arc::new(AtomicUsize::new(0));
        let client = SequencedMultiArtifactClient {
            manifests: Mutex::new(VecDeque::from([
                manifest,
                serde_json::to_string(&json!({
                    "schema_version": 1,
                    "minimum_pv_version": "0.1.0",
                    "resources": [],
                }))?,
            ])),
            archives: BTreeMap::from([
                (mailpit_url.to_string(), mailpit_archive),
                (composer_url.to_string(), composer_archive),
            ]),
            manifest_requests: Arc::clone(&manifest_requests),
        };
        let installed_mailpit_release = paths.resources().join("mailpit/1.0/releases/1.0.0-pv1");
        let installed_mailpit_executable = installed_mailpit_release.join("bin/pv-fake-mailpit");
        state::fs::write_sensitive_file(&installed_mailpit_executable, "#!/bin/sh\nexit 0\n")?;
        set_executable(&installed_mailpit_executable)?;
        state::fs::symlink_file(
            &Utf8PathBuf::from("releases/1.0.0-pv1"),
            &paths.resources().join("mailpit/1.0/current"),
        )?;
        let mut database = Database::open(&paths)?;
        database.record_managed_resource_track_installed(
            "mailpit",
            "1.0",
            "1.0.0-pv1",
            &installed_mailpit_release,
        )?;
        database.record_managed_resource_track_desired(
            "composer",
            COMPOSER_TEST_TRACK,
            state::ManagedResourceDesiredState::Installed,
        )?;
        drop(database);
        let job_id = start_update_job(&paths)?;
        let catalog = crate::managed_resources::fake_runtime_catalog_with_manifest_client(
            OFFLINE_TEST_MANIFEST_URL,
            client,
        )?;
        let events = update_events(paths, &job_id, &catalog).await?;
        assert!(events.iter().any(|event| {
            event
                == &json!({
                    "type": "download_progress",
                    "job_id": job_id,
                    "resource": "composer",
                    "track": COMPOSER_TEST_TRACK,
                    "artifact_version": "2.8.1-pv1",
                    "downloaded_bytes": 0,
                    "total_bytes": composer_size,
                })
        }));
        assert_eq!(
            events.first(),
            Some(&json!({
                "type": "job_started",
                "job_id": job_id,
                "kind": "update",
                "scope": "system",
            }))
        );
        assert_eq!(
            events.get(1).and_then(|event| event["type"].as_str()),
            Some("log")
        );
        assert_eq!(
            live_phase_names(&events),
            [
                "manifest",
                "download",
                "install",
                "demand_discovery",
                "resources",
                "download",
                "install",
                "resources",
                "project_apply",
                "workers",
                "gateway",
                "finalization",
            ],
            "{events:#?}"
        );
        assert_eq!(
            events.last().and_then(|event| event["type"].as_str()),
            Some("job_failed")
        );
        assert_eq!(manifest_requests.load(Ordering::SeqCst), 1);

        Ok(())
    }

    #[tokio::test]
    async fn streamed_job_writes_heartbeat_before_quiet_completion_finishes() -> anyhow::Result<()>
    {
        let (client, server) = duplex(1024);
        let mut writer = protocol::transport(server);
        let (finish_sender, finish_receiver) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            complete_streamed_job_with_heartbeat(
                &mut writer,
                "job_1",
                "job still running",
                Duration::from_millis(5),
                async {
                    finish_receiver.await.map_err(|_error| {
                        crate::DaemonError::Io(io::Error::other("completion cancelled"))
                    })?;

                    Ok("job done".to_string())
                },
            )
            .await
        });

        let mut reader = protocol::transport(client);
        let line = timeout(Duration::from_millis(100), reader.next())
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing heartbeat line"))??;
        let event = serde_json::from_str::<serde_json::Value>(&line)?;

        assert_eq!(
            event,
            json!({
                "type": "log",
                "job_id": "job_1",
                "message": "job still running",
            })
        );

        finish_sender
            .send(())
            .map_err(|_error| anyhow::anyhow!("completion task dropped"))?;
        assert_eq!(task.await??, "job done");

        Ok(())
    }

    #[tokio::test]
    async fn streamed_job_writes_download_progress_events() -> anyhow::Result<()> {
        let (client, server) = duplex(1024);
        let mut writer = protocol::transport(server);
        let (event_sender, event_receiver) = channel(FOREGROUND_JOB_PROGRESS_BUFFER);
        let (_phase_sender, phase_receiver) = watch::channel(Vec::new());
        let (finish_sender, finish_receiver) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            complete_streamed_job_with_heartbeat_and_events(
                &mut writer,
                "job_1",
                "job still running",
                Duration::from_secs(60),
                async {
                    finish_receiver.await.map_err(|_error| {
                        crate::DaemonError::Io(io::Error::other("completion cancelled"))
                    })?;

                    Ok("job done".to_string())
                },
                event_receiver,
                phase_receiver,
            )
            .await
        });

        event_sender
            .send(ForegroundJobEvent::DownloadProgress {
                resource: "redis".to_string(),
                track: "8.8".to_string(),
                artifact_version: "8.8.1-pv1".to_string(),
                downloaded_bytes: 42,
                total_bytes: 100,
            })
            .await
            .map_err(|_error| anyhow::anyhow!("progress receiver dropped"))?;

        let mut reader = protocol::transport(client);
        let line = timeout(Duration::from_millis(100), reader.next())
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing download progress line"))??;
        let event = serde_json::from_str::<serde_json::Value>(&line)?;

        assert_eq!(
            event,
            json!({
                "type": "download_progress",
                "job_id": "job_1",
                "resource": "redis",
                "track": "8.8",
                "artifact_version": "8.8.1-pv1",
                "downloaded_bytes": 42,
                "total_bytes": 100,
            })
        );

        finish_sender
            .send(())
            .map_err(|_error| anyhow::anyhow!("completion task dropped"))?;
        assert_eq!(task.await?.result?, "job done");

        Ok(())
    }

    #[tokio::test]
    async fn streamed_job_writes_ordered_phases_and_heartbeat() -> anyhow::Result<()> {
        let (client, server) = duplex(1024);
        let mut writer = protocol::transport(server);
        let (_event_sender, event_receiver) = channel(FOREGROUND_JOB_PROGRESS_BUFFER);
        let (phase_sender, phase_receiver) = watch::channel(Vec::new());
        let (finish_sender, finish_receiver) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            complete_streamed_job_with_heartbeat_and_events(
                &mut writer,
                "job_1",
                "job still running",
                Duration::from_millis(20),
                async {
                    finish_receiver.await.map_err(|_error| {
                        crate::DaemonError::Io(io::Error::other("completion cancelled"))
                    })?;

                    Ok("job done".to_string())
                },
                event_receiver,
                phase_receiver,
            )
            .await
        });
        let mut reader = protocol::transport(client);

        phase_sender.send_modify(|phases| {
            phases.push(crate::structured_log::ReconciliationPhase::DemandDiscovery);
        });
        let discovery = timeout(Duration::from_millis(100), reader.next())
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing demand discovery phase"))??;
        phase_sender.send_modify(|phases| {
            phases.push(crate::structured_log::ReconciliationPhase::Resources);
        });
        let resources = timeout(Duration::from_millis(100), reader.next())
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing resources phase"))??;
        let heartbeat = timeout(Duration::from_millis(100), reader.next())
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing heartbeat"))??;

        assert_eq!(
            [discovery, resources, heartbeat]
                .map(|line| serde_json::from_str::<serde_json::Value>(&line))
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?,
            vec![
                json!({
                    "type": "progress",
                    "job_id": "job_1",
                    "message": "demand_discovery",
                }),
                json!({
                    "type": "progress",
                    "job_id": "job_1",
                    "message": "resources",
                }),
                json!({
                    "type": "log",
                    "job_id": "job_1",
                    "message": "job still running",
                }),
            ]
        );

        finish_sender
            .send(())
            .map_err(|_error| anyhow::anyhow!("completion task dropped"))?;
        assert_eq!(task.await?.result?, "job done");

        Ok(())
    }

    #[tokio::test]
    async fn streamed_job_flushes_batched_phases_before_completion() -> anyhow::Result<()> {
        let (client, server) = duplex(1024);
        let mut writer = protocol::transport(server);
        let (_event_sender, event_receiver) = channel(FOREGROUND_JOB_PROGRESS_BUFFER);
        let (phase_sender, phase_receiver) = watch::channel(Vec::new());
        phase_sender.send_modify(|phases| {
            phases.extend([
                crate::structured_log::ReconciliationPhase::DemandDiscovery,
                crate::structured_log::ReconciliationPhase::Resources,
                crate::structured_log::ReconciliationPhase::Finalization,
            ]);
        });

        let completion = complete_streamed_job_with_heartbeat_and_events(
            &mut writer,
            "job_1",
            "job still running",
            Duration::from_secs(60),
            async { Ok("job done".to_string()) },
            event_receiver,
            phase_receiver,
        )
        .await;
        let mut reader = protocol::transport(client);
        let mut events = Vec::new();
        for _phase in 0..3 {
            let line = timeout(Duration::from_millis(100), reader.next())
                .await?
                .ok_or_else(|| anyhow::anyhow!("missing batched phase"))??;
            events.push(serde_json::from_str::<serde_json::Value>(&line)?);
        }

        assert_eq!(completion.result?, "job done");
        assert!(completion.transport_is_open);
        assert_eq!(
            events,
            vec![
                json!({
                    "type": "progress",
                    "job_id": "job_1",
                    "message": "demand_discovery",
                }),
                json!({
                    "type": "progress",
                    "job_id": "job_1",
                    "message": "resources",
                }),
                json!({
                    "type": "progress",
                    "job_id": "job_1",
                    "message": "finalization",
                }),
            ]
        );

        Ok(())
    }

    #[tokio::test]
    async fn download_progress_flood_does_not_hide_phase() -> anyhow::Result<()> {
        let (client, server) = duplex(8192);
        let mut writer = protocol::transport(server);
        let (event_sender, event_receiver) = channel(FOREGROUND_JOB_PROGRESS_BUFFER);
        let total_bytes = u64::try_from(FOREGROUND_JOB_PROGRESS_BUFFER)?;
        for downloaded_bytes in 0..FOREGROUND_JOB_PROGRESS_BUFFER {
            event_sender.try_send(ForegroundJobEvent::DownloadProgress {
                resource: "redis".to_string(),
                track: "8.8".to_string(),
                artifact_version: "8.8.1-pv1".to_string(),
                downloaded_bytes: u64::try_from(downloaded_bytes)?,
                total_bytes,
            })?;
        }
        let (phase_sender, phase_receiver) = watch::channel(Vec::new());
        phase_sender.send_modify(|phases| {
            phases.push(crate::structured_log::ReconciliationPhase::Install);
        });
        let (finish_sender, finish_receiver) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            complete_streamed_job_with_heartbeat_and_events(
                &mut writer,
                "job_1",
                "job still running",
                Duration::from_secs(60),
                async {
                    finish_receiver.await.map_err(|_error| {
                        crate::DaemonError::Io(io::Error::other("completion cancelled"))
                    })?;

                    Ok("job done".to_string())
                },
                event_receiver,
                phase_receiver,
            )
            .await
        });
        let mut reader = protocol::transport(client);
        let mut phase = None;

        for _event in 0..=FOREGROUND_JOB_PROGRESS_BUFFER {
            let line = timeout(Duration::from_millis(100), reader.next())
                .await?
                .ok_or_else(|| anyhow::anyhow!("job stream ended before phase"))??;
            let event = serde_json::from_str::<serde_json::Value>(&line)?;
            if event["type"] == "progress" {
                phase = Some(event);
                break;
            }
        }

        assert_eq!(
            phase,
            Some(json!({
                "type": "progress",
                "job_id": "job_1",
                "message": "install",
            }))
        );
        finish_sender
            .send(())
            .map_err(|_error| anyhow::anyhow!("completion task dropped"))?;
        assert_eq!(task.await?.result?, "job done");

        Ok(())
    }

    #[tokio::test]
    async fn streamed_job_completion_wins_when_heartbeat_write_blocks() -> anyhow::Result<()> {
        let (blocked_write_sender, blocked_write_receiver) = oneshot::channel();
        let mut writer = protocol::transport(InitiallyWritableStream::with_blocked_write_signal(
            0,
            blocked_write_sender,
        ));
        let (finish_sender, finish_receiver) = oneshot::channel::<()>();
        let mut task = tokio::spawn(async move {
            complete_streamed_job_with_heartbeat(
                &mut writer,
                "job_1",
                "job still running",
                Duration::from_millis(5),
                async {
                    finish_receiver.await.map_err(|_error| {
                        crate::DaemonError::Io(io::Error::other("completion cancelled"))
                    })?;

                    Ok("job done".to_string())
                },
            )
            .await
        });

        timeout(Duration::from_millis(100), blocked_write_receiver)
            .await?
            .map_err(|_error| anyhow::anyhow!("heartbeat writer dropped"))?;
        finish_sender
            .send(())
            .map_err(|_error| anyhow::anyhow!("completion task dropped"))?;
        let outcome = timeout(Duration::from_millis(300), &mut task).await;
        if outcome.is_err() {
            task.abort();
        }
        assert_eq!(outcome???, "job done");

        Ok(())
    }

    #[tokio::test]
    async fn streamed_job_completion_wins_when_download_progress_write_blocks() -> anyhow::Result<()>
    {
        let (blocked_write_sender, blocked_write_receiver) = oneshot::channel();
        let mut writer = protocol::transport(InitiallyWritableStream::with_blocked_write_signal(
            0,
            blocked_write_sender,
        ));
        let (event_sender, event_receiver) = channel(FOREGROUND_JOB_PROGRESS_BUFFER);
        let (phase_sender, phase_receiver) = watch::channel(Vec::new());
        let (finish_sender, finish_receiver) = oneshot::channel::<()>();
        let mut task = tokio::spawn(async move {
            complete_streamed_job_with_heartbeat_and_events(
                &mut writer,
                "job_1",
                "job still running",
                Duration::from_secs(60),
                async {
                    finish_receiver.await.map_err(|_error| {
                        crate::DaemonError::Io(io::Error::other("completion cancelled"))
                    })?;

                    Ok("job done".to_string())
                },
                event_receiver,
                phase_receiver,
            )
            .await
        });

        event_sender
            .send(ForegroundJobEvent::DownloadProgress {
                resource: "redis".to_string(),
                track: "8.8".to_string(),
                artifact_version: "8.8.1-pv1".to_string(),
                downloaded_bytes: 42,
                total_bytes: 100,
            })
            .await
            .map_err(|_error| anyhow::anyhow!("progress receiver dropped"))?;
        timeout(Duration::from_millis(100), blocked_write_receiver)
            .await?
            .map_err(|_error| anyhow::anyhow!("progress writer dropped"))?;
        phase_sender.send_modify(|phases| {
            phases.push(crate::structured_log::ReconciliationPhase::Install);
        });
        finish_sender
            .send(())
            .map_err(|_error| anyhow::anyhow!("completion task dropped"))?;
        let outcome = timeout(Duration::from_millis(300), &mut task).await;
        if outcome.is_err() {
            task.abort();
        }
        let completion = outcome??;

        assert_eq!(completion.result?, "job done");
        assert!(completion.transport_is_open);

        Ok(())
    }

    #[tokio::test]
    async fn slow_phase_subscriber_does_not_cancel_completion() -> anyhow::Result<()> {
        let mut writer = protocol::transport(PendingStream);
        let (_event_sender, event_receiver) = channel(FOREGROUND_JOB_PROGRESS_BUFFER);
        let (phase_sender, phase_receiver) = watch::channel(Vec::new());
        let (finish_sender, finish_receiver) = oneshot::channel::<()>();
        let mut task = tokio::spawn(async move {
            complete_streamed_job_with_heartbeat_and_events(
                &mut writer,
                "job_1",
                "job still running",
                Duration::from_secs(60),
                async {
                    finish_receiver.await.map_err(|_error| {
                        crate::DaemonError::Io(io::Error::other("completion cancelled"))
                    })?;

                    Ok("job done".to_string())
                },
                event_receiver,
                phase_receiver,
            )
            .await
        });

        phase_sender.send_modify(|phases| {
            phases.push(crate::structured_log::ReconciliationPhase::Resources);
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        finish_sender
            .send(())
            .map_err(|_error| anyhow::anyhow!("completion task dropped"))?;
        let outcome = timeout(Duration::from_millis(300), &mut task).await;
        if outcome.is_err() {
            task.abort();
        }
        let completion = outcome??;

        assert_eq!(completion.result?, "job done");
        assert!(completion.transport_is_open);

        Ok(())
    }

    #[tokio::test]
    async fn stalled_phase_progress_does_not_close_job_stream() -> anyhow::Result<()> {
        let (client, server) = duplex(1024);
        let writable = Arc::new(AtomicBool::new(false));
        let (stalled_sender, stalled_receiver) = oneshot::channel();
        let writer = protocol::transport(GatedWriteStream::new(
            server,
            Arc::clone(&writable),
            stalled_sender,
        ));
        let (_event_sender, event_receiver) = channel(FOREGROUND_JOB_PROGRESS_BUFFER);
        let (phase_sender, phase_receiver) = watch::channel(Vec::new());
        let (finish_sender, finish_receiver) = oneshot::channel::<()>();
        let mut task = tokio::spawn(async move {
            let mut writer = writer;
            let completion = complete_streamed_job_with_heartbeat_and_events(
                &mut writer,
                "job_1",
                "job still running",
                Duration::from_secs(60),
                async {
                    finish_receiver.await.map_err(|_error| {
                        crate::DaemonError::Io(io::Error::other("completion cancelled"))
                    })?;

                    Ok("job done".to_string())
                },
                event_receiver,
                phase_receiver,
            )
            .await;
            if completion.transport_is_open {
                let _terminal_result = write_foreground_terminal_event(
                    &mut writer,
                    &protocol::DaemonEvent::JobCompleted {
                        job_id: "job_1",
                        summary: "job done",
                    },
                )
                .await;
            }

            completion
        });

        phase_sender.send_modify(|phases| {
            phases.push(crate::structured_log::ReconciliationPhase::Install);
        });
        timeout(Duration::from_millis(100), stalled_receiver)
            .await
            .map_err(|_error| anyhow::anyhow!("phase write did not stall"))?
            .map_err(|_error| anyhow::anyhow!("phase write stream dropped"))?;
        tokio::time::sleep(FOREGROUND_JOB_STREAM_WRITE_TIMEOUT + Duration::from_millis(50)).await;
        writable.store(true, Ordering::SeqCst);
        finish_sender
            .send(())
            .map_err(|_error| anyhow::anyhow!("completion task dropped"))?;
        let completion = timeout(Duration::from_millis(300), &mut task).await??;

        assert_eq!(completion.result?, "job done");
        assert!(completion.transport_is_open);

        let mut reader = protocol::transport(client);
        let line = timeout(Duration::from_millis(100), reader.next())
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing terminal event"))??;
        let event = serde_json::from_str::<serde_json::Value>(&line)?;
        assert_eq!(event["type"], "job_completed");
        assert!(reader.next().await.is_none());

        Ok(())
    }

    #[tokio::test]
    async fn streamed_job_ignores_download_progress_write_errors() -> anyhow::Result<()> {
        let mut writer = protocol::transport(FailingWriteStream::default());
        let (event_sender, event_receiver) = channel(FOREGROUND_JOB_PROGRESS_BUFFER);
        let (_phase_sender, phase_receiver) = watch::channel(Vec::new());
        let (finish_sender, finish_receiver) = oneshot::channel::<()>();
        let mut task = tokio::spawn(async move {
            complete_streamed_job_with_heartbeat_and_events(
                &mut writer,
                "job_1",
                "job still running",
                Duration::from_secs(60),
                async {
                    finish_receiver.await.map_err(|_error| {
                        crate::DaemonError::Io(io::Error::other("completion cancelled"))
                    })?;

                    Ok("job done".to_string())
                },
                event_receiver,
                phase_receiver,
            )
            .await
        });

        event_sender
            .send(ForegroundJobEvent::DownloadProgress {
                resource: "redis".to_string(),
                track: "8.8".to_string(),
                artifact_version: "8.8.1-pv1".to_string(),
                downloaded_bytes: 42,
                total_bytes: 100,
            })
            .await
            .map_err(|_error| anyhow::anyhow!("progress receiver dropped"))?;
        tokio::time::sleep(Duration::from_millis(20)).await;
        finish_sender
            .send(())
            .map_err(|_error| anyhow::anyhow!("completion task dropped"))?;
        let outcome = timeout(Duration::from_millis(300), &mut task).await;
        if outcome.is_err() {
            task.abort();
        }
        assert_eq!((outcome??).result?, "job done");

        Ok(())
    }

    struct PendingStream;

    impl AsyncRead for PendingStream {
        fn poll_read(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            _buffer: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Poll::Pending
        }
    }

    impl AsyncWrite for PendingStream {
        fn poll_write(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            _buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            Poll::Pending
        }

        fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    struct InitiallyWritableStream {
        remaining_writes: usize,
        blocked_write_sender: Option<oneshot::Sender<Instant>>,
    }

    impl InitiallyWritableStream {
        fn with_blocked_write_signal(
            remaining_writes: usize,
            blocked_write_sender: oneshot::Sender<Instant>,
        ) -> Self {
            Self {
                remaining_writes,
                blocked_write_sender: Some(blocked_write_sender),
            }
        }
    }

    impl AsyncRead for InitiallyWritableStream {
        fn poll_read(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            _buffer: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Poll::Pending
        }
    }

    impl AsyncWrite for InitiallyWritableStream {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            if self.remaining_writes == 0 {
                if let Some(sender) = self.blocked_write_sender.take() {
                    let _send_result = sender.send(Instant::now());
                }

                return Poll::Pending;
            }

            self.remaining_writes -= 1;
            Poll::Ready(Ok(buffer.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    struct GatedWriteStream {
        inner: DuplexStream,
        writable: Arc<AtomicBool>,
        blocked_write_sender: Option<oneshot::Sender<()>>,
    }

    impl GatedWriteStream {
        fn new(
            inner: DuplexStream,
            writable: Arc<AtomicBool>,
            blocked_write_sender: oneshot::Sender<()>,
        ) -> Self {
            Self {
                inner,
                writable,
                blocked_write_sender: Some(blocked_write_sender),
            }
        }
    }

    impl AsyncRead for GatedWriteStream {
        fn poll_read(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            _buffer: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Poll::Pending
        }
    }

    impl AsyncWrite for GatedWriteStream {
        fn poll_write(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
            buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            if !self.writable.load(Ordering::SeqCst) {
                if let Some(sender) = self.blocked_write_sender.take() {
                    let _send_result = sender.send(());
                }

                // The stalled write is dropped instead of being buffered for a later flush.
                return Poll::Ready(Ok(buffer.len()));
            }

            Pin::new(&mut self.inner).poll_write(context, buffer)
        }

        fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
            if !self.writable.load(Ordering::SeqCst) {
                return Poll::Pending;
            }

            Pin::new(&mut self.inner).poll_flush(context)
        }

        fn poll_shutdown(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[derive(Default)]
    struct FailingWriteStream {
        failed_write_sender: Option<oneshot::Sender<()>>,
    }

    impl FailingWriteStream {
        fn with_signal(failed_write_sender: oneshot::Sender<()>) -> Self {
            Self {
                failed_write_sender: Some(failed_write_sender),
            }
        }
    }

    impl AsyncRead for FailingWriteStream {
        fn poll_read(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            _buffer: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Poll::Pending
        }
    }

    impl AsyncWrite for FailingWriteStream {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            _buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            if let Some(sender) = self.failed_write_sender.take() {
                let _send_result = sender.send(());
            }
            Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "stream closed",
            )))
        }

        fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn coalesced_update_response_is_error_without_job_id() -> anyhow::Result<()> {
        let (client, server) = duplex(1024);
        let mut writer = protocol::transport(server);

        write_coalesced_update_response(&mut writer).await?;
        drop(writer);

        let mut reader = protocol::transport(client);
        let line = reader
            .next()
            .await
            .ok_or_else(|| anyhow::anyhow!("missing response line"))??;
        let response = serde_json::from_str::<serde_json::Value>(&line)?;

        assert_eq!(
            response,
            json!({
                "type": "response",
                "protocol_version": protocol::PROTOCOL_VERSION,
                "status": "error",
                "message": "update already queued or running",
            })
        );
        assert!(reader.next().await.is_none());

        Ok(())
    }

    #[test]
    fn background_reconciliation_failure_marks_started_job_failed() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let job_id = start_reconciliation_job(&paths, "system")?;

        let result = complete_or_fail_background_reconciliation(&paths, &job_id, || {
            Err(crate::DaemonError::Io(io::Error::other("reconcile failed")))
        });

        assert!(result.is_err());
        let database = Database::open(&paths)?;
        let job = database
            .recent_jobs()?
            .into_iter()
            .find(|job| job.id == job_id)
            .ok_or_else(|| anyhow::anyhow!("missing job {job_id}"))?;
        assert_eq!(job.status, JobStatus::Failed);
        assert_eq!(job.error.as_deref(), Some("I/O error: reconcile failed"));

        Ok(())
    }

    #[test]
    fn background_reconciliation_error_records_failed_job() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let error = crate::DaemonError::Io(io::Error::other("background task failed"));

        record_background_reconciliation_error(&paths, "project:project_1", &error)?;

        let database = Database::open(&paths)?;
        let job = database
            .recent_jobs()?
            .into_iter()
            .find(|job| job.scope == "project:project_1")
            .ok_or_else(|| anyhow::anyhow!("missing background failure job"))?;
        assert_eq!(job.status, JobStatus::Failed);
        assert_eq!(
            job.error.as_deref(),
            Some("I/O error: background task failed")
        );

        Ok(())
    }

    #[test]
    fn background_error_deduplication_resets_after_successful_coverage() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let error = crate::DaemonError::Io(io::Error::other("background task failed"));

        record_background_reconciliation_error(&paths, "project:project_1", &error)?;
        record_background_reconciliation_error(&paths, "project:project_1", &error)?;
        let mut database = Database::open(&paths)?;
        assert_eq!(database.recent_jobs()?.len(), 1);
        let success = database.start_job("reconcile", "project:project_1")?;
        database.complete_job_with_coverage(
            &success.id,
            "Project reconciled",
            &[state::JobDiagnosticSubject::Project {
                id: "project_1".to_owned(),
            }],
        )?;
        drop(database);

        record_background_reconciliation_error(&paths, "project:project_1", &error)?;

        let database = Database::open(&paths)?;
        let failed = database
            .recent_jobs()?
            .into_iter()
            .filter(|job| job.status == JobStatus::Failed)
            .collect::<Vec<_>>();
        assert_eq!(failed.len(), 2);
        assert_eq!(database.unresolved_job_failures()?.len(), 1);

        Ok(())
    }

    #[test]
    fn background_reconciliation_error_writes_structured_daemon_log() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let error = crate::DaemonError::Io(io::Error::other("background task failed"));

        record_background_reconciliation_error(&paths, "project:project_1", &error)?;

        let content = state::fs::read_to_string(&paths.daemon_log())?;
        let events = content
            .lines()
            .map(serde_json::from_str::<serde_json::Value>)
            .collect::<Result<Vec<_>, _>>()?;

        assert!(events.iter().any(|event| {
            event["event"] == "job_started"
                && event["kind"] == "reconcile"
                && event["scope"] == "project:project_1"
        }));
        assert!(events.iter().any(|event| {
            event["event"] == "job_failed"
                && event["kind"] == "reconcile"
                && event["scope"] == "project:project_1"
                && event["error"] == "I/O error: background task failed"
        }));

        Ok(())
    }

    #[test]
    fn background_reconciliation_error_persists_when_structured_log_fails() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let error = crate::DaemonError::Io(io::Error::other("background task failed"));
        Database::open(&paths)?;
        create_directory(&paths.daemon_log())?;

        record_background_reconciliation_error(&paths, "project:project_1", &error)?;

        let database = Database::open(&paths)?;
        let job = database
            .recent_jobs()?
            .into_iter()
            .find(|job| job.scope == "project:project_1")
            .ok_or_else(|| anyhow::anyhow!("missing background failure job"))?;
        assert_eq!(job.status, JobStatus::Failed);
        assert_eq!(
            job.error.as_deref(),
            Some("I/O error: background task failed")
        );

        Ok(())
    }

    #[test]
    fn abandonment_failure_writes_structured_daemon_log() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        Database::open(&paths)?;

        let result = abandon_reconciliation_job(&paths, "job_missing");

        assert!(matches!(
            result,
            Err(DaemonError::State(StateError::JobNotFound { id })) if id == "job_missing"
        ));
        let content = state::fs::read_to_string(&paths.daemon_log())?;
        let events = content
            .lines()
            .map(serde_json::from_str::<serde_json::Value>)
            .collect::<Result<Vec<_>, _>>()?;
        assert!(events.iter().any(|event| {
            event["event"] == "job_abandonment_failed"
                && event["job_id"] == "job_missing"
                && event["kind"] == "reconcile"
        }));

        Ok(())
    }

    #[tokio::test]
    async fn background_reconciliation_rejects_jobs_lock_without_job() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let jobs_lock = JobsLock::acquire(&paths)?;
        let result = run_background_reconciliation_job(
            paths.clone(),
            ReconciliationQueue::new(),
            ReconciliationScope::System,
            None,
        )
        .await;

        assert!(matches!(
            result,
            Err(crate::DaemonError::State(StateError::CoordinationLockHeld { path }))
                if path == paths.jobs_lock()
        ));
        drop(jobs_lock);

        let database = Database::open(&paths)?;
        assert!(database.recent_jobs()?.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn startup_shutdown_preserves_enqueue_failure() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        state::fs::write_sensitive_file(paths.db(), "not a database")?;
        let (shutdown_sender, shutdown_receiver) = oneshot::channel();
        let (_fallback_shutdown_sender, fallback_shutdown_receiver) = watch::channel(false);
        shutdown_sender
            .send(())
            .map_err(|()| anyhow::anyhow!("startup shutdown receiver was dropped"))?;

        let result = run_startup_reconciliation_job(
            paths,
            ReconciliationQueue::new(),
            None,
            shutdown_receiver,
            fallback_shutdown_receiver,
        )
        .await;

        assert!(matches!(
            result,
            Err(super::BackgroundReconciliationError::Admission(error))
                if matches!(*error, DaemonError::State(StateError::Sqlite(_)))
        ));

        Ok(())
    }

    #[tokio::test]
    async fn background_reconciliation_discards_removed_project_scopes() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        Database::open(&paths)?;

        run_background_reconciliation_job(
            paths.clone(),
            ReconciliationQueue::new(),
            ReconciliationScope::project("removed-before-admission")?,
            None,
        )
        .await?;
        assert!(Database::open(&paths)?.recent_jobs()?.is_empty());

        let project_id = link_background_test_project(
            &paths,
            &tempdir.path().join("queued-project"),
            "queued-project.test",
        )?;

        let queue = ReconciliationQueue::new();
        let blocker = queued(enqueue_reconciliation_job(
            &paths,
            &queue,
            ReconciliationScope::System,
        )?)?;
        let running = blocker.wait_for_turn().await;
        let scope = ReconciliationScope::project(project_id.clone())?;
        let scope_text = scope.to_string();
        let queued_paths = paths.clone();
        let queued_queue = queue.clone();
        let queued_task = tokio::spawn(async move {
            run_background_reconciliation_job(queued_paths, queued_queue, scope, None).await
        });
        wait_for_job_scope(&paths, &scope_text).await?;
        Database::open(&paths)?.unlink_project(&project_id)?;
        let foreground_result = enqueue_foreground_reconciliation_job(
            &paths,
            &queue,
            ReconciliationScope::project(project_id.clone())?,
        );
        assert!(matches!(
            foreground_result,
            Err(DaemonError::State(StateError::ProjectNotFound { target }))
                if target == project_id
        ));
        running.finish();

        queued_task.await??;
        let database = Database::open(&paths)?;
        let job = database
            .recent_jobs()?
            .into_iter()
            .find(|job| job.scope == scope_text)
            .ok_or_else(|| anyhow::anyhow!("missing removed Project job"))?;
        assert_eq!(job.status, JobStatus::Succeeded);
        assert_eq!(
            job.summary.as_deref(),
            Some("Project was removed before background reconciliation; skipped")
        );
        let coverage_count = Connection::open(paths.db().as_std_path())?.query_row(
            "SELECT COUNT(*) FROM job_diagnostic_outcomes WHERE job_id = ?1",
            [&job.id],
            |row| row.get::<_, i64>(0),
        )?;
        assert_eq!(coverage_count, 0);

        Ok(())
    }

    #[test]
    fn foreground_reconciliation_still_rejects_missing_project() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        Database::open(&paths)?;

        let result = enqueue_foreground_reconciliation_job(
            &paths,
            &ReconciliationQueue::new(),
            ReconciliationScope::project("missing")?,
        );

        assert!(matches!(
            result,
            Err(DaemonError::State(StateError::ProjectNotFound { target })) if target == "missing"
        ));
        assert!(Database::open(&paths)?.recent_jobs()?.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn startup_shutdown_wins_ready_queue_handoff() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let queue = ReconciliationQueue::new();
        let blocker = queued(enqueue_reconciliation_job(
            &paths,
            &queue,
            ReconciliationScope::project("blocker")?,
        )?)?;
        let running = blocker.wait_for_turn().await;
        let startup = queued(enqueue_startup_reconciliation_job(&paths, &queue)?)?;
        let startup_job_id = startup.job_id().to_string();
        let (shutdown_sender, mut shutdown_receiver) = oneshot::channel();

        shutdown_sender
            .send(())
            .map_err(|()| anyhow::anyhow!("startup shutdown receiver was dropped"))?;
        running.finish();
        assert!(
            wait_for_startup_reconciliation_turn(startup, &mut shutdown_receiver)
                .await
                .is_none()
        );

        let database = Database::open(&paths)?;
        let startup = database
            .recent_jobs()?
            .into_iter()
            .find(|job| job.id == startup_job_id)
            .ok_or_else(|| anyhow::anyhow!("missing startup reconciliation job"))?;
        assert_eq!(startup.status, JobStatus::Failed);
        assert_eq!(
            startup.error.as_deref(),
            Some("reconciliation was abandoned before completion")
        );

        Ok(())
    }

    #[tokio::test]
    async fn queued_background_reconciliation_reserves_only_jobs_lock() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let queue = ReconciliationQueue::new();
        let first = queued(enqueue_reconciliation_job(
            &paths,
            &queue,
            ReconciliationScope::System,
        )?)?;
        let running = first.wait_for_turn().await;
        let project_id = link_background_test_project(
            &paths,
            &tempdir.path().join("queued-project"),
            "queued-project.test",
        )?;
        let queued_paths = paths.clone();
        let queued_queue = queue.clone();
        let queued_scope = ReconciliationScope::project(project_id.clone())?;
        let queued_task = tokio::spawn(async move {
            run_background_reconciliation_job(queued_paths, queued_queue, queued_scope, None).await
        });

        wait_for_job_scope(&paths, &format!("project:{project_id}")).await?;
        let update_lock = UpdateLock::acquire(&paths)?;
        let jobs_lock = JobsLock::acquire(&paths);

        assert!(matches!(
            jobs_lock,
            Err(StateError::CoordinationLockHeld { path }) if path == paths.jobs_lock()
        ));
        drop(update_lock);

        queued_task.abort();
        let _join_result = queued_task.await;
        running.finish();
        let _jobs_lock = JobsLock::acquire(&paths)?;

        Ok(())
    }

    #[tokio::test]
    async fn queued_foreground_job_streams_heartbeats_until_its_turn() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let queue = ReconciliationQueue::new();
        let first = queued(enqueue_update_job(&paths, &queue)?)?;
        let running = first.wait_for_turn().await;
        let waiting = queued(enqueue_reconciliation_job(
            &paths,
            &queue,
            ReconciliationScope::System,
        )?)?;
        let waiting_job_id = waiting.job_id().to_string();
        let (client, server) = duplex(1024);
        let task = tokio::spawn(async move {
            let mut transport = protocol::transport(server);
            wait_for_foreground_turn(
                waiting,
                &mut transport,
                true,
                Duration::from_millis(5),
                None,
            )
            .await
        });
        let mut reader = protocol::transport(client);

        for _heartbeat in 0..2 {
            let line = timeout(Duration::from_millis(100), reader.next())
                .await?
                .ok_or_else(|| anyhow::anyhow!("missing queued heartbeat"))??;
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&line)?,
                json!({
                    "type": "log",
                    "job_id": waiting_job_id,
                    "message": "Waiting for the reconciliation slot",
                })
            );
        }

        running.finish();
        let (waiting_running, stream_is_open) = task
            .await?
            .ok_or_else(|| anyhow::anyhow!("queued foreground job was cancelled"))?;
        assert!(stream_is_open);
        assert_eq!(waiting_running.job_id(), waiting_job_id);
        waiting_running.finish();

        Ok(())
    }

    #[tokio::test]
    async fn queued_foreground_job_continues_after_heartbeat_write_fails() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let queue = ReconciliationQueue::new();
        let first = queued(enqueue_update_job(&paths, &queue)?)?;
        let running = first.wait_for_turn().await;
        let waiting = queued(enqueue_reconciliation_job(
            &paths,
            &queue,
            ReconciliationScope::System,
        )?)?;
        let waiting_job_id = waiting.job_id().to_string();
        let (failed_write_sender, failed_write_receiver) = oneshot::channel();
        let task = tokio::spawn(async move {
            let mut transport =
                protocol::transport(FailingWriteStream::with_signal(failed_write_sender));
            wait_for_foreground_turn(
                waiting,
                &mut transport,
                true,
                Duration::from_millis(5),
                None,
            )
            .await
        });

        timeout(Duration::from_millis(100), failed_write_receiver)
            .await?
            .map_err(|_error| anyhow::anyhow!("queued heartbeat writer dropped"))?;
        running.finish();
        let (waiting_running, stream_is_open) = task
            .await?
            .ok_or_else(|| anyhow::anyhow!("queued foreground job was cancelled"))?;

        assert!(!stream_is_open);
        assert_eq!(waiting_running.job_id(), waiting_job_id);
        waiting_running.finish();

        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn queued_foreground_reconciliation_streams_only_after_its_turn() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let queue = ReconciliationQueue::new();
        let first = queued(enqueue_update_job(&paths, &queue)?)?;
        let running = first.wait_for_turn().await;
        let (client, server) = UnixStream::pair()?;
        let task_paths = paths.clone();
        let task_queue = queue.clone();
        let scope = ReconciliationScope::resource("caddy", "2")?;
        let (_fallback_sender, fallback_receiver) = watch::channel(false);
        let task = tokio::spawn(async move {
            run_reconciliation_job(
                task_paths,
                task_queue,
                protocol::transport(server),
                scope,
                None,
                &fallback_receiver,
            )
            .await
        });
        let mut reader = protocol::transport(client);
        let accepted = timeout(Duration::from_millis(100), reader.next())
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing accepted response"))??;
        let accepted = serde_json::from_str::<serde_json::Value>(&accepted)?;

        assert_eq!(accepted["type"], "response");
        assert_eq!(accepted["status"], "accepted");
        assert!(
            timeout(Duration::from_millis(20), reader.next())
                .await
                .is_err()
        );

        running.finish();
        let mut events = Vec::new();
        loop {
            let line = timeout(Duration::from_millis(500), reader.next())
                .await?
                .ok_or_else(|| anyhow::anyhow!("job stream ended before completion"))??;
            let event = serde_json::from_str::<serde_json::Value>(&line)?;
            let completed = event["type"] == "job_completed";
            events.push(event);
            if completed {
                break;
            }
        }

        assert_eq!(events[0]["type"], "job_started");
        assert_eq!(events[1]["type"], "log");
        let phases = live_phase_names(&events);
        assert_eq!(
            phases,
            ["workers", "gateway", "finalization"],
            "{events:#?}"
        );
        assert_eq!(
            events.last().and_then(|event| event["type"].as_str()),
            Some("job_completed")
        );
        task.await??;

        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn cancelled_queued_foreground_reconciliation_is_abandoned_before_its_turn()
    -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let (client, server) = UnixStream::pair()?;
        let mut client = protocol::transport(client);
        let queue = ReconciliationQueue::new();
        let active = queued(enqueue_update_job(&paths, &queue)?)?
            .wait_for_turn()
            .await;
        let (fallback_sender, fallback_receiver) = watch::channel(false);
        let task_paths = paths.clone();
        let mut task = tokio::spawn(async move {
            run_reconciliation_job(
                task_paths,
                queue,
                protocol::transport(server),
                ReconciliationScope::resource("caddy", "2")?,
                None,
                &fallback_receiver,
            )
            .await
        });

        let accepted = timeout(Duration::from_millis(300), client.next())
            .await?
            .ok_or_else(|| anyhow::anyhow!("foreground stream ended before queue admission"))??;
        let accepted = serde_json::from_str::<serde_json::Value>(&accepted)?;
        assert_eq!(accepted["status"], "accepted");
        assert!(!task.is_finished());
        fallback_sender
            .send(true)
            .map_err(|_| anyhow::anyhow!("queued foreground reconciliation stopped early"))?;
        let result = timeout(Duration::from_millis(300), &mut task).await??;

        assert!(result.is_ok());
        let job = Database::open(&paths)?
            .recent_jobs()?
            .into_iter()
            .find(|job| job.kind == "reconcile")
            .ok_or_else(|| anyhow::anyhow!("missing queued reconciliation job"))?;
        assert_eq!(job.status, JobStatus::Failed);
        assert_eq!(
            job.error.as_deref(),
            Some("reconciliation was abandoned before completion")
        );
        let coverage_count = Connection::open(paths.db().as_std_path())?.query_row(
            "SELECT COUNT(*) FROM job_diagnostic_outcomes WHERE job_id = ?1 AND outcome = 'success'",
            [&job.id],
            |row| row.get::<_, i64>(0),
        )?;
        assert_eq!(coverage_count, 0);
        active.finish();

        Ok(())
    }

    #[tokio::test]
    async fn cancelled_queued_background_reconciliation_is_abandoned() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let queue = ReconciliationQueue::new();
        let active = queued(enqueue_update_job(&paths, &queue)?)?
            .wait_for_turn()
            .await;
        let waiting = queued(enqueue_reconciliation_job(
            &paths,
            &queue,
            ReconciliationScope::resource("caddy", "2")?,
        )?)?;
        let waiting_job_id = waiting.job_id().to_owned();
        let (fallback_sender, fallback_receiver) = watch::channel(false);
        let completion_paths = paths.clone();
        let completion_task = tokio::spawn(async move {
            complete_queued_background_reconciliation_job(
                &completion_paths,
                waiting,
                None,
                Some(&fallback_receiver),
            )
            .await
        });

        tokio::task::yield_now().await;
        assert!(!completion_task.is_finished());
        fallback_sender
            .send(true)
            .map_err(|_| anyhow::anyhow!("queued reconciliation stopped before cancellation"))?;
        timeout(Duration::from_millis(300), completion_task)
            .await??
            .map_err(BackgroundReconciliationError::into_error)?;

        let job = Database::open(&paths)?
            .recent_jobs()?
            .into_iter()
            .find(|job| job.id == waiting_job_id)
            .ok_or_else(|| anyhow::anyhow!("missing queued reconciliation job"))?;
        assert_eq!(job.status, JobStatus::Failed);
        assert_eq!(
            job.error.as_deref(),
            Some("reconciliation was abandoned before completion")
        );
        active.finish();

        Ok(())
    }

    #[tokio::test]
    async fn fallback_shutdown_finishes_resource_phase_before_abandoning_system_job()
    -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        seed_installed_caddy(&paths)?;
        let _caddy_guard = SeededRuntimeGuard::with_gateway(paths.clone());
        let (client, _archive_size) = scripted_artifact_client(
            tempdir.path(),
            "composer",
            COMPOSER_TEST_TRACK,
            COMPOSER_TEST_ARTIFACT_VERSION,
            COMPOSER_TEST_ARCHIVE_FILE_NAME,
            "composer.phar",
        )?;
        let started = Arc::new(AtomicBool::new(false));
        let (release_sender, release_receiver) = mpsc::channel();
        let client = HeldManifestArtifactClient {
            inner: client,
            release_receiver: Mutex::new(release_receiver),
            started: Some(Arc::clone(&started)),
        };
        Database::open(&paths)?.record_managed_resource_track_desired(
            "composer",
            COMPOSER_TEST_TRACK,
            ManagedResourceDesiredState::Installed,
        )?;
        let catalog = Arc::new(
            crate::managed_resources::ManagedResourceRuntimeCatalog::without_adapters_with_manifest_client(
                OFFLINE_TEST_MANIFEST_URL,
                client,
            )?,
        );
        let queue = ReconciliationQueue::new();
        let queued = queued(enqueue_reconciliation_job(
            &paths,
            &queue,
            ReconciliationScope::System,
        )?)?;
        let job_id = queued.job_id().to_owned();
        let (fallback_sender, fallback_receiver) = watch::channel(false);
        let task_paths = paths.clone();
        let task_catalog = Arc::clone(&catalog);
        let mut completion_task = tokio::spawn(async move {
            complete_queued_background_reconciliation_job(
                &task_paths,
                queued,
                Some(task_catalog.as_ref()),
                Some(&fallback_receiver),
            )
            .await
        });
        timeout(Duration::from_secs(5), async {
            while !started.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await?;

        fallback_sender
            .send(true)
            .map_err(|_| anyhow::anyhow!("system reconciliation stopped before cancellation"))?;
        assert!(
            timeout(Duration::from_millis(100), &mut completion_task)
                .await
                .is_err()
        );
        assert!(matches!(
            JobsLock::acquire(&paths),
            Err(StateError::CoordinationLockHeld { .. })
        ));
        release_sender.send(())?;
        timeout(Duration::from_secs(5), completion_task)
            .await??
            .map_err(BackgroundReconciliationError::into_error)?;

        let job = Database::open(&paths)?
            .recent_jobs()?
            .into_iter()
            .find(|job| job.id == job_id)
            .ok_or_else(|| anyhow::anyhow!("missing system reconciliation job"))?;
        assert_eq!(job.status, JobStatus::Failed);
        assert_eq!(
            job.error.as_deref(),
            Some("reconciliation was abandoned before completion")
        );
        let completed_phases = reconciliation_phase_events(&paths, &job.id)?
            .into_iter()
            .filter_map(|event| event["phase"].as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        assert_eq!(completed_phases, ["queue", "demand_discovery", "resources"]);
        let coverage_count = Connection::open(paths.db().as_std_path())?.query_row(
            "SELECT COUNT(*) FROM job_diagnostic_outcomes WHERE job_id = ?1 AND outcome = 'success'",
            [&job.id],
            |row| row.get::<_, i64>(0),
        )?;
        assert_eq!(coverage_count, 0);
        assert!(!paths.gateway_pid().exists());
        assert!(!paths.gateway_runtime_metadata().exists());
        assert!(!paths.gateway_root_config().exists());

        Ok(())
    }

    #[test]
    fn fallback_after_update_mutation_preserves_changed_and_no_op_boundaries() -> anyhow::Result<()>
    {
        for (artifact_version, archive_file_name, expected_summary) in [
            ("2.8.1-pv1", "composer-2.8.1-pv1-any.tar.gz", None),
            (
                COMPOSER_TEST_ARTIFACT_VERSION,
                COMPOSER_TEST_ARCHIVE_FILE_NAME,
                Some("current"),
            ),
        ] {
            let tempdir = tempdir()?;
            let paths = PvPaths::for_home(tempdir.path().join("home"));
            seed_installed_artifact(
                &paths,
                "composer",
                COMPOSER_TEST_TRACK,
                COMPOSER_TEST_ARTIFACT_VERSION,
                "composer.phar",
            )?;
            let (client, _archive_size) = scripted_artifact_client(
                tempdir.path(),
                "composer",
                COMPOSER_TEST_TRACK,
                artifact_version,
                archive_file_name,
                "composer.phar",
            )?;
            let started = Arc::new(AtomicBool::new(false));
            let (release_sender, release_receiver) = mpsc::channel();
            let client = HeldManifestArtifactClient {
                inner: client,
                release_receiver: Mutex::new(release_receiver),
                started: Some(Arc::clone(&started)),
            };
            let catalog = Arc::new(
                crate::managed_resources::ManagedResourceRuntimeCatalog::without_adapters_with_manifest_client(
                    OFFLINE_TEST_MANIFEST_URL,
                    client,
                )?,
            );
            let queue = ReconciliationQueue::new();
            let runtime = crate::build_runtime()?;
            let waiting = queued(enqueue_update_job(&paths, &queue)?)?;
            let running = runtime.block_on(waiting.wait_for_turn());
            let job_id = running.job_id().to_owned();
            let (fallback_sender, fallback_receiver) = watch::channel(false);
            let completion_paths = paths.clone();
            let completion_catalog = Arc::clone(&catalog);
            let completion_job_id = job_id.clone();
            let completion_thread = std::thread::Builder::new()
                .name("held-update-fallback".to_owned())
                .spawn(move || -> anyhow::Result<_> {
                    let runtime = crate::build_runtime()?;
                    let completion = runtime.block_on(complete_update_job_with_progress(
                        &completion_paths,
                        &completion_job_id,
                        Some(completion_catalog.as_ref()),
                        DaemonDownloadProgress::disabled(),
                        running.timing(),
                        Some(&fallback_receiver),
                    ));
                    if matches!(completion, ReconciliationJobCompletion::Cancelled) {
                        drop(running);
                    } else {
                        running.finish();
                    }

                    Ok(completion)
                })?;
            let deadline = Instant::now() + Duration::from_secs(5);
            while !started.load(Ordering::SeqCst) {
                if Instant::now() >= deadline {
                    return Err(anyhow::anyhow!(
                        "update manifest request did not reach its test gate"
                    ));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(matches!(
                JobsLock::acquire(&paths),
                Err(StateError::CoordinationLockHeld { path }) if path == paths.jobs_lock()
            ));

            fallback_sender
                .send(true)
                .map_err(|_| anyhow::anyhow!("update stopped before fallback was signalled"))?;
            assert!(
                !completion_thread.is_finished(),
                "update returned before its non-interruptible mutation completed"
            );
            release_sender
                .send(())
                .map_err(|_| anyhow::anyhow!("update dropped its manifest gate"))?;
            let completion = completion_thread
                .join()
                .map_err(|_| anyhow::anyhow!("update completion thread panicked"))??;
            let job = Database::open(&paths)?
                .recent_jobs()?
                .into_iter()
                .find(|job| job.id == job_id)
                .ok_or_else(|| anyhow::anyhow!("missing update job {job_id}"))?;

            match (expected_summary, completion) {
                (Some(expected_summary), ReconciliationJobCompletion::Succeeded(summary)) => {
                    assert_eq!(summary, expected_summary);
                    assert_eq!(job.status, JobStatus::Succeeded);
                }
                (None, ReconciliationJobCompletion::Cancelled) => {
                    assert_eq!(job.status, JobStatus::Failed);
                    assert_eq!(
                        job.error.as_deref(),
                        Some("Managed Resource update was abandoned before completion")
                    );
                    let current_path = state::fs::read_link(
                        &paths
                            .resources()
                            .join("composer")
                            .join(COMPOSER_TEST_TRACK)
                            .join("current"),
                    )?;
                    assert_eq!(
                        current_path,
                        Utf8PathBuf::from(format!("releases/{artifact_version}"))
                    );
                    let phases = reconciliation_phase_events(&paths, &job_id)?;
                    for phase in [
                        "demand_discovery",
                        "resources",
                        "project_apply",
                        "workers",
                        "gateway",
                    ] {
                        assert!(
                            !phases.iter().any(|event| event["phase"] == phase),
                            "update entered {phase} after fallback: {phases:#?}"
                        );
                    }
                }
                (expected_summary, _completion) => {
                    return Err(anyhow::anyhow!(
                        "unexpected update completion for expected summary {expected_summary:?}"
                    ));
                }
            }
            let _jobs_lock = JobsLock::acquire(&paths)?;
        }

        Ok(())
    }

    #[test]
    fn system_project_failures_outrank_gateway_cancellation() -> anyhow::Result<()> {
        let sentinel = DaemonError::UnexpectedProtocolResponse {
            reason: "project sentinel".to_owned(),
        };
        let result = cancel_or_preserve_system_reconciliation_errors(
            Ok(()),
            Ok(SystemProjectReconciliationReport {
                failures: vec![sentinel],
                ..SystemProjectReconciliationReport::default()
            }),
            Ok(()),
        );
        let error = result
            .err()
            .ok_or_else(|| anyhow::anyhow!("cancellation replaced the Project failure"))?;
        assert_eq!(error.to_string(), "daemon protocol error: project sentinel");

        Ok(())
    }

    #[tokio::test]
    async fn background_reconciliation_coalesces_under_daemon_jobs_lock() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let queue = ReconciliationQueue::new();
        let first = queued(enqueue_reconciliation_job(
            &paths,
            &queue,
            ReconciliationScope::System,
        )?)?;
        let running = first.wait_for_turn().await;
        let project_id = link_background_test_project(
            &paths,
            &tempdir.path().join("coalesced-project"),
            "coalesced-project.test",
        )?;
        let scope = ReconciliationScope::project(project_id.clone())?;
        let queued_paths = paths.clone();
        let queued_queue = queue.clone();
        let queued_scope = scope.clone();
        let queued_task = tokio::spawn(async move {
            run_background_reconciliation_job(queued_paths, queued_queue, queued_scope, None).await
        });

        wait_for_job_scope(&paths, &format!("project:{project_id}")).await?;
        run_background_reconciliation_job(paths.clone(), queue.clone(), scope, None).await?;

        queued_task.abort();
        let _join_result = queued_task.await;
        running.finish();

        Ok(())
    }

    #[test]
    fn foreground_reconciliation_result_takes_precedence_over_accepted_write_error() {
        let result = foreground_reconciliation_result(
            Err(crate::DaemonError::Io(io::Error::other(
                "accepted write failed",
            ))),
            Err(crate::DaemonError::State(StateError::JobNotFound {
                id: "reconcile_1".to_string(),
            })),
        );

        assert!(matches!(
            result,
            Err(crate::DaemonError::State(StateError::JobNotFound { id }))
                if id == "reconcile_1"
        ));
    }

    #[test]
    fn foreground_reconciliation_returns_accepted_write_error_after_successful_reconciliation() {
        let result = foreground_reconciliation_result(
            Err(crate::DaemonError::Io(io::Error::other(
                "accepted write failed",
            ))),
            Ok(()),
        );

        assert!(matches!(
            result,
            Err(crate::DaemonError::Io(error)) if error.to_string() == "accepted write failed"
        ));
    }

    #[tokio::test]
    async fn dropping_queued_reconciliation_marks_persisted_job_failed() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let queue = ReconciliationQueue::new();
        let first = queued(enqueue_reconciliation_job(
            &paths,
            &queue,
            ReconciliationScope::System,
        )?)?;
        let running = first.wait_for_turn().await;
        let queued_scope = ReconciliationScope::project("project_1")?;
        let queued = queued(enqueue_reconciliation_job(&paths, &queue, queued_scope)?)?;
        let queued_job_id = queued.job_id().to_string();

        drop(queued);

        let database = Database::open(&paths)?;
        let job = database
            .recent_jobs()?
            .into_iter()
            .find(|job| job.id == queued_job_id)
            .ok_or_else(|| anyhow::anyhow!("missing abandoned job {queued_job_id}"))?;
        assert_eq!(job.status, JobStatus::Failed);
        assert_eq!(
            job.error.as_deref(),
            Some("reconciliation was abandoned before completion")
        );

        running.finish();

        Ok(())
    }

    fn link_background_test_project(
        paths: &PvPaths,
        project_path: &Utf8Path,
        primary_hostname: &str,
    ) -> anyhow::Result<String> {
        let config_path = project_path.join("pv.yml");
        state::fs::write_sensitive_file(&config_path, "php: \"8.4\"\n")?;
        let project = Database::open(paths)?
            .link_project(LinkProjectInput {
                path: project_path.to_path_buf(),
                original_path: project_path.to_path_buf(),
                primary_hostname: primary_hostname.to_owned(),
                config_path,
                desired_php_track: Some("8.4".to_owned()),
                additional_hostnames: Vec::new(),
            })?
            .project;

        Ok(project.id)
    }

    fn queued(result: EnqueueResult) -> anyhow::Result<crate::QueuedReconciliation> {
        match result {
            EnqueueResult::Queued(queued) => Ok(queued),
            EnqueueResult::Coalesced(job) => Err(anyhow::anyhow!(
                "scope unexpectedly coalesced into {}",
                job.job_id()
            )),
        }
    }

    async fn wait_for_job_scope(paths: &PvPaths, scope: &str) -> anyhow::Result<()> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);

        loop {
            let database = Database::open(paths)?;
            if database.recent_jobs()?.iter().any(|job| job.scope == scope) {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("timed out waiting for job scope {scope}");
            }

            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    async fn reconciliation_events(
        paths: PvPaths,
        job_id: &str,
        scope: ReconciliationScope,
        catalog: &crate::managed_resources::ManagedResourceRuntimeCatalog,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        let (client, daemon) = duplex(64 * 1024);

        stream_started_reconciliation_job(
            paths,
            protocol::transport(daemon),
            true,
            job_id,
            scope,
            Some(catalog),
            ReconciliationJobTiming::immediate(),
        )
        .await?;

        let mut reader = protocol::transport(client);
        let mut events = Vec::new();
        while let Some(line) = reader.next().await {
            events.push(serde_json::from_str::<serde_json::Value>(&line?)?);
        }

        Ok(events)
    }

    fn live_phase_names(events: &[serde_json::Value]) -> Vec<&str> {
        events
            .iter()
            .filter(|event| event["type"] == "progress")
            .filter_map(|event| event["message"].as_str())
            .collect()
    }

    fn reconciliation_phase_events(
        paths: &PvPaths,
        job_id: &str,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        Ok(daemon_log_events(paths)?
            .into_iter()
            .filter(|event| {
                event["event"] == "reconciliation_phase_completed" && event["job_id"] == job_id
            })
            .collect())
    }

    fn daemon_log_events(paths: &PvPaths) -> anyhow::Result<Vec<serde_json::Value>> {
        Ok(state::fs::read_to_string(&paths.daemon_log())?
            .lines()
            .map(serde_json::from_str::<serde_json::Value>)
            .collect::<Result<Vec<_>, _>>()?)
    }

    #[derive(Clone, Copy)]
    enum ProjectPhaseScenario {
        /// The initial apply records PHP demand, Resources install the desired Gateway
        /// runtime artifacts (Caddy plus the PHP/FrankenPHP pair), and the post-install
        /// apply succeeds.
        Success,
        /// Project config is invalid, so the first Project Apply fails before any
        /// missing-artifact lookup runs.
        InvalidProjectConfig,
        /// The cached PHP archive is gone, so resource installation fails.
        MissingPhpArtifact,
        /// PHP installs but its extension metadata cannot be read, so only the post-install
        /// Project Apply fails.
        UnreadablePhpExtensions,
        /// The Project declares a backing resource PV has no adapter for and no seeded env
        /// context, so Resources has nothing to install and the apply records the failure.
        DeclaredBackingResource,
        /// The Project declares a backing resource PV does have an adapter for, but its
        /// artifact cannot be resolved, so the install fails inside Resources.
        DeclaredBackingResourceInstallFailure,
        /// The legacy repair pass and the final Project Apply both fail, so the apply error is
        /// primary and the repair error is preserved alongside it.
        RepairAndApplyFailure,
        /// The Project declares no resources and requests no optional PHP extensions, so it
        /// demands no artifact work even while other tracks are desired but missing.
        NoProjectArtifactDemand,
        /// The Project declares a backing resource PV has no adapter for, but whose track env
        /// context is seeded, so there is no artifact to install and the apply tolerates it.
        SeededResourceWithoutAdapter,
    }

    /// Runs one project reconciliation fixture scenario and returns its recorded phases plus
    /// job outcome.
    async fn run_project_php_extension_reconciliation(
        paths: &PvPaths,
        tempdir: &Utf8Path,
        scenario: ProjectPhaseScenario,
    ) -> anyhow::Result<(Vec<serde_json::Value>, String, bool)> {
        let project_path = tempdir.join("project");
        let config_path = project_path.join("pv.yml");

        state::fs::write_sensitive_file(
            &config_path,
            match scenario {
                ProjectPhaseScenario::InvalidProjectConfig => {
                    "php:\n  version: \"8.5\"\n  extensions: [redis]\nbogus_key: true\n"
                }
                ProjectPhaseScenario::DeclaredBackingResource => {
                    "mailpit:\n  version: \"1.0\"\n  env:\n    MAIL_HOST: \"${smtp_host}\"\n"
                }
                ProjectPhaseScenario::NoProjectArtifactDemand => "php: \"8.4\"\n",
                ProjectPhaseScenario::DeclaredBackingResourceInstallFailure => {
                    "serve: false\nmailpit:\n  version: \"1.0\"\n"
                }
                ProjectPhaseScenario::RepairAndApplyFailure => {
                    "php:\n  version: \"8.5\"\n  extensions: [redis]\nmailpit:\n  version: \"1.0\"\n"
                }
                ProjectPhaseScenario::SeededResourceWithoutAdapter => {
                    "serve: false\npostgres:\n  version: \"18\"\n"
                }
                _ => "php:\n  version: \"8.5\"\n  extensions: [redis]\n",
            },
        )?;
        match scenario {
            ProjectPhaseScenario::UnreadablePhpExtensions => seed_cached_php_pair_with_php_archive(
                paths,
                tempdir,
                seed_php_archive_with_invalid_extension_metadata,
            )?,
            // Left unseeded so no required artifact is available to install.
            ProjectPhaseScenario::DeclaredBackingResource
            | ProjectPhaseScenario::NoProjectArtifactDemand
            | ProjectPhaseScenario::SeededResourceWithoutAdapter
            | ProjectPhaseScenario::DeclaredBackingResourceInstallFailure => {}
            _ => seed_cached_php_pair(paths, tempdir)?,
        }
        if matches!(
            scenario,
            ProjectPhaseScenario::MissingPhpArtifact | ProjectPhaseScenario::RepairAndApplyFailure
        ) {
            remove_cached_archive(paths, PHP_TEST_ARCHIVE_FILE_NAME)?;
        }
        let mut database = Database::open(paths)?;
        if matches!(scenario, ProjectPhaseScenario::SeededResourceWithoutAdapter) {
            database.record_managed_resource_track_env_context(
                "postgres",
                "18",
                &BTreeMap::from([("host".to_string(), "127.0.0.1".to_string())]),
            )?;
        }
        let linked = database.link_project(LinkProjectInput {
            path: project_path.clone(),
            original_path: project_path,
            primary_hostname: "project.test".to_owned(),
            config_path,
            desired_php_track: None,
            additional_hostnames: Vec::new(),
        })?;
        drop(database);
        let scope = format!("project:{}", linked.project.id).parse::<ReconciliationScope>()?;
        let (events, succeeded) = match scenario {
            // An adapter-backed declaration needs a catalog that can reach the resource, so the
            // install is attempted and fails on the artifact rather than on a missing adapter.
            ProjectPhaseScenario::DeclaredBackingResourceInstallFailure => {
                let catalog = crate::managed_resources::fake_runtime_catalog_with_manifest_client(
                    OFFLINE_TEST_MANIFEST_URL,
                    ScriptedArtifactClient {
                        manifest: "not valid manifest JSON".to_owned(),
                        archive: Vec::new(),
                    },
                )?;

                run_reconciliation_job_with_catalog(paths, scope, &catalog).await?
            }
            _ => run_scenario_reconciliation_job(paths, scope).await?,
        };

        Ok((events, linked.project.id, succeeded))
    }

    /// Returns the job's recorded phases and whether the job itself succeeded, so a scenario
    /// cannot assert the right phases while the overall outcome is wrong.
    async fn run_scenario_reconciliation_job(
        paths: &PvPaths,
        scope: ReconciliationScope,
    ) -> anyhow::Result<(Vec<serde_json::Value>, bool)> {
        let catalog =
            crate::managed_resources::ManagedResourceRuntimeCatalog::without_adapters_with_manifest_url(
                OFFLINE_TEST_MANIFEST_URL,
            )?;

        run_reconciliation_job_with_catalog(paths, scope, &catalog).await
    }

    async fn run_reconciliation_job_with_catalog(
        paths: &PvPaths,
        scope: ReconciliationScope,
        catalog: &crate::managed_resources::ManagedResourceRuntimeCatalog,
    ) -> anyhow::Result<(Vec<serde_json::Value>, bool)> {
        let job_id = start_reconciliation_job(paths, &scope.to_string())?;
        let succeeded = complete_reconciliation_job_with_progress(
            paths,
            &job_id,
            &scope,
            Some(catalog),
            DaemonDownloadProgress::disabled(),
            ReconciliationJobTiming::immediate(),
            None,
        )
        .await
        .is_ok();

        Ok((reconciliation_phase_events(paths, &job_id)?, succeeded))
    }

    /// Every phase this sums must carry `elapsed_ms`. Staged top-level phases and any emitted
    /// operation records are expected not to nest, so their total cannot exceed the job's own
    /// execution time; this assertion trips if `CompleteApply` performs artifact work. Queue
    /// time is excluded because it is measured before execution begins, and finalization
    /// supplies the total rather than contributing to it.
    fn assert_phase_time_within_execution(events: &[serde_json::Value]) -> anyhow::Result<()> {
        let mut phase_total = 0;
        for event in events
            .iter()
            .filter(|event| event["phase"] != "queue" && event["phase"] != "finalization")
        {
            let elapsed = event["elapsed_ms"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("phase {} is missing elapsed_ms", event["phase"]))?;
            phase_total += elapsed;
        }
        let total_execution = events
            .iter()
            .find(|event| event["phase"] == "finalization")
            .and_then(|event| event["total_execution_ms"].as_u64())
            .ok_or_else(|| anyhow::anyhow!("missing finalization total_execution_ms"))?;

        assert!(
            phase_total <= total_execution,
            "phases overlap: {phase_total}ms of phase time exceeds {total_execution}ms of execution"
        );

        Ok(())
    }

    /// The `phase/subject` of every phase the job recorded as failed, which pins the job's
    /// outcome so a correct phase order cannot hide a wrong result.
    fn failed_phases(events: &[serde_json::Value]) -> Vec<String> {
        events
            .iter()
            .filter(|event| event["outcome"] == "failed")
            .map(|event| {
                format!(
                    "{}/{}",
                    event["phase"].as_str().unwrap_or_default(),
                    event["subject"].as_str().unwrap_or_default()
                )
            })
            .collect()
    }

    /// Every `phase/subject/outcome` the job recorded, for diagnosing an unexpected outcome.
    fn job_phase_outcomes(events: &[serde_json::Value]) -> Vec<String> {
        events
            .iter()
            .map(|event| {
                format!(
                    "{}/{}/{}",
                    event["phase"].as_str().unwrap_or_default(),
                    event["subject"].as_str().unwrap_or_default(),
                    event["outcome"].as_str().unwrap_or_default()
                )
            })
            .collect()
    }

    /// Ordered `phase/subject/outcome` records before Gateway work, with the project id
    /// normalized; includes any Manifest, Download, or Install records emitted by Project
    /// Apply.
    fn project_resource_phases(events: &[serde_json::Value], project_id: &str) -> Vec<String> {
        events
            .iter()
            .filter(|event| {
                matches!(
                    event["phase"].as_str(),
                    Some("project_apply" | "resources" | "manifest" | "download" | "install")
                )
            })
            .map(|event| {
                let phase = event["phase"].as_str().unwrap_or_default();
                let outcome = event["outcome"].as_str().unwrap_or_default();
                let subject = event["subject"].as_str().unwrap_or_default();
                let subject = if subject == project_id {
                    "project"
                } else {
                    subject
                };

                format!("{phase}/{subject}/{outcome}")
            })
            .collect()
    }

    fn remove_cached_archive(paths: &PvPaths, archive_file_name: &str) -> anyhow::Result<()> {
        for path in state::fs::read_dir_paths(paths.downloads())? {
            if path
                .file_name()
                .is_some_and(|name| name.ends_with(archive_file_name))
            {
                state::fs::remove_file(&path)?;
            }
        }

        Ok(())
    }

    async fn update_events(
        paths: PvPaths,
        job_id: &str,
        catalog: &crate::managed_resources::ManagedResourceRuntimeCatalog,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        let (client, daemon) = duplex(64 * 1024);

        stream_started_update_job(
            paths,
            protocol::transport(daemon),
            true,
            job_id,
            Some(catalog),
            ReconciliationJobTiming::immediate(),
            None,
        )
        .await
        .into_result()?;

        let mut reader = protocol::transport(client);
        let mut events = Vec::new();
        while let Some(line) = reader.next().await {
            events.push(serde_json::from_str::<serde_json::Value>(&line?)?);
        }

        Ok(events)
    }

    fn seed_cached_php_pair(paths: &PvPaths, tempdir: &Utf8Path) -> anyhow::Result<()> {
        seed_cached_php_pair_with_php_archive(paths, tempdir, seed_php_archive)
    }

    fn seed_cached_php_pair_with_php_archive(
        paths: &PvPaths,
        tempdir: &Utf8Path,
        seed_php_archive: fn(&Utf8Path, &Utf8Path) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        let caddy = CachedArtifact::new(
            "caddy",
            CADDY_TEST_ARCHIVE_FILE_NAME,
            CADDY_TEST_ARTIFACT_VERSION,
            seed_caddy_archive,
        );
        let php = CachedArtifact::new(
            "php",
            PHP_TEST_ARCHIVE_FILE_NAME,
            PHP_TEST_ARTIFACT_VERSION,
            seed_php_archive,
        );
        let frankenphp = CachedArtifact::new(
            "frankenphp",
            FRANKENPHP_TEST_ARCHIVE_FILE_NAME,
            PHP_TEST_ARTIFACT_VERSION,
            seed_frankenphp_archive,
        );
        let caddy = cache_artifact(paths, tempdir, caddy)?;
        let php = cache_artifact(paths, tempdir, php)?;
        let frankenphp = cache_artifact(paths, tempdir, frankenphp)?;
        let manifest = php_pair_manifest(&[caddy, php, frankenphp]);

        state::fs::write_sensitive_file(&paths.downloads().join("manifest.json"), &manifest)?;

        Ok(())
    }

    fn seed_caddy_archive(tempdir: &Utf8Path, archive_path: &Utf8Path) -> anyhow::Result<()> {
        let archive_parent = tempdir.join("caddy-archive");
        let root_name = format!("caddy-{CADDY_TEST_ARTIFACT_VERSION}");
        let executable = archive_parent.join(&root_name).join("bin/caddy");

        write_caddy_fixture(&executable)?;
        create_archive(&archive_parent, archive_path, &root_name)
    }

    fn seed_installed_caddy(paths: &PvPaths) -> anyhow::Result<()> {
        let release_path = paths
            .resources()
            .join("caddy")
            .join(CADDY_TEST_TRACK)
            .join("releases")
            .join(CADDY_TEST_ARTIFACT_VERSION);
        write_caddy_fixture(&release_path.join("bin/caddy"))?;
        let current_path = release_path
            .parent()
            .and_then(Utf8Path::parent)
            .ok_or_else(|| anyhow::anyhow!("missing Caddy track directory"))?
            .join("current");
        state::fs::symlink_file(
            &Utf8PathBuf::from(format!("releases/{CADDY_TEST_ARTIFACT_VERSION}")),
            &current_path,
        )?;

        let certified_key = generate_simple_self_signed(vec!["pv-gateway.localhost".to_owned()])?;
        state::fs::write_sensitive_file(&paths.ca_certificate(), &certified_key.cert.pem())?;
        state::fs::write_sensitive_file(
            &paths.ca_private_key(),
            &certified_key.signing_key.serialize_pem(),
        )?;

        let mut database = Database::open(paths)?;
        seed_gateway_ports(&mut database)?;
        database.record_managed_resource_track_installed(
            "caddy",
            CADDY_TEST_TRACK,
            CADDY_TEST_ARTIFACT_VERSION,
            &release_path,
        )?;

        Ok(())
    }

    fn seed_gateway_ports(database: &mut Database) -> anyhow::Result<()> {
        let mut listeners = Vec::with_capacity(2);
        let mut ports = Vec::with_capacity(2);
        while ports.len() < 2 {
            let listener = TcpListener::bind("127.0.0.1:0")?;
            let port = listener.local_addr()?.port();
            if ports.contains(&port) {
                continue;
            }

            ports.push(port);
            listeners.push(listener);
        }

        for (gateway_port, port) in [
            (GatewayPort::Http, ports[0]),
            (GatewayPort::Https, ports[1]),
        ] {
            database.assign_port(
                PortRequest::gateway(gateway_port, port, port, port),
                |_port| true,
            )?;
        }

        drop(listeners);
        Ok(())
    }

    fn write_caddy_fixture(executable: &Utf8Path) -> anyhow::Result<()> {
        state::fs::write_sensitive_file(executable, FAKE_CADDY_SCRIPT)?;
        state::fs::write_sensitive_file(
            &Utf8PathBuf::from(format!("{executable}.server.py")),
            FAKE_CADDY_SERVER_SCRIPT,
        )?;
        set_executable(executable)
    }

    struct SeededRuntimeRecord {
        label: String,
        pid_path: Utf8PathBuf,
        metadata_path: Utf8PathBuf,
        process_name: String,
        resource_name: String,
        track: String,
        command_root: Utf8PathBuf,
        config_path: Utf8PathBuf,
        log_path: Utf8PathBuf,
        arguments: Option<Vec<String>>,
    }

    struct SeededRuntimeGuard {
        paths: Option<PvPaths>,
        runtimes: Vec<SeededRuntimeRecord>,
    }

    impl SeededRuntimeGuard {
        fn with_gateway(paths: PvPaths) -> Self {
            let gateway = seeded_gateway_record(&paths);
            Self {
                paths: Some(paths),
                runtimes: vec![gateway],
            }
        }

        fn with_resource(paths: PvPaths, resource_name: &str, track: &str) -> Self {
            let mut guard = Self {
                paths: Some(paths),
                runtimes: Vec::new(),
            };
            guard.register_resource(resource_name, track);
            guard
        }

        fn register_worker(&mut self, runtime_key: &str, command_root: &Utf8Path) {
            if let Some(paths) = self.paths.as_ref() {
                let config_path = paths.worker_root_config(runtime_key);
                self.runtimes.push(SeededRuntimeRecord {
                    label: format!("FrankenPHP worker {runtime_key}"),
                    pid_path: paths.worker_pid(runtime_key),
                    metadata_path: paths.worker_runtime_metadata(runtime_key),
                    process_name: format!("php-worker-{runtime_key}"),
                    resource_name: "frankenphp".to_owned(),
                    track: runtime_key.to_owned(),
                    command_root: command_root.to_path_buf(),
                    config_path: config_path.clone(),
                    log_path: paths.worker_log(runtime_key),
                    arguments: Some(vec![
                        "run".to_owned(),
                        "--config".to_owned(),
                        config_path.into_string(),
                        "--adapter".to_owned(),
                        "caddyfile".to_owned(),
                    ]),
                });
            }
        }

        fn register_resource(&mut self, resource_name: &str, track: &str) {
            if let Some(paths) = self.paths.as_ref() {
                self.runtimes.push(SeededRuntimeRecord {
                    label: format!("Managed Resource {resource_name} {track}"),
                    pid_path: paths.resource_pid(resource_name, track),
                    metadata_path: paths.resource_runtime_metadata(resource_name, track),
                    process_name: format!("{resource_name}-{track}"),
                    resource_name: resource_name.to_owned(),
                    track: track.to_owned(),
                    command_root: paths.resources().join(resource_name).join(track),
                    config_path: paths.resource_runtime_config(resource_name, track),
                    log_path: paths.resource_log(resource_name, track),
                    arguments: None,
                });
            }
        }

        async fn cleanup(&mut self) -> anyhow::Result<()> {
            let Some(paths) = self.paths.as_ref() else {
                return Ok(());
            };
            stop_seeded_runtimes(paths, &self.runtimes).await?;
            self.paths = None;
            Ok(())
        }
    }

    fn seeded_gateway_record(paths: &PvPaths) -> SeededRuntimeRecord {
        let config_path = paths.gateway_root_config();
        SeededRuntimeRecord {
            label: "Gateway".to_owned(),
            pid_path: paths.gateway_pid(),
            metadata_path: paths.gateway_runtime_metadata(),
            process_name: "gateway".to_owned(),
            resource_name: "caddy".to_owned(),
            track: CADDY_TEST_TRACK.to_owned(),
            command_root: paths.resources().join("caddy").join(CADDY_TEST_TRACK),
            config_path: config_path.clone(),
            log_path: paths.gateway_supervisor_log(),
            arguments: Some(vec![
                "run".to_owned(),
                "--config".to_owned(),
                config_path.into_string(),
                "--adapter".to_owned(),
                "caddyfile".to_owned(),
            ]),
        }
    }

    impl Drop for SeededRuntimeGuard {
        fn drop(&mut self) {
            let Some(paths) = self.paths.as_ref() else {
                return;
            };
            let runtimes = &self.runtimes;
            let cleanup_result = std::thread::scope(|scope| {
                let cleanup_thread = std::thread::Builder::new().spawn_scoped(scope, || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            anyhow::anyhow!("cleanup runtime construction failed: {error}")
                        })?;

                    runtime.block_on(stop_seeded_runtimes(paths, runtimes))
                });
                match cleanup_thread {
                    Ok(cleanup_thread) => match cleanup_thread.join() {
                        Ok(result) => result.map_err(|error| format!("cleanup failed: {error:#}")),
                        Err(_panic) => Err("cleanup thread panicked".to_owned()),
                    },
                    Err(error) => Err(format!("cleanup thread construction failed: {error}")),
                }
            });
            if let Err(failure) = cleanup_result {
                let failure = seeded_cleanup_failure_with_emergency(
                    failure,
                    emergency_cleanup_seeded_runtimes(paths, runtimes),
                );
                report_seeded_runtime_cleanup_failure(paths, &failure);
            }
        }
    }

    fn seeded_cleanup_failure_with_emergency(
        primary: String,
        emergency: anyhow::Result<()>,
    ) -> String {
        match emergency {
            Ok(()) => primary,
            Err(error) => format!("{primary}; emergency cleanup failed: {error:#}"),
        }
    }

    fn emergency_cleanup_seeded_runtimes(
        paths: &PvPaths,
        runtimes: &[SeededRuntimeRecord],
    ) -> anyhow::Result<()> {
        let supervisor = ProcessSupervisor::new(paths.clone());
        let mut failures = Vec::new();

        for runtime in runtimes {
            let publication_deadline = std::time::Instant::now() + Duration::from_millis(500);
            if let Err(error) =
                emergency_cleanup_seeded_runtime(&supervisor, runtime, publication_deadline)
            {
                failures.push(format!("{}: {error:#}", runtime.label));
            }
        }

        if failures.is_empty() {
            Ok(())
        } else {
            anyhow::bail!(failures.join("; "))
        }
    }

    fn emergency_cleanup_seeded_runtime(
        supervisor: &ProcessSupervisor,
        runtime: &SeededRuntimeRecord,
        publication_deadline: std::time::Instant,
    ) -> anyhow::Result<()> {
        loop {
            let pid_exists = state::fs::path_entry_exists(&runtime.pid_path)?;
            let metadata_exists = state::fs::path_entry_exists(&runtime.metadata_path)?;
            match (pid_exists, metadata_exists) {
                (false, false) if std::time::Instant::now() >= publication_deadline => {
                    return Ok(());
                }
                (false, false) => {}
                (true, true) => {
                    if !seeded_runtime_record_matches_expected(runtime)? {
                        anyhow::bail!("runtime metadata does not match its registered identity");
                    }
                    let pid_snapshot = state::fs::read_to_string(&runtime.pid_path)?;
                    let metadata_snapshot = state::fs::read_to_string(&runtime.metadata_path)?;
                    if let Some(process) =
                        supervisor.adopt_recorded(&runtime.pid_path, &runtime.metadata_path)?
                    {
                        process.kill_and_wait_for_test(Duration::from_secs(1))?;
                        let records_unchanged = state::fs::read_to_string(&runtime.pid_path)
                            .is_ok_and(|contents| contents == pid_snapshot)
                            && state::fs::read_to_string(&runtime.metadata_path)
                                .is_ok_and(|contents| contents == metadata_snapshot);
                        if records_unchanged {
                            state::fs::remove_file_if_exists(&runtime.pid_path)?;
                            state::fs::remove_file_if_exists(&runtime.metadata_path)?;
                        } else if std::time::Instant::now() >= publication_deadline {
                            anyhow::bail!("runtime records changed during emergency cleanup");
                        }
                    } else if std::time::Instant::now() >= publication_deadline {
                        anyhow::bail!("runtime was not adoptable through its recorded identity");
                    }
                }
                _ if std::time::Instant::now() >= publication_deadline => {
                    anyhow::bail!("runtime has incomplete ownership records");
                }
                _ => {}
            }

            if std::time::Instant::now() < publication_deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    fn report_seeded_runtime_cleanup_failure(paths: &PvPaths, message: &str) {
        let record = json!({
            "level": "error",
            "target": "jobs",
            "event": "seeded_runtime_cleanup_failed",
            "message": message,
        });
        if let Ok(mut log) = state::fs::open_append_file(&paths.daemon_log()) {
            let _write_result = log.write_all(format!("{record}\n").as_bytes());
        }
        let _write_result = writeln!(io::stderr().lock(), "{record}");
    }

    async fn stop_seeded_caddy(paths: &PvPaths) -> anyhow::Result<()> {
        stop_seeded_runtimes(paths, &[seeded_gateway_record(paths)]).await
    }

    async fn stop_seeded_runtimes(
        paths: &PvPaths,
        runtimes: &[SeededRuntimeRecord],
    ) -> anyhow::Result<()> {
        let supervisor = ProcessSupervisor::new(paths.clone());
        let mut failures = Vec::new();

        for runtime in runtimes {
            if let Err(error) = stop_seeded_runtime(&supervisor, runtime).await {
                failures.push(format!("{}: {error}", runtime.label));
            }
        }

        if failures.is_empty() {
            Ok(())
        } else {
            anyhow::bail!(failures.join("; "))
        }
    }

    async fn stop_seeded_runtime(
        supervisor: &ProcessSupervisor,
        runtime: &SeededRuntimeRecord,
    ) -> anyhow::Result<()> {
        let pid_path = &runtime.pid_path;
        let metadata_path = &runtime.metadata_path;
        let deadline = Instant::now() + Duration::from_millis(500);

        loop {
            let has_pid = state::fs::path_entry_exists(pid_path)?;
            let has_metadata = state::fs::path_entry_exists(metadata_path)?;
            if !has_pid && !has_metadata {
                if Instant::now() >= deadline {
                    return Ok(());
                }
                sleep(Duration::from_millis(10)).await;
                continue;
            }
            if has_metadata && !seeded_runtime_record_matches_expected(runtime)? {
                anyhow::bail!("runtime metadata does not match its registered identity");
            }
            if has_pid && !has_metadata {
                if Instant::now() < deadline {
                    sleep(Duration::from_millis(10)).await;
                    continue;
                }
                let pid = state::fs::read_to_string(pid_path)?.trim().parse::<u32>()?;
                if !seeded_runtime_process_and_group_are_absent(pid)? {
                    anyhow::bail!(
                        "pid was published without metadata while its process group is alive"
                    );
                }
                state::fs::remove_file_if_exists(pid_path)?;
                return Ok(());
            }
            if !has_pid {
                if Instant::now() < deadline {
                    sleep(Duration::from_millis(10)).await;
                    continue;
                }
                let metadata: serde_json::Value =
                    serde_json::from_str(&state::fs::read_to_string(metadata_path)?)?;
                let pid = metadata["pid"]
                    .as_u64()
                    .and_then(|pid| u32::try_from(pid).ok())
                    .ok_or_else(|| anyhow::anyhow!("runtime metadata has no valid pid"))?;
                if !seeded_runtime_process_and_group_are_absent(pid)? {
                    anyhow::bail!("metadata still names a live process group");
                }
                state::fs::remove_file_if_exists(metadata_path)?;
                return Ok(());
            }

            let Some(process) = supervisor.adopt_recorded(pid_path, metadata_path)? else {
                if Instant::now() >= deadline {
                    if remove_absent_seeded_runtime_records(runtime)? {
                        return Ok(());
                    }
                    anyhow::bail!("runtime was not adoptable");
                }
                sleep(Duration::from_millis(10)).await;
                continue;
            };
            let pid = process.pid();
            let stop_result = match process.stop(Duration::from_secs(1)).await {
                Ok(()) => Ok(()),
                Err(stop_error) => match seeded_runtime_process_and_group_are_absent(pid) {
                    Ok(true)
                        if matches!(
                            stop_error,
                            DaemonError::RuntimeProcessIdentityChanged { .. }
                        ) =>
                    {
                        Ok(())
                    }
                    Ok(true) => Err(stop_error),
                    Ok(false) => anyhow::bail!(
                        "stop failed: {stop_error}; process group remained after verified cleanup"
                    ),
                    Err(inspection_error) => anyhow::bail!(
                        "stop failed: {stop_error}; process-group inspection failed: {inspection_error}"
                    ),
                },
            };
            let record_cleanup = (|| {
                state::fs::remove_file_if_exists(pid_path)?;
                state::fs::remove_file_if_exists(metadata_path)?;
                if state::fs::path_entry_exists(pid_path)?
                    || state::fs::path_entry_exists(metadata_path)?
                {
                    anyhow::bail!("runtime files remained after cleanup");
                }
                Ok::<_, anyhow::Error>(())
            })();

            return match (stop_result, record_cleanup) {
                (Ok(()), Ok(())) => Ok(()),
                (Err(stop_error), Ok(())) => Err(stop_error.into()),
                (Ok(()), Err(cleanup_error)) => Err(cleanup_error),
                (Err(stop_error), Err(cleanup_error)) => anyhow::bail!(
                    "stop failed: {stop_error}; record cleanup failed: {cleanup_error:#}"
                ),
            };
        }
    }

    fn remove_absent_seeded_runtime_records(runtime: &SeededRuntimeRecord) -> anyhow::Result<bool> {
        if !seeded_runtime_record_matches_expected(runtime)? {
            anyhow::bail!("runtime metadata does not match its registered identity");
        }
        let pid = state::fs::read_to_string(&runtime.pid_path)?
            .trim()
            .parse::<u32>()?;
        let metadata: serde_json::Value =
            serde_json::from_str(&state::fs::read_to_string(&runtime.metadata_path)?)?;
        let metadata_pid = metadata["pid"]
            .as_u64()
            .and_then(|pid| u32::try_from(pid).ok())
            .ok_or_else(|| anyhow::anyhow!("runtime metadata has no valid pid"))?;

        for recorded_pid in [pid, metadata_pid] {
            if !seeded_runtime_process_and_group_are_absent(recorded_pid)? {
                return Ok(false);
            }
        }

        state::fs::remove_file_if_exists(&runtime.pid_path)?;
        state::fs::remove_file_if_exists(&runtime.metadata_path)?;
        if state::fs::path_entry_exists(&runtime.pid_path)?
            || state::fs::path_entry_exists(&runtime.metadata_path)?
        {
            anyhow::bail!("runtime files remained after cleanup");
        }

        Ok(true)
    }

    fn seeded_runtime_record_matches_expected(
        runtime: &SeededRuntimeRecord,
    ) -> anyhow::Result<bool> {
        let metadata: serde_json::Value =
            serde_json::from_str(&state::fs::read_to_string(&runtime.metadata_path)?)?;
        let arguments_match = runtime
            .arguments
            .as_ref()
            .is_none_or(|arguments| metadata["arguments"] == json!(arguments));

        Ok(metadata["name"] == runtime.process_name
            && metadata["resource_name"] == runtime.resource_name
            && metadata["track"] == runtime.track
            && metadata["command"]
                .as_str()
                .is_some_and(|command| Utf8Path::new(command).starts_with(&runtime.command_root))
            && metadata["config_path"] == runtime.config_path.as_str()
            && metadata["log_path"] == runtime.log_path.as_str()
            && arguments_match)
    }

    #[cfg(target_os = "macos")]
    fn seeded_runtime_process_and_group_are_absent(pid: u32) -> anyhow::Result<bool> {
        let process = Pid::from_raw(i32::try_from(pid)?)
            .ok_or_else(|| anyhow::anyhow!("invalid seeded runtime pid {pid}"))?;
        let leader_absent = match test_kill_process(process) {
            Ok(()) => false,
            Err(rustix::io::Errno::SRCH) => true,
            Err(error) => anyhow::bail!("failed to inspect seeded runtime pid {pid}: {error}"),
        };
        let group_absent = match test_kill_process_group(process) {
            Ok(()) => false,
            Err(rustix::io::Errno::SRCH) => true,
            Err(error) => {
                anyhow::bail!("failed to inspect seeded runtime process group {pid}: {error}")
            }
        };

        Ok(leader_absent && group_absent)
    }

    #[cfg(not(target_os = "macos"))]
    fn seeded_runtime_process_and_group_are_absent(_pid: u32) -> anyhow::Result<bool> {
        anyhow::bail!("seeded runtime process-group inspection is unsupported on this platform")
    }

    #[derive(Clone, Copy)]
    struct CachedArtifact {
        resource_name: &'static str,
        archive_file_name: &'static str,
        artifact_version: &'static str,
        seed_archive: fn(&Utf8Path, &Utf8Path) -> anyhow::Result<()>,
    }

    impl CachedArtifact {
        fn new(
            resource_name: &'static str,
            archive_file_name: &'static str,
            artifact_version: &'static str,
            seed_archive: fn(&Utf8Path, &Utf8Path) -> anyhow::Result<()>,
        ) -> Self {
            Self {
                resource_name,
                archive_file_name,
                artifact_version,
                seed_archive,
            }
        }
    }

    struct CachedManifestArtifact {
        artifact: CachedArtifact,
        sha256: String,
        size: u64,
    }

    fn cache_artifact(
        paths: &PvPaths,
        tempdir: &Utf8Path,
        artifact: CachedArtifact,
    ) -> anyhow::Result<CachedManifestArtifact> {
        let archive_path = tempdir.join(artifact.archive_file_name);

        (artifact.seed_archive)(tempdir, &archive_path)?;
        let sha256 = sha256_file(&archive_path)?;
        let cache_path = paths
            .downloads()
            .join(format!("{sha256}-{}", artifact.archive_file_name));

        copy_file(&archive_path, &cache_path)?;

        Ok(CachedManifestArtifact {
            artifact,
            sha256,
            size: file_size(&cache_path)?,
        })
    }

    fn seed_php_archive(tempdir: &Utf8Path, archive_path: &Utf8Path) -> anyhow::Result<()> {
        seed_php_archive_with_extension_metadata(
            tempdir,
            archive_path,
            r#"[{"name":"redis","load_kind":"extension","path":"lib/php/extensions/redis.so"}]"#,
        )
    }

    /// Installs cleanly but cannot be read for optional extension modules, so the artifact
    /// only fails the Project Apply that runs after installation.
    fn seed_php_archive_with_invalid_extension_metadata(
        tempdir: &Utf8Path,
        archive_path: &Utf8Path,
    ) -> anyhow::Result<()> {
        seed_php_archive_with_extension_metadata(
            tempdir,
            archive_path,
            "not valid extension metadata",
        )
    }

    fn seed_php_archive_with_extension_metadata(
        tempdir: &Utf8Path,
        archive_path: &Utf8Path,
        extension_metadata: &str,
    ) -> anyhow::Result<()> {
        let archive_parent = tempdir.join("php-archive");
        let root_name = format!("php-{PHP_TEST_ARTIFACT_VERSION}");
        let root = archive_parent.join(&root_name);
        let executable = root.join("bin/php");

        state::fs::write_sensitive_file(&executable, "#!/bin/sh\nexit 0\n")?;
        set_executable(&executable)?;
        state::fs::write_sensitive_file(
            &root.join("share/pv/php-extensions.json"),
            extension_metadata,
        )?;
        state::fs::write_sensitive_file(&root.join("lib/php/extensions/redis.so"), "")?;
        create_archive(&archive_parent, archive_path, &root_name)
    }

    fn seed_frankenphp_archive(tempdir: &Utf8Path, archive_path: &Utf8Path) -> anyhow::Result<()> {
        let archive_parent = tempdir.join("frankenphp-archive");
        let root_name = format!("frankenphp-{PHP_TEST_ARTIFACT_VERSION}");
        let root = archive_parent.join(&root_name);
        let executable = root.join("bin/frankenphp");

        state::fs::write_sensitive_file(&executable, "#!/bin/sh\nexit 0\n")?;
        set_executable(&executable)?;
        create_archive(&archive_parent, archive_path, &root_name)
    }

    fn scripted_artifact_client(
        tempdir: &Utf8Path,
        resource_name: &str,
        track: &str,
        artifact_version: &str,
        archive_file_name: &str,
        executable_relative_path: &str,
    ) -> anyhow::Result<(ScriptedArtifactClient, u64)> {
        let archive_path = tempdir.join(archive_file_name);

        seed_artifact_archive(
            tempdir,
            &archive_path,
            resource_name,
            artifact_version,
            executable_relative_path,
        )?;
        let archive = read_file(&archive_path)?;
        let total_bytes = archive.len() as u64;
        let sha256 = sha256_file(&archive_path)?;
        let upstream_version = artifact_version
            .strip_suffix("-pv1")
            .unwrap_or(artifact_version);
        let manifest = serde_json::to_string(&json!({
            "schema_version": 1,
            "minimum_pv_version": "0.1.0",
            "resources": [{
                "name": resource_name,
                "default_track": track,
                "tracks": [{
                    "name": track,
                    "artifacts": [{
                        "artifact_version": artifact_version,
                        "upstream_version": upstream_version,
                        "pv_build_revision": "1",
                        "platform": "any",
                        "url": format!("https://artifacts.example.test/{archive_file_name}"),
                        "sha256": sha256,
                        "size": total_bytes,
                        "published_at": "2026-06-08T00:00:00Z",
                    }],
                }],
            }],
        }))?;

        Ok((ScriptedArtifactClient { manifest, archive }, total_bytes))
    }

    fn seed_artifact_archive(
        tempdir: &Utf8Path,
        archive_path: &Utf8Path,
        resource_name: &str,
        artifact_version: &str,
        executable_relative_path: &str,
    ) -> anyhow::Result<()> {
        let archive_parent = tempdir.join(format!("{resource_name}-archive"));
        let root_name = format!("{resource_name}-{artifact_version}");
        let root = archive_parent.join(&root_name);
        let executable = root.join(executable_relative_path);

        state::fs::write_sensitive_file(&executable, "#!/bin/sh\nexit 0\n")?;
        set_executable(&executable)?;
        create_archive(&archive_parent, archive_path, &root_name)
    }

    fn seed_installed_artifact(
        paths: &PvPaths,
        resource_name: &str,
        track: &str,
        artifact_version: &str,
        executable_relative_path: &str,
    ) -> anyhow::Result<()> {
        let release_path = paths
            .resources()
            .join(resource_name)
            .join(track)
            .join("releases")
            .join(artifact_version);
        let executable = release_path.join(executable_relative_path);
        state::fs::write_sensitive_file(&executable, "#!/bin/sh\nexit 0\n")?;
        set_executable(&executable)?;
        state::fs::symlink_file(
            &Utf8PathBuf::from(format!("releases/{artifact_version}")),
            &paths
                .resources()
                .join(resource_name)
                .join(track)
                .join("current"),
        )?;
        Database::open(paths)?.record_managed_resource_track_installed(
            resource_name,
            track,
            artifact_version,
            &release_path,
        )?;

        Ok(())
    }

    #[derive(Debug)]
    struct ScriptedArtifactClient {
        manifest: String,
        archive: Vec<u8>,
    }

    impl resources::ResourceHttpClient for ScriptedArtifactClient {
        fn get_text(&self, _url: &str) -> resources::Result<String> {
            Ok(self.manifest.clone())
        }

        fn download(&self, url: &str, writer: &mut dyn Write) -> resources::Result<()> {
            writer.write_all(&self.archive).map_err(|error| {
                resources::ResourcesError::HttpRequestFailed {
                    url: url.to_string(),
                    reason: error.to_string(),
                }
            })
        }
    }

    #[derive(Debug)]
    struct HeldManifestArtifactClient {
        inner: ScriptedArtifactClient,
        release_receiver: Mutex<mpsc::Receiver<()>>,
        started: Option<Arc<AtomicBool>>,
    }

    impl resources::ResourceHttpClient for HeldManifestArtifactClient {
        fn get_text(&self, url: &str) -> resources::Result<String> {
            if let Some(started) = &self.started {
                started.store(true, Ordering::SeqCst);
            }
            let release_receiver = match self.release_receiver.lock() {
                Ok(release_receiver) => release_receiver,
                Err(poisoned) => poisoned.into_inner(),
            };
            release_receiver
                .recv_timeout(Duration::from_secs(5))
                .map_err(|error| resources::ResourcesError::HttpRequestFailed {
                    url: url.to_owned(),
                    reason: format!("held manifest request was not released: {error}"),
                })?;

            self.inner.get_text(url)
        }

        fn download(&self, url: &str, writer: &mut dyn Write) -> resources::Result<()> {
            self.inner.download(url, writer)
        }
    }

    #[derive(Debug)]
    struct DelayedScriptedArtifactClient {
        inner: ScriptedArtifactClient,
        delay: Duration,
    }

    struct FailingArtifactClient {
        inner: ScriptedArtifactClient,
        download_failures: usize,
        download_attempts: Arc<AtomicUsize>,
        manifest_requests: Arc<AtomicUsize>,
    }

    impl ResourceHttpClient for FailingArtifactClient {
        fn get_text(&self, url: &str) -> resources::Result<String> {
            self.manifest_requests.fetch_add(1, Ordering::SeqCst);
            self.inner.get_text(url)
        }

        fn download(&self, url: &str, writer: &mut dyn Write) -> resources::Result<()> {
            let attempt = self.download_attempts.fetch_add(1, Ordering::SeqCst);
            if attempt < self.download_failures {
                return Err(ResourcesError::HttpStatusFailed {
                    url: url.to_owned(),
                    status_code: if attempt == 0 { 404 } else { 410 },
                });
            }
            self.inner.download(url, writer)
        }
    }

    impl DelayedScriptedArtifactClient {
        fn new(inner: ScriptedArtifactClient, delay: Duration) -> Self {
            Self { inner, delay }
        }
    }

    impl resources::ResourceHttpClient for DelayedScriptedArtifactClient {
        fn get_text(&self, url: &str) -> resources::Result<String> {
            self.inner.get_text(url)
        }

        fn download(&self, url: &str, writer: &mut dyn Write) -> resources::Result<()> {
            std::thread::sleep(self.delay);
            self.inner.download(url, writer)
        }
    }

    #[derive(Debug)]
    struct MultiArtifactClient {
        manifest: String,
        archives: BTreeMap<String, Vec<u8>>,
    }

    struct LinkingProjectArtifactClient {
        inner: MultiArtifactClient,
        paths: PvPaths,
        project: LinkProjectInput,
    }

    impl ResourceHttpClient for LinkingProjectArtifactClient {
        fn get_text(&self, url: &str) -> resources::Result<String> {
            Database::open(&self.paths)
                .and_then(|mut database| database.link_project(self.project.clone()))
                .map_err(|error| ResourcesError::Filesystem {
                    path: self.paths.db().to_string(),
                    reason: error.to_string(),
                })?;
            self.inner.get_text(url)
        }

        fn download(&self, url: &str, writer: &mut dyn Write) -> resources::Result<()> {
            self.inner.download(url, writer)
        }
    }

    struct ReconfiguringProjectArtifactClient {
        inner: MultiArtifactClient,
        config_path: Utf8PathBuf,
        config: String,
    }

    impl ResourceHttpClient for ReconfiguringProjectArtifactClient {
        fn get_text(&self, url: &str) -> resources::Result<String> {
            state::fs::write_sensitive_file(&self.config_path, &self.config).map_err(|error| {
                ResourcesError::Filesystem {
                    path: self.config_path.to_string(),
                    reason: error.to_string(),
                }
            })?;
            self.inner.get_text(url)
        }

        fn download(&self, url: &str, writer: &mut dyn Write) -> resources::Result<()> {
            self.inner.download(url, writer)
        }
    }

    struct SequencedMultiArtifactClient {
        manifests: Mutex<VecDeque<String>>,
        archives: BTreeMap<String, Vec<u8>>,
        manifest_requests: Arc<AtomicUsize>,
    }

    impl resources::ResourceHttpClient for SequencedMultiArtifactClient {
        fn get_text(&self, url: &str) -> resources::Result<String> {
            self.manifest_requests.fetch_add(1, Ordering::SeqCst);
            self.manifests
                .lock()
                .map_err(|_poison| resources::ResourcesError::HttpRequestFailed {
                    url: url.to_string(),
                    reason: "manifest response lock poisoned".to_string(),
                })?
                .pop_front()
                .ok_or_else(|| resources::ResourcesError::HttpRequestFailed {
                    url: url.to_string(),
                    reason: "no scripted manifest response".to_string(),
                })
        }

        fn download(&self, url: &str, writer: &mut dyn Write) -> resources::Result<()> {
            let archive = self.archives.get(url).ok_or_else(|| {
                resources::ResourcesError::HttpRequestFailed {
                    url: url.to_string(),
                    reason: "missing scripted archive".to_string(),
                }
            })?;
            writer.write_all(archive).map_err(|error| {
                resources::ResourcesError::HttpRequestFailed {
                    url: url.to_string(),
                    reason: error.to_string(),
                }
            })
        }
    }

    impl resources::ResourceHttpClient for MultiArtifactClient {
        fn get_text(&self, _url: &str) -> resources::Result<String> {
            Ok(self.manifest.clone())
        }

        fn download(&self, url: &str, writer: &mut dyn Write) -> resources::Result<()> {
            let archive = self.archives.get(url).ok_or_else(|| {
                resources::ResourcesError::HttpRequestFailed {
                    url: url.to_string(),
                    reason: "missing scripted archive".to_string(),
                }
            })?;

            writer.write_all(archive).map_err(|error| {
                resources::ResourcesError::HttpRequestFailed {
                    url: url.to_string(),
                    reason: error.to_string(),
                }
            })
        }
    }

    fn manifest_artifact(
        artifact_version: &str,
        upstream_version: &str,
        url: &str,
        sha256: &str,
        size: u64,
    ) -> serde_json::Value {
        json!({
            "artifact_version": artifact_version,
            "upstream_version": upstream_version,
            "pv_build_revision": "1",
            "platform": "any",
            "url": url,
            "sha256": sha256,
            "size": size,
            "published_at": "2026-06-08T00:00:00Z",
        })
    }

    fn php_pair_manifest(artifacts: &[CachedManifestArtifact]) -> String {
        let resources = artifacts
            .iter()
            .map(php_pair_manifest_resource)
            .collect::<Vec<_>>()
            .join(",\n");

        format!(
            r#"{{
  "schema_version": 1,
  "minimum_pv_version": "0.1.0",
  "resources": [
{resources}
  ]
}}
"#
        )
    }

    fn php_pair_manifest_resource(cached: &CachedManifestArtifact) -> String {
        let artifact = cached.artifact;
        let track = if artifact.resource_name == "caddy" {
            CADDY_TEST_TRACK
        } else {
            PHP_TEST_TRACK
        };
        let upstream_version = artifact
            .artifact_version
            .strip_suffix("-pv1")
            .unwrap_or(artifact.artifact_version);

        format!(
            r#"    {{
      "name": "{resource_name}",
      "default_track": "{track}",
      "tracks": [
        {{
          "name": "{track}",
          "artifacts": [
            {{
              "artifact_version": "{artifact_version}",
              "upstream_version": "{upstream_version}",
              "pv_build_revision": "1",
              "platform": "any",
              "url": "https://artifacts.example.test/{archive_file_name}",
              "sha256": "{sha256}",
              "size": {size},
              "published_at": "2026-06-08T00:00:00Z"
            }}
          ]
        }}
      ]
            }}"#,
            resource_name = artifact.resource_name,
            track = track,
            artifact_version = artifact.artifact_version,
            upstream_version = upstream_version,
            archive_file_name = artifact.archive_file_name,
            sha256 = cached.sha256,
            size = cached.size,
        )
    }

    fn create_archive(
        archive_parent: &Utf8Path,
        archive_path: &Utf8Path,
        root_name: &str,
    ) -> anyhow::Result<()> {
        run_fixture_command(
            "tar",
            &[
                "-czf",
                archive_path.as_str(),
                "-C",
                archive_parent.as_str(),
                root_name,
            ],
        )?;

        Ok(())
    }

    fn sha256_file(path: &Utf8Path) -> anyhow::Result<String> {
        let output = run_fixture_command("shasum", &["-a", "256", path.as_str()])
            .or_else(|_error| run_fixture_command("sha256sum", &[path.as_str()]))?;
        let text = String::from_utf8(output)?;
        let Some(sha256) = text.split_whitespace().next() else {
            anyhow::bail!("shasum output did not include a sha256 digest");
        };

        Ok(sha256.to_string())
    }

    #[expect(
        clippy::disallowed_types,
        reason = "daemon jobs tests shell out to build archive fixtures without extra dev-dependencies"
    )]
    fn run_fixture_command(program: &str, args: &[&str]) -> anyhow::Result<Vec<u8>> {
        let output = process::Command::new(program)
            .env("COPYFILE_DISABLE", "1")
            .args(args)
            .output()?;
        if !output.status.success() {
            anyhow::bail!(
                "fixture command `{program}` failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(output.stdout)
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "daemon jobs tests seed cached artifact fixtures directly"
    )]
    fn copy_file(from: &Utf8Path, to: &Utf8Path) -> anyhow::Result<()> {
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(from, to)?;

        Ok(())
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "daemon jobs tests read generated archive fixture bytes"
    )]
    fn read_file(path: &Utf8Path) -> anyhow::Result<Vec<u8>> {
        Ok(fs::read(path)?)
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "daemon jobs tests read fixture archive metadata for manifest size"
    )]
    fn file_size(path: &Utf8Path) -> anyhow::Result<u64> {
        Ok(fs::metadata(path)?.len())
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "daemon jobs tests set fixture executable bits directly"
    )]
    fn set_executable(path: &Utf8Path) -> anyhow::Result<()> {
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)?;

        Ok(())
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "daemon jobs tests create fixture directories"
    )]
    fn create_directory(path: &Utf8Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(path)?;

        Ok(())
    }
}
