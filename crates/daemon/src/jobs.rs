use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::io;
use std::sync::{Arc, Mutex, OnceLock};

use crate::DaemonError;
use crate::gateway::{
    CADDY_NOT_INSTALLED, ProjectGatewayReconciliationOutcome, reconcile_gateway_runtimes,
    reconcile_gateway_runtimes_with_phase_log, reconcile_project_gateway_runtimes_with_phase_log,
};
use crate::ipc::LocalStream;
use crate::managed_resources::{
    ManagedResourceRuntimeCatalog, ManagedResourceUpdateReport,
    reconcile_persisted_resource_track_with_progress,
    reconcile_system_resources_with_catalog_and_progress, reconcile_system_resources_with_progress,
    stop_undemanded_system_resource_runtimes,
};
use crate::project_env::{
    DemandedResourceTrack, ProjectDemand, discover_project_demand,
    reconcile_project_env_from_persisted_state,
    reconcile_project_env_with_runtime_catalog_and_progress, record_project_env_failure,
};
use crate::reconciliation::{
    EnqueueResult, QueuedReconciliation, ReconciliationJobTiming, ReconciliationQueue,
    ReconciliationScope, RunningReconciliation,
};
use crate::structured_log::{self, PhaseOutcome, ReconciliationPhase, ReconciliationPhaseLog};
use protocol::{DaemonEvent, DaemonResponse, DaemonTransport, write_line};
use state::{
    Database, JobDiagnosticSubject, ManagedResourceDesiredState, ProjectRecord, PvPaths, StateError,
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
struct StreamedJobCompletion {
    result: Result<String, DaemonError>,
    transport_is_open: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct DaemonDownloadProgress {
    sender: Option<Sender<ForegroundJobEvent>>,
    phase_sender: Option<watch::Sender<Vec<ReconciliationPhase>>>,
    phase_log: Option<Arc<Mutex<ReconciliationPhaseLog>>>,
    manifest_snapshot:
        Arc<OnceLock<Result<Arc<resources::ArtifactManifestRefresh>, resources::ResourcesError>>>,
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
        }
    }

    pub(crate) fn disabled() -> Self {
        Self {
            sender: None,
            phase_sender: None,
            phase_log: None,
            manifest_snapshot: Arc::new(OnceLock::new()),
        }
    }

    fn with_phase_log(mut self, phase_log: ReconciliationPhaseLog) -> Self {
        self.phase_log = Some(Arc::new(Mutex::new(phase_log)));
        self
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

    fn operation_finished(&self, event: resources::ResourceOperationEvent<'_, '_>) {
        let Some(phase_log) = &self.phase_log else {
            return;
        };
        let manifest_operation = matches!(event.operation, resources::ResourceOperation::Manifest);
        let (phase, subject, counts) = match event.operation {
            resources::ResourceOperation::Manifest => (
                ReconciliationPhase::Manifest,
                "artifact_manifest".to_owned(),
                vec![("manifest_count", 1)],
            ),
            resources::ResourceOperation::Download(artifact) => (
                ReconciliationPhase::Download,
                artifact_subject(artifact),
                vec![("artifact_count", 1), ("artifact_bytes", artifact.size())],
            ),
            resources::ResourceOperation::Install(artifact) => (
                ReconciliationPhase::Install,
                artifact_subject(artifact),
                vec![("artifact_count", 1)],
            ),
        };
        let (outcome, fields) = match event.outcome {
            resources::ResourceOperationOutcome::Succeeded if manifest_operation => {
                (PhaseOutcome::Succeeded, vec![("manifest_source", "latest")])
            }
            resources::ResourceOperationOutcome::Succeeded => (PhaseOutcome::Succeeded, Vec::new()),
            resources::ResourceOperationOutcome::Failed => (PhaseOutcome::Failed, Vec::new()),
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
) -> Result<(), DaemonError> {
    let parsed_scope = scope.parse::<ReconciliationScope>();
    if kind == "reconcile" {
        return match parsed_scope {
            Ok(parsed_scope) => {
                run_reconciliation_job(paths, queue, transport, parsed_scope, runtime_catalog).await
            }
            Err(error) => {
                run_invalid_reconciliation_scope_job(paths, transport, scope, error).await
            }
        };
    }
    if kind == "update" && scope == "system" {
        return run_update_job(paths, queue, transport, runtime_catalog).await;
    }

    run_started_job(paths, transport, kind, scope).await
}

pub(crate) async fn run_background_reconciliation_job(
    paths: PvPaths,
    queue: ReconciliationQueue,
    scope: ReconciliationScope,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
) -> Result<(), DaemonError> {
    let result = enqueue_reconciliation_job(&paths, &queue, scope)?;

    complete_background_reconciliation_job(&paths, result, runtime_catalog).await
}

pub(crate) async fn run_startup_reconciliation_job(
    paths: PvPaths,
    queue: ReconciliationQueue,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    mut shutdown: oneshot::Receiver<()>,
) -> Result<(), DaemonError> {
    let result = loop {
        let enqueue_paths = paths.clone();
        let enqueue_queue = queue.clone();
        let mut enqueue_task = tokio::task::spawn_blocking(move || {
            enqueue_startup_reconciliation_job(&enqueue_paths, &enqueue_queue)
        });
        let result = tokio::select! {
            result = &mut enqueue_task => result?,
            _ = &mut shutdown => {
                let _enqueue_result = enqueue_task.await?;
                return Ok(());
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
            Err(error) => return Err(error),
        }
    };

    tokio::select! {
        biased;
        _ = &mut shutdown => Ok(()),
        result = complete_background_reconciliation_job(&paths, result, runtime_catalog) => result,
    }
}

async fn complete_background_reconciliation_job(
    paths: &PvPaths,
    result: EnqueueResult,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
) -> Result<(), DaemonError> {
    let EnqueueResult::Queued(queued) = result else {
        return Ok(());
    };
    let running = queued.wait_for_turn().await;
    let job_id = running.job_id().to_string();
    let scope = running.scope().clone();
    let result =
        complete_reconciliation_job(paths, &job_id, &scope, runtime_catalog, running.timing())
            .await
            .map(|_summary| ());

    running.finish();

    result
}

async fn run_reconciliation_job(
    paths: PvPaths,
    queue: ReconciliationQueue,
    mut transport: DaemonTransport<LocalStream>,
    scope: ReconciliationScope,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
) -> Result<(), DaemonError> {
    let result = match enqueue_reconciliation_job(&paths, &queue, scope) {
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
            let (running, stream_is_open) = wait_for_foreground_turn(
                queued,
                &mut transport,
                stream_is_open,
                FOREGROUND_JOB_HEARTBEAT_INTERVAL,
            )
            .await;
            let scope = running.scope().clone();
            let result = stream_started_reconciliation_job(
                paths,
                transport,
                stream_is_open,
                running.job_id(),
                scope,
                runtime_catalog,
                running.timing(),
            )
            .await;

            running.finish();

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

fn enqueue_reconciliation_job(
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
            let (running, stream_is_open) = wait_for_foreground_turn(
                queued,
                &mut transport,
                stream_is_open,
                FOREGROUND_JOB_HEARTBEAT_INTERVAL,
            )
            .await;
            let result = stream_started_update_job(
                paths,
                transport,
                stream_is_open,
                running.job_id(),
                runtime_catalog,
            )
            .await;

            running.finish();

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
) -> Result<(), DaemonError>
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
        let completion = complete_streamed_job_with_heartbeat_and_events(
            &mut transport,
            job_id,
            "Managed Resource update still running",
            FOREGROUND_JOB_HEARTBEAT_INTERVAL,
            complete_update_job_with_progress(&paths, job_id, runtime_catalog, progress),
            event_receiver,
            phase_receiver,
        )
        .await;

        (completion.result, completion.transport_is_open)
    } else {
        (
            complete_update_job(&paths, job_id, runtime_catalog).await,
            false,
        )
    };
    started_stream_result?;

    if !stream_is_open || !transport_is_open {
        return update_result.map(|_summary| ());
    }

    match update_result {
        Ok(summary) => {
            write_foreground_terminal_event(
                &mut transport,
                &DaemonEvent::JobCompleted {
                    job_id,
                    summary: &summary,
                },
            )
            .await?;
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
            .await?;
        }
    }

    Ok(())
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
) -> (RunningReconciliation, bool)
where
    Stream: AsyncWrite + Unpin,
{
    let job_id = queued.job_id().to_string();
    let wait_for_turn = queued.wait_for_turn();
    tokio::pin!(wait_for_turn);
    let mut heartbeat = interval_at(Instant::now() + heartbeat_interval, heartbeat_interval);
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            biased;
            running = &mut wait_for_turn => return (running, stream_is_open),
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

async fn stream_started_reconciliation_job<Stream>(
    paths: PvPaths,
    mut transport: DaemonTransport<Stream>,
    stream_is_open: bool,
    job_id: &str,
    scope: ReconciliationScope,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    timing: ReconciliationJobTiming,
) -> Result<(), DaemonError>
where
    Stream: AsyncWrite + Unpin,
{
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
            let completion = complete_streamed_job_with_heartbeat_and_events(
                &mut transport,
                job_id,
                "Reconciliation still running",
                FOREGROUND_JOB_HEARTBEAT_INTERVAL,
                complete_reconciliation_job_with_progress(
                    &paths,
                    job_id,
                    &scope,
                    runtime_catalog,
                    progress,
                    timing,
                ),
                event_receiver,
                phase_receiver,
            )
            .await;

            (completion.result, completion.transport_is_open)
        } else {
            (
                complete_reconciliation_job(&paths, job_id, &scope, runtime_catalog, timing).await,
                false,
            )
        };
    started_stream_result?;

    if !stream_is_open || !transport_is_open {
        return reconciliation_result.map(|_summary| ());
    }

    match reconciliation_result {
        Ok(summary) => {
            write_foreground_terminal_event(
                &mut transport,
                &DaemonEvent::JobCompleted {
                    job_id,
                    summary: &summary,
                },
            )
            .await?;
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
            .await?;
        }
    }

    Ok(())
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

async fn complete_streamed_job_with_heartbeat_and_events<Stream, Completion>(
    transport: &mut DaemonTransport<Stream>,
    job_id: &str,
    heartbeat_message: &'static str,
    heartbeat_interval: Duration,
    completion: Completion,
    mut events: Receiver<ForegroundJobEvent>,
    mut phases: watch::Receiver<Vec<ReconciliationPhase>>,
) -> StreamedJobCompletion
where
    Stream: AsyncWrite + Unpin,
    Completion: Future<Output = Result<String, DaemonError>>,
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
                    return StreamedJobCompletion {
                        result: completion.await,
                        transport_is_open: false,
                    };
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

async fn finish_streamed_job<Stream>(
    transport: &mut DaemonTransport<Stream>,
    job_id: &str,
    phases: &mut watch::Receiver<Vec<ReconciliationPhase>>,
    next_phase: &mut usize,
    result: Result<String, DaemonError>,
) -> StreamedJobCompletion
where
    Stream: AsyncWrite + Unpin,
{
    let has_pending_phase = phases.borrow().len() > *next_phase;
    let transport_is_open = if has_pending_phase {
        write_pending_phases(transport, job_id, phases, next_phase).await
    } else {
        true
    };

    StreamedJobCompletion {
        result,
        transport_is_open,
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
    )
    .await
}

async fn complete_update_job_with_progress(
    paths: &PvPaths,
    job_id: &str,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    progress: DaemonDownloadProgress,
) -> Result<String, DaemonError> {
    let phase_log = ReconciliationPhaseLog::new(paths, job_id, "update", "system")
        .with_progress(progress.phase_sender.clone());
    let progress = progress.with_phase_log(phase_log.clone());
    let result = complete_update_job_inner(paths, runtime_catalog, progress, &phase_log).await;
    let finalization_timer = phase_log.start(ReconciliationPhase::Finalization, "job");

    match &result {
        Ok(completed) => {
            let mut database = Database::open(paths)?;
            let mut coverage = vec![JobDiagnosticSubject::UpdateAssessment];
            coverage.extend(completed.coverage.iter().cloned());
            database.complete_job_with_coverage(job_id, &completed.summary, &coverage)?;
            structured_log::job_completed(paths, job_id, "update", "system", &completed.summary);
        }
        Err(error) => {
            let error_message = error.error.to_string();
            let mut database = Database::open(paths)?;
            database.fail_job_with_subject(job_id, &error_message, &error.subject)?;
            structured_log::job_failed(paths, job_id, "update", "system", &error_message);
        }
    }
    finalization_timer.finish(PhaseOutcome::from_succeeded(result.is_ok()), &[]);

    result
        .map(|completed| completed.summary)
        .map_err(|failure| *failure.error)
}

async fn complete_update_job_inner(
    paths: &PvPaths,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    progress: DaemonDownloadProgress,
    phase_log: &ReconciliationPhaseLog,
) -> Result<CompletedUpdateJob, FailedUpdateJob> {
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
            return Err(reconcile_partial_update_failure(
                paths,
                runtime_catalog,
                progress,
                phase_log,
                update_error,
            )
            .await);
        }
    };

    if report.updated_count == 0 {
        return Ok(CompletedUpdateJob {
            summary: unchanged_update_summary(&report),
            coverage: Vec::new(),
        });
    }

    let project_report = complete_update_step_with_caddy_compensation(
        paths,
        &report,
        reconcile_system_projects_and_resources_with_progress(
            paths,
            runtime_catalog,
            progress,
            phase_log,
        )
        .await,
        JobDiagnosticSubject::SystemReconciliation,
    )
    .await?;
    let gateway_summary = complete_update_step_with_caddy_compensation(
        paths,
        &report,
        reconcile_gateway_runtimes_with_phase_log(paths, phase_log).await,
        JobDiagnosticSubject::GatewayRuntime,
    )
    .await?;
    let reconciliation_summary = system_reconciliation_summary(&project_report, &gateway_summary);
    let coverage = complete_update_step_with_caddy_compensation(
        paths,
        &report,
        completed_system_reconciliation_coverage(paths, &project_report),
        JobDiagnosticSubject::SystemReconciliation,
    )
    .await?;

    let summary = format!(
        "updated {} artifact(s); reconciled: {reconciliation_summary}",
        report.updated_count
    );

    Ok(CompletedUpdateJob { summary, coverage })
}

async fn complete_update_step_with_caddy_compensation<T>(
    paths: &PvPaths,
    report: &ManagedResourceUpdateReport,
    result: Result<T, DaemonError>,
    subject: JobDiagnosticSubject,
) -> Result<T, FailedUpdateJob> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => Err(compensate_caddy_update_failure(paths, report, error, subject).await),
    }
}

async fn compensate_caddy_update_failure(
    paths: &PvPaths,
    report: &ManagedResourceUpdateReport,
    original_error: DaemonError,
    subject: JobDiagnosticSubject,
) -> FailedUpdateJob {
    match report.rollback_caddy(paths) {
        Ok(true) => match reconcile_gateway_runtimes(paths).await {
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
) -> FailedUpdateJob {
    if let Err(reconciliation_error) = reconcile_system_projects_and_resources_with_progress(
        paths,
        runtime_catalog,
        progress,
        phase_log,
    )
    .await
    {
        return FailedUpdateJob::new(
            partial_update_reconciliation_error(update_error, reconciliation_error),
            JobDiagnosticSubject::UpdateAssessment,
        );
    }
    if let Err(reconciliation_error) =
        reconcile_gateway_runtimes_with_phase_log(paths, phase_log).await
    {
        return FailedUpdateJob::new(
            partial_update_reconciliation_error(update_error, reconciliation_error),
            JobDiagnosticSubject::UpdateAssessment,
        );
    }

    FailedUpdateJob::new(update_error, JobDiagnosticSubject::UpdateAssessment)
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

async fn complete_managed_resource_reconciliation_with_progress(
    paths: &PvPaths,
    name: &crate::reconciliation::ReconciliationScopeComponent,
    track: &crate::reconciliation::ReconciliationScopeComponent,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    progress: DaemonDownloadProgress,
    phase_log: &ReconciliationPhaseLog,
) -> Result<CompletedReconciliationJob, DaemonError> {
    let resource_timer = phase_log.start(
        ReconciliationPhase::Resources,
        format!("{}:{}", name.as_str(), track.as_str()),
    );
    let resource_result = reconcile_persisted_resource_track_with_progress(
        paths,
        name.as_str(),
        track.as_str(),
        runtime_catalog,
        progress,
    )
    .await;
    resource_timer.finish(
        PhaseOutcome::from_succeeded(resource_result.is_ok()),
        &[("resource_count", 1)],
    );
    let (projects, resource_failures) = resource_result?;

    let project_timer = phase_log.start(ReconciliationPhase::ProjectApply, "dependent_projects");
    let project_result = reconcile_persisted_project_envs(paths, &projects, resource_failures);
    finish_project_phase(project_timer, &project_result);
    let project_report = project_result?;
    let summary =
        managed_resource_reconciliation_summary(name.as_str(), track.as_str(), &project_report);
    let mut coverage = vec![JobDiagnosticSubject::Resource {
        name: name.as_str().to_owned(),
        track: track.as_str().to_owned(),
    }];
    coverage.extend(project_report.successful_project_coverage());

    Ok(CompletedReconciliationJob { summary, coverage })
}

fn reconcile_persisted_project_envs(
    paths: &PvPaths,
    projects: &[ProjectRecord],
    mut resource_failures: BTreeMap<String, DaemonError>,
) -> Result<SystemProjectReconciliationReport, DaemonError> {
    let mut report = SystemProjectReconciliationReport {
        total: projects.len(),
        ..SystemProjectReconciliationReport::default()
    };

    for project in projects {
        let project_label = project.primary_hostname.as_deref().unwrap_or(&project.slug);
        if let Some(error) = resource_failures.remove(&project.id) {
            let error_message = error.to_string();
            let mut database = match Database::open(paths) {
                Ok(database) => database,
                Err(recording) => {
                    return Err(DaemonError::ProjectAllocationFailureRecordingFailed {
                        project_id: project.id.clone(),
                        allocation: Box::new(error),
                        recording: Box::new(recording.into()),
                    });
                }
            };
            if let Err(recording) =
                record_project_env_failure(&mut database, &project.id, &error_message)
            {
                return Err(DaemonError::ProjectAllocationFailureRecordingFailed {
                    project_id: project.id.clone(),
                    allocation: Box::new(error),
                    recording: Box::new(recording),
                });
            }
            report
                .failures
                .push(format!("{project_label}: {error_message}"));
            continue;
        }
        match reconcile_project_env_from_persisted_state(paths, &project.id) {
            Ok(summary) => {
                report.succeeded += 1;
                report.successful_project_ids.push(project.id.clone());
                report.summaries.push(summary.to_owned());
            }
            Err(error @ DaemonError::ProjectEnvFailureRecordingFailed { .. }) => {
                return Err(error);
            }
            Err(error) => {
                report.failures.push(format!("{project_label}: {error}"));
            }
        }
    }

    Ok(report)
}

async fn complete_reconciliation_job(
    paths: &PvPaths,
    job_id: &str,
    scope: &ReconciliationScope,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    timing: ReconciliationJobTiming,
) -> Result<String, DaemonError> {
    complete_reconciliation_job_with_progress(
        paths,
        job_id,
        scope,
        runtime_catalog,
        DaemonDownloadProgress::disabled(),
        timing,
    )
    .await
}

async fn complete_reconciliation_job_with_progress(
    paths: &PvPaths,
    job_id: &str,
    scope: &ReconciliationScope,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    progress: DaemonDownloadProgress,
    timing: ReconciliationJobTiming,
) -> Result<String, DaemonError> {
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
    let progress = progress.with_phase_log(phase_log.clone());
    let effective_scope = effective_reconciliation_scope(scope);
    let result = match &effective_scope {
        ReconciliationScope::System => {
            complete_system_reconciliation_with_progress(
                paths,
                runtime_catalog,
                progress,
                &phase_log,
            )
            .await
        }
        ReconciliationScope::Resource { name, .. } if gateway_runtime_resource(name.as_str()) => {
            complete_gateway_reconciliation(paths, &phase_log).await
        }
        ReconciliationScope::Resource { name, track } => {
            complete_managed_resource_reconciliation_with_progress(
                paths,
                name,
                track,
                runtime_catalog,
                progress,
                &phase_log,
            )
            .await
        }
        ReconciliationScope::Project { id } => {
            complete_project_reconciliation_with_progress(
                paths,
                id,
                runtime_catalog,
                progress,
                &phase_log,
            )
            .await
        }
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

                    Ok(completed.summary)
                }
                Err(error) => {
                    fail_reconciliation_job(paths, job_id, &scope_text, &error)?;

                    Err(error)
                }
            }
        }
        Err(error) => {
            fail_reconciliation_job(paths, job_id, &scope_text, &error)?;

            Err(error)
        }
    };
    finalization_timer.finish(
        PhaseOutcome::from_succeeded(final_result.is_ok()),
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

fn effective_reconciliation_scope(scope: &ReconciliationScope) -> ReconciliationScope {
    match scope {
        ReconciliationScope::Resource { name, .. }
            if matches!(name.as_str(), "php" | "frankenphp") =>
        {
            ReconciliationScope::System
        }
        scope => scope.clone(),
    }
}

fn fail_reconciliation_job(
    paths: &PvPaths,
    job_id: &str,
    scope: &str,
    error: &DaemonError,
) -> Result<(), DaemonError> {
    let error_message = error.to_string();
    let mut database = Database::open(paths)?;
    database.fail_job(job_id, &error_message)?;
    structured_log::job_failed(paths, job_id, "reconcile", scope, &error_message);

    Ok(())
}

async fn complete_gateway_reconciliation(
    paths: &PvPaths,
    phase_log: &ReconciliationPhaseLog,
) -> Result<CompletedReconciliationJob, DaemonError> {
    let summary = reconcile_gateway_runtimes_with_phase_log(paths, phase_log).await?;

    Ok(CompletedReconciliationJob {
        summary,
        coverage: vec![JobDiagnosticSubject::GatewayRuntime],
    })
}

async fn complete_system_reconciliation_with_progress(
    paths: &PvPaths,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    progress: DaemonDownloadProgress,
    phase_log: &ReconciliationPhaseLog,
) -> Result<CompletedReconciliationJob, DaemonError> {
    let project_report = reconcile_system_projects_and_resources_with_progress(
        paths,
        runtime_catalog,
        progress,
        phase_log,
    )
    .await?;
    let gateway_summary = reconcile_gateway_runtimes_with_phase_log(paths, phase_log).await?;
    let summary = system_reconciliation_summary(&project_report, &gateway_summary);
    let coverage = completed_system_reconciliation_coverage(paths, &project_report)?;

    Ok(CompletedReconciliationJob { summary, coverage })
}

async fn complete_project_reconciliation_with_progress(
    paths: &PvPaths,
    id: &crate::reconciliation::ReconciliationScopeComponent,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    progress: DaemonDownloadProgress,
    phase_log: &ReconciliationPhaseLog,
) -> Result<CompletedReconciliationJob, DaemonError> {
    let project_timer = phase_log.start(ReconciliationPhase::ProjectApply, id.as_str());
    let project_result = reconcile_project_env_and_missing_resources_with_progress(
        paths,
        id.as_str(),
        runtime_catalog,
        progress.clone(),
    )
    .await;
    project_timer.finish(
        PhaseOutcome::from_succeeded(project_result.is_ok()),
        &[("project_count", 1)],
    );
    let project_env_summary = project_result?;
    let gateway_outcome =
        reconcile_project_gateway_runtimes_with_phase_log(paths, id.as_str(), phase_log).await?;
    let (gateway_summary, gateway_evaluated) = match gateway_outcome {
        ProjectGatewayReconciliationOutcome::Reconciled {
            summary,
            gateway_evaluated,
        } => (summary, gateway_evaluated),
        ProjectGatewayReconciliationOutcome::PromoteSystem => {
            return complete_system_reconciliation_with_progress(
                paths,
                runtime_catalog,
                progress,
                phase_log,
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

    Ok(CompletedReconciliationJob { summary, coverage })
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
    reconcile_project_env_and_missing_resources_with_progress(
        paths,
        project_id,
        runtime_catalog,
        DaemonDownloadProgress::disabled(),
    )
    .await
}

async fn reconcile_project_env_and_missing_resources_with_progress(
    paths: &PvPaths,
    project_id: &str,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    progress: DaemonDownloadProgress,
) -> Result<crate::project_env::ProjectEnvReconciliationSummary, DaemonError> {
    let summary = reconcile_project_env_with_runtime_catalog_and_progress(
        paths,
        project_id,
        runtime_catalog,
        None,
        &BTreeSet::new(),
        progress.clone(),
    )
    .await?;
    if !summary.requested_php_extensions() || !missing_gateway_runtime_resource(paths)? {
        return Ok(summary);
    }

    reconcile_system_resources_with_runtime_catalog_and_progress(
        paths,
        runtime_catalog,
        &BTreeSet::new(),
        progress.clone(),
    )
    .await?;
    reconcile_project_env_with_runtime_catalog_and_progress(
        paths,
        project_id,
        runtime_catalog,
        None,
        &BTreeSet::new(),
        progress,
    )
    .await
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct SystemProjectReconciliationReport {
    total: usize,
    succeeded: usize,
    successful_project_ids: Vec<String>,
    summaries: Vec<String>,
    failures: Vec<String>,
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

async fn reconcile_system_projects_with_progress(
    paths: &PvPaths,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    demanded_tracks: &BTreeSet<DemandedResourceTrack>,
    project_demands: &BTreeMap<String, ProjectDemand>,
    progress: &DaemonDownloadProgress,
) -> Result<SystemProjectReconciliationReport, DaemonError> {
    let projects = linked_projects(paths)?;
    let mut report = SystemProjectReconciliationReport {
        total: projects.len(),
        ..SystemProjectReconciliationReport::default()
    };

    for project in projects {
        match reconcile_project_env_with_runtime_catalog_and_progress(
            paths,
            &project.id,
            runtime_catalog,
            project_demands.get(&project.id),
            demanded_tracks,
            progress.clone(),
        )
        .await
        {
            Ok(summary) => {
                report.succeeded += 1;
                report.successful_project_ids.push(project.id);
                report.summaries.push(summary.as_str().to_owned());
            }
            Err(error) => {
                let project_label = project.primary_hostname.as_deref().unwrap_or(&project.slug);
                report.failures.push(format!("{project_label}: {error}"));
            }
        }
    }

    Ok(report)
}

async fn reconcile_system_projects_and_resources_with_progress(
    paths: &PvPaths,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    progress: DaemonDownloadProgress,
    phase_log: &ReconciliationPhaseLog,
) -> Result<SystemProjectReconciliationReport, DaemonError> {
    let discovery_timer = phase_log.start(ReconciliationPhase::DemandDiscovery, "linked_projects");
    let discovery_result = discover_system_project_demand(paths);
    finish_demand_discovery_phase(discovery_timer, &discovery_result);
    let demand = discovery_result?;

    let resources_timer = phase_log.start(ReconciliationPhase::Resources, "desired_resources");
    let resources_result = reconcile_system_resources_with_runtime_catalog_and_progress(
        paths,
        runtime_catalog,
        &demand.resource_tracks,
        progress.clone(),
    )
    .await;
    resources_timer.finish(PhaseOutcome::from_succeeded(resources_result.is_ok()), &[]);
    resources_result?;

    let project_timer = phase_log.start(ReconciliationPhase::ProjectApply, "linked_projects");
    let project_result = reconcile_system_projects_with_progress(
        paths,
        runtime_catalog,
        &demand.resource_tracks,
        &demand.project_demands,
        &progress,
    )
    .await;
    finish_project_phase(project_timer, &project_result);
    let report = project_result?;
    stop_undemanded_system_resource_runtimes(paths, runtime_catalog).await?;

    Ok(report)
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

fn linked_projects(paths: &PvPaths) -> Result<Vec<ProjectRecord>, DaemonError> {
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
            report.failures.join(", ")
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
    let effective_scope = effective_reconciliation_scope(scope);
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
    use std::net::TcpListener;
    use std::os::unix::fs::PermissionsExt;
    use std::pin::Pin;
    use std::process;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll};
    use std::time::Instant;

    use camino::{Utf8Path, Utf8PathBuf};
    use camino_tempfile::tempdir;
    use futures_util::StreamExt;
    use insta::{Settings, assert_debug_snapshot};
    use rcgen::generate_simple_self_signed;
    use rusqlite::Connection;
    use serde_json::json;
    use state::{
        Database, GatewayPort, JobDiagnosticSubject, JobStatus, JobsLock, LinkProjectInput,
        ManagedResourceDesiredState, PortRequest, ProjectEnvObservedStatus,
        ProjectManagedResourceInput, ProjectMode, ProjectPhpRuntimeInput, PvPaths,
        RuntimeObservedStatus, RuntimeSubject, StateError, UpdateLock,
    };
    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf, duplex};
    #[cfg(unix)]
    use tokio::net::UnixStream;
    use tokio::sync::{mpsc::channel, oneshot, watch};
    use tokio::time::{Duration, timeout};

    use crate::project_env::reconcile_project_env_from_persisted_state;

    use super::{
        FOREGROUND_JOB_PROGRESS_BUFFER, FOREGROUND_JOB_STREAM_WRITE_TIMEOUT, ForegroundJobEvent,
        SystemProjectReconciliationReport, abandon_reconciliation_job,
        complete_managed_resource_reconciliation_with_progress,
        complete_or_fail_background_reconciliation, complete_project_reconciliation_with_progress,
        complete_streamed_job_with_heartbeat, complete_streamed_job_with_heartbeat_and_events,
        complete_system_reconciliation_with_progress, complete_update_job,
        completed_system_reconciliation_coverage, discover_system_project_demand,
        effective_reconciliation_scope, enqueue_reconciliation_job, enqueue_update_job,
        foreground_reconciliation_result, managed_resource_reconciliation_summary,
        reconcile_persisted_project_envs, reconcile_project_env_and_missing_resources,
        reconcile_system_projects_with_progress,
        reconcile_system_resources_with_runtime_catalog_and_progress,
        record_background_reconciliation_error, run_background_reconciliation_job,
        run_reconciliation_job, start_reconciliation_job, start_update_job,
        stop_undemanded_system_resource_runtimes, stream_started_reconciliation_job,
        stream_started_update_job, wait_for_foreground_turn, write_coalesced_update_response,
    };
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
    const FAKE_CADDY_SCRIPT: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/test-fixtures/gateway/fake-caddy.sh"
    ));
    const FAKE_CADDY_SERVER_SCRIPT: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/test-fixtures/gateway/fake-caddy-server.py"
    ));

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
        seed_installed_caddy(&paths)?;
        let _caddy_guard = SeededCaddyGuard::new(paths.clone());
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
        )
        .await?;

        assert_eq!(
            completed.coverage,
            [JobDiagnosticSubject::Project { id: project.id }]
        );
        assert!(!paths.gateway_pid().exists());
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
        let _caddy_guard = SeededCaddyGuard::new(paths.clone());
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
        let partial_snapshot = (
            partial.summary,
            partial.coverage,
            partial_statuses,
            persisted_beta_demand,
            state::fs::read_to_string(&projects[0].path.join(".env"))?,
            state::fs::path_entry_exists(&projects[1].path.join(".env"))?,
            state::fs::path_entry_exists(&projects[2].path.join(".env"))?,
        );

        state::fs::write_sensitive_file(&projects[1].config_path, project_config)?;
        reconcile_project_env_from_persisted_state(&paths, &projects[1].id)?;
        assert_eq!(
            Database::open(&paths)?
                .project_env_observed_state(&projects[1].id)?
                .map(|observed| observed.status),
            Some(ProjectEnvObservedStatus::Rendered)
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
                Some(ProjectEnvObservedStatus::Rendered),
                None,
            ]
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
            .map(|scope| (scope.clone(), effective_reconciliation_scope(scope)))
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
            failures: vec![format!("failed.test: {}", failed.project.id)],
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
    async fn system_reconciliation_pins_discovered_php_track_across_manifest_refresh()
    -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let project_path = tempdir.path().join("project");
        let config_path = project_path.join("pv.yml");

        state::fs::write_sensitive_file(
            &config_path,
            "serve: false\nphp:\n  version: latest\n  extensions: [redis]\nenv:\n  APP_NAME: project\n",
        )?;
        seed_cached_php_pair(&paths, tempdir.path())?;
        let cached_manifest_path = paths.downloads().join("manifest.json");
        let cached_manifest = state::fs::read_to_string(&cached_manifest_path)?;
        let mut refreshed_manifest = serde_json::from_str::<serde_json::Value>(&cached_manifest)?;
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
        let _caddy_guard = SeededCaddyGuard::new(paths.clone());
        let mut database = Database::open(&paths)?;
        database.link_project(LinkProjectInput {
            path: project_path.clone(),
            original_path: project_path,
            primary_hostname: "project.test".to_owned(),
            config_path,
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
            MultiArtifactClient {
                manifest: refreshed_manifest.clone(),
                archives: BTreeMap::new(),
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
        )
        .await?;

        let database = Database::open(&paths)?;
        let project = database
            .projects()?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("expected linked project"))?;

        assert_eq!(project.php_runtime.track.as_deref(), Some(PHP_TEST_TRACK));
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
        let observed_writes =
            write_counter.query_row("SELECT count FROM test_project_env_writes", [], |row| {
                row.get::<_, i64>(0)
            })?;
        assert_eq!(observed_writes, 1);

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
            assert_eq!(
                demand.project_demands.get(project_id),
                Some(&super::ProjectDemand {
                    resource_tracks: demand.resource_tracks.clone(),
                    php_track: None,
                    used_persisted_state: false,
                })
            );
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
        state::fs::write_sensitive_file(
            &config_path,
            "serve: false\nphp:\n  version: \"8.5\"\n  extensions: [redis]\nenv:\n  APP_NAME: applied\n",
        )?;
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
        )
        .await?;
        stop_undemanded_system_resource_runtimes(&paths, Some(&catalog)).await?;

        assert_eq!(report.succeeded, 1);
        assert!(report.failures.is_empty());
        let env = state::fs::read_to_string(&project_path.join(".env"))?;
        assert!(env.contains("APP_NAME=applied"));
        assert!(!env.contains("APP_NAME=discovered"));
        let database = Database::open(&paths)?;
        let project = database
            .project_by_id(&linked.project.id)?
            .ok_or_else(|| anyhow::anyhow!("expected linked project"))?;
        assert_eq!(project.mode, ProjectMode::ResourceOnly);
        assert_eq!(project.php_runtime.track.as_deref(), Some(PHP_TEST_TRACK));
        assert_eq!(project.php_runtime.loaded_extensions, ["redis"]);
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
    async fn stream_write_error_is_returned_after_job_completion_is_persisted() -> anyhow::Result<()>
    {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        seed_installed_caddy(&paths)?;
        let _caddy_guard = SeededCaddyGuard::new(paths.clone());
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
        let _caddy_guard = SeededCaddyGuard::new(paths.clone());
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
        let _caddy_guard = SeededCaddyGuard::new(paths.clone());
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
        let phases = reconciliation_phase_events(&paths, &job_id)?;
        for phase in ["manifest", "download", "install"] {
            let event = phases
                .iter()
                .find(|event| event["phase"] == phase)
                .ok_or_else(|| anyhow::anyhow!("missing {phase} phase event"))?;
            assert!(event["elapsed_ms"].as_u64().is_some());
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
                "finalization",
            ]
        );

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
        let phases = reconciliation_phase_events(&paths, &job_id)?;
        for (phase, expected_count) in [("manifest", 1), ("resources", 1), ("finalization", 1)] {
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
                "manifest",
                "download",
                "install",
                "project_apply",
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
        assert!(!completion.transport_is_open);

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
        assert!(!completion.transport_is_open);

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
        let queued_paths = paths.clone();
        let queued_queue = queue.clone();
        let queued_scope = ReconciliationScope::project("project_1")?;
        let queued_task = tokio::spawn(async move {
            run_background_reconciliation_job(queued_paths, queued_queue, queued_scope, None).await
        });

        wait_for_job_scope(&paths, "project:project_1").await?;
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
            wait_for_foreground_turn(waiting, &mut transport, true, Duration::from_millis(5)).await
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
        let (waiting_running, stream_is_open) = task.await?;
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
            wait_for_foreground_turn(waiting, &mut transport, true, Duration::from_millis(5)).await
        });

        timeout(Duration::from_millis(100), failed_write_receiver)
            .await?
            .map_err(|_error| anyhow::anyhow!("queued heartbeat writer dropped"))?;
        running.finish();
        let (waiting_running, stream_is_open) = task.await?;

        assert!(!stream_is_open);
        assert_eq!(waiting_running.job_id(), waiting_job_id);
        waiting_running.finish();

        Ok(())
    }

    #[cfg(unix)]
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
        let task = tokio::spawn(async move {
            run_reconciliation_job(
                task_paths,
                task_queue,
                protocol::transport(server),
                scope,
                None,
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
        let scope = ReconciliationScope::project("project_1")?;
        let queued_paths = paths.clone();
        let queued_queue = queue.clone();
        let queued_scope = scope.clone();
        let queued_task = tokio::spawn(async move {
            run_background_reconciliation_job(queued_paths, queued_queue, queued_scope, None).await
        });

        wait_for_job_scope(&paths, "project:project_1").await?;
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
        Ok(state::fs::read_to_string(&paths.daemon_log())?
            .lines()
            .map(serde_json::from_str::<serde_json::Value>)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|event| {
                event["event"] == "reconciliation_phase_completed" && event["job_id"] == job_id
            })
            .collect())
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
        )
        .await?;

        let mut reader = protocol::transport(client);
        let mut events = Vec::new();
        while let Some(line) = reader.next().await {
            events.push(serde_json::from_str::<serde_json::Value>(&line?)?);
        }

        Ok(events)
    }

    fn seed_cached_php_pair(paths: &PvPaths, tempdir: &Utf8Path) -> anyhow::Result<()> {
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

    struct SeededCaddyGuard {
        paths: PvPaths,
    }

    impl SeededCaddyGuard {
        fn new(paths: PvPaths) -> Self {
            Self { paths }
        }
    }

    impl Drop for SeededCaddyGuard {
        fn drop(&mut self) {
            let paths = self.paths.clone();
            std::thread::scope(|scope| {
                let cleanup_thread = scope.spawn(move || {
                    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    else {
                        return;
                    };

                    let _cleanup_result = runtime.block_on(stop_seeded_caddy(&paths));
                });
                let _join_result = cleanup_thread.join();
            });
        }
    }

    async fn stop_seeded_caddy(paths: &PvPaths) -> anyhow::Result<()> {
        let supervisor = ProcessSupervisor::new(paths.clone());
        if let Some(caddy) =
            supervisor.adopt_recorded(&paths.gateway_pid(), &paths.gateway_runtime_metadata())?
        {
            caddy.stop(Duration::from_secs(1)).await?;
        }

        Ok(())
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
        let archive_parent = tempdir.join("php-archive");
        let root_name = format!("php-{PHP_TEST_ARTIFACT_VERSION}");
        let root = archive_parent.join(&root_name);
        let executable = root.join("bin/php");

        state::fs::write_sensitive_file(&executable, "#!/bin/sh\nexit 0\n")?;
        set_executable(&executable)?;
        state::fs::write_sensitive_file(
            &root.join("share/pv/php-extensions.json"),
            r#"[{"name":"redis","load_kind":"extension","path":"lib/php/extensions/redis.so"}]"#,
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
    struct DelayedScriptedArtifactClient {
        inner: ScriptedArtifactClient,
        delay: Duration,
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
