use std::future::Future;

use crate::DaemonError;
use crate::gateway::{FRANKENPHP_NOT_INSTALLED, reconcile_gateway_runtimes};
use crate::ipc::LocalStream;
use crate::managed_resources::{
    ManagedResourceRuntimeCatalog, ManagedResourceUpdateReport, reconcile_system_resources,
    reconcile_system_resources_with_catalog,
};
use crate::project_env::{reconcile_project_env, reconcile_project_env_with_runtime_catalog};
use crate::reconciliation::{EnqueueResult, ReconciliationQueue, ReconciliationScope};
use crate::structured_log;
use protocol::{DaemonEvent, DaemonResponse, DaemonTransport, write_line};
use state::{Database, JobStatus, ProjectRecord, PvPaths, StateError};
use tokio::io::AsyncWrite;
use tokio::time::{Duration, Instant, MissedTickBehavior, interval_at, timeout};

const FOREGROUND_JOB_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
const FOREGROUND_JOB_HEARTBEAT_WRITE_TIMEOUT: Duration = Duration::from_millis(100);

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
    let EnqueueResult::Queued(queued) = result else {
        return Ok(());
    };
    let running = queued.wait_for_turn().await;
    let job_id = running.job_id().to_string();
    let scope = running.scope().clone();
    let result = complete_reconciliation_job(&paths, &job_id, &scope, runtime_catalog)
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
        Err(DaemonError::State(error @ StateError::UpdateInProgress { .. })) => {
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
            let running = queued.wait_for_turn().await;
            let scope = running.scope().clone();
            let result = stream_started_reconciliation_job(
                paths,
                transport,
                stream_is_open,
                running.job_id(),
                scope,
                runtime_catalog,
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

async fn run_update_job(
    paths: PvPaths,
    queue: ReconciliationQueue,
    mut transport: DaemonTransport<LocalStream>,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
) -> Result<(), DaemonError> {
    let result = match enqueue_update_job(&paths, &queue) {
        Ok(result) => result,
        Err(DaemonError::State(error @ StateError::UpdateInProgress { .. })) => {
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
            let running = queued.wait_for_turn().await;
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

    let update_result = if stream_is_open && started_stream_result.is_ok() {
        complete_streamed_job_with_heartbeat(
            &mut transport,
            job_id,
            "Managed Resource update still running",
            FOREGROUND_JOB_HEARTBEAT_INTERVAL,
            complete_update_job(&paths, job_id, runtime_catalog),
        )
        .await
    } else {
        complete_update_job(&paths, job_id, runtime_catalog).await
    };
    started_stream_result?;

    if !stream_is_open {
        return update_result.map(|_summary| ());
    }

    match update_result {
        Ok(summary) => {
            write_line(
                &mut transport,
                &DaemonEvent::Progress {
                    job_id,
                    message: &summary,
                },
            )
            .await?;
            write_line(
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
            write_line(
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

async fn stream_started_reconciliation_job<Stream>(
    paths: PvPaths,
    mut transport: DaemonTransport<Stream>,
    stream_is_open: bool,
    job_id: &str,
    scope: ReconciliationScope,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
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

    let reconciliation_result = if stream_is_open && started_stream_result.is_ok() {
        complete_streamed_job_with_heartbeat(
            &mut transport,
            job_id,
            "Reconciliation still running",
            FOREGROUND_JOB_HEARTBEAT_INTERVAL,
            complete_reconciliation_job(&paths, job_id, &scope, runtime_catalog),
        )
        .await
    } else {
        complete_reconciliation_job(&paths, job_id, &scope, runtime_catalog).await
    };
    started_stream_result?;

    if !stream_is_open {
        return reconciliation_result.map(|_summary| ());
    }

    match reconciliation_result {
        Ok(summary) => {
            let progress = reconciliation_progress_message(&scope, &summary);
            write_line(
                &mut transport,
                &DaemonEvent::Progress {
                    job_id,
                    message: &progress,
                },
            )
            .await?;
            write_line(
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
            write_line(
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
                    _ = timeout(FOREGROUND_JOB_HEARTBEAT_WRITE_TIMEOUT, heartbeat_result) => {}
                }
            }
        }
    }
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
    let mut database = Database::open(paths)?;
    database.fail_job(job_id, "reconciliation was abandoned before completion")?;

    Ok(())
}

fn abandon_update_job(paths: &PvPaths, job_id: &str) -> Result<(), DaemonError> {
    let mut database = Database::open(paths)?;
    database.fail_job(
        job_id,
        "Managed Resource update was abandoned before completion",
    )?;

    Ok(())
}

async fn complete_update_job(
    paths: &PvPaths,
    job_id: &str,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
) -> Result<String, DaemonError> {
    let result = complete_update_job_inner(paths, runtime_catalog).await;

    match &result {
        Ok(summary) => {
            let mut database = Database::open(paths)?;
            database.complete_job(job_id, summary)?;
            structured_log::job_completed(paths, job_id, "update", "system", summary);
        }
        Err(error) => {
            let error_message = error.to_string();
            let mut database = Database::open(paths)?;
            database.fail_job(job_id, &error_message)?;
            structured_log::job_failed(paths, job_id, "update", "system", &error_message);
        }
    }

    result
}

async fn complete_update_job_inner(
    paths: &PvPaths,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
) -> Result<String, DaemonError> {
    let report = crate::managed_resources::update_installed(paths.clone(), runtime_catalog)?;
    if report.updated_count == 0 {
        return Ok(unchanged_update_summary(&report));
    }

    let project_report = reconcile_system_projects_and_resources(paths, runtime_catalog).await?;
    let gateway_summary = reconcile_gateway_runtimes(paths).await?;
    let reconciliation_summary = system_reconciliation_summary(&project_report, &gateway_summary);

    Ok(format!(
        "updated {} artifact(s); reconciled: {reconciliation_summary}",
        report.updated_count
    ))
}

fn unchanged_update_summary(report: &ManagedResourceUpdateReport) -> String {
    if report.installed_count == 0 {
        "none installed".to_string()
    } else {
        "current".to_string()
    }
}

async fn complete_managed_resource_reconciliation(
    paths: &PvPaths,
    job_id: &str,
    name: &crate::reconciliation::ReconciliationScopeComponent,
    track: &crate::reconciliation::ReconciliationScopeComponent,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
) -> Result<String, DaemonError> {
    let project_report = reconcile_system_projects(paths, runtime_catalog).await?;
    let summary =
        managed_resource_reconciliation_summary(name.as_str(), track.as_str(), &project_report);
    let mut database = Database::open(paths)?;

    database.complete_job(job_id, &summary)?;

    Ok(summary)
}

async fn complete_reconciliation_job(
    paths: &PvPaths,
    job_id: &str,
    scope: &ReconciliationScope,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
) -> Result<String, DaemonError> {
    let result = match scope {
        ReconciliationScope::System => {
            complete_system_reconciliation(paths, job_id, runtime_catalog).await
        }
        ReconciliationScope::Resource { name, .. } if gateway_runtime_resource(name.as_str()) => {
            complete_gateway_reconciliation(paths, job_id).await
        }
        ReconciliationScope::Resource { name, track } => {
            complete_managed_resource_reconciliation(paths, job_id, name, track, runtime_catalog)
                .await
        }
        ReconciliationScope::Project { id } => {
            complete_project_reconciliation(paths, job_id, id, runtime_catalog).await
        }
    };

    if let Err(error) = &result {
        let error_message = error.to_string();
        let mut database = Database::open(paths)?;
        database.fail_job(job_id, &error_message)?;
        structured_log::job_failed(
            paths,
            job_id,
            "reconcile",
            &scope.to_string(),
            &error_message,
        );
    } else if let Ok(summary) = &result {
        structured_log::job_completed(paths, job_id, "reconcile", &scope.to_string(), summary);
    }

    result
}

async fn complete_gateway_reconciliation(
    paths: &PvPaths,
    job_id: &str,
) -> Result<String, DaemonError> {
    let summary = reconcile_gateway_runtimes(paths).await?;
    let mut database = Database::open(paths)?;

    database.complete_job(job_id, &summary)?;

    Ok(summary)
}

async fn complete_system_reconciliation(
    paths: &PvPaths,
    job_id: &str,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
) -> Result<String, DaemonError> {
    let project_report = reconcile_system_projects_and_resources(paths, runtime_catalog).await?;
    let gateway_summary = reconcile_gateway_runtimes(paths).await?;
    let summary = system_reconciliation_summary(&project_report, &gateway_summary);
    let mut database = Database::open(paths)?;

    database.complete_job(job_id, &summary)?;

    Ok(summary)
}

async fn complete_project_reconciliation(
    paths: &PvPaths,
    job_id: &str,
    id: &crate::reconciliation::ReconciliationScopeComponent,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
) -> Result<String, DaemonError> {
    let project_env_summary =
        reconcile_project_env_for_runtime_catalog(paths, id.as_str(), runtime_catalog).await?;
    let gateway_summary = reconcile_gateway_runtimes(paths).await?;
    let summary = if gateway_summary == FRANKENPHP_NOT_INSTALLED {
        project_env_summary.as_str().to_string()
    } else {
        format!("{}; {gateway_summary}", project_env_summary.as_str())
    };
    let mut database = Database::open(paths)?;

    database.complete_job(job_id, &summary)?;

    Ok(summary)
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct SystemProjectReconciliationReport {
    total: usize,
    succeeded: usize,
    summaries: Vec<String>,
    failures: Vec<String>,
}

async fn reconcile_system_projects(
    paths: &PvPaths,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
) -> Result<SystemProjectReconciliationReport, DaemonError> {
    let projects = linked_projects(paths)?;
    let mut report = SystemProjectReconciliationReport {
        total: projects.len(),
        ..SystemProjectReconciliationReport::default()
    };

    for project in projects {
        match reconcile_project_env_for_runtime_catalog(paths, &project.id, runtime_catalog).await {
            Ok(summary) => {
                report.succeeded += 1;
                report.summaries.push(summary.as_str().to_owned());
            }
            Err(error) => {
                report
                    .failures
                    .push(format!("{}: {error}", project.primary_hostname));
            }
        }
    }

    Ok(report)
}

async fn reconcile_system_projects_and_resources(
    paths: &PvPaths,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
) -> Result<SystemProjectReconciliationReport, DaemonError> {
    reconcile_system_projects(paths, runtime_catalog).await?;
    reconcile_system_resources_with_runtime_catalog(paths, runtime_catalog).await?;
    reconcile_system_projects(paths, runtime_catalog).await
}

async fn reconcile_project_env_for_runtime_catalog(
    paths: &PvPaths,
    project_id: &str,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
) -> Result<crate::project_env::ProjectEnvReconciliationSummary, DaemonError> {
    if let Some(catalog) = runtime_catalog {
        return reconcile_project_env_with_runtime_catalog(paths, project_id, Some(catalog)).await;
    }

    reconcile_project_env(paths, project_id).await
}

async fn reconcile_system_resources_with_runtime_catalog(
    paths: &PvPaths,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
) -> Result<(), DaemonError> {
    if let Some(catalog) = runtime_catalog {
        let mut database = Database::open(paths)?;

        return reconcile_system_resources_with_catalog(paths, &mut database, catalog).await;
    }

    reconcile_system_resources(paths).await
}

fn linked_projects(paths: &PvPaths) -> Result<Vec<ProjectRecord>, DaemonError> {
    let database = Database::open(paths)?;

    Ok(database.projects()?)
}

fn system_reconciliation_summary(
    project_report: &SystemProjectReconciliationReport,
    gateway_summary: &str,
) -> String {
    let Some(project_summary) = system_project_summary(project_report) else {
        return gateway_summary.to_owned();
    };

    if gateway_summary == FRANKENPHP_NOT_INSTALLED {
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
        return format!(
            "Managed Resource {resource_name} track {track} standalone reconciliation deferred"
        );
    };

    format!(
        "Managed Resource {resource_name} track {track} standalone reconciliation deferred; {project_summary}"
    )
}

fn reconciliation_progress_message(scope: &ReconciliationScope, summary: &str) -> String {
    match scope {
        ReconciliationScope::System
        | ReconciliationScope::Resource { .. }
        | ReconciliationScope::Project { .. } => summary.to_string(),
    }
}

fn reconciliation_started_message(scope: &ReconciliationScope) -> &'static str {
    match scope {
        ReconciliationScope::Project { .. } => "Project env reconciliation started",
        ReconciliationScope::System => "System reconciliation started",
        ReconciliationScope::Resource { name, .. } if gateway_runtime_resource(name.as_str()) => {
            "Gateway runtime reconciliation started"
        }
        ReconciliationScope::Resource { .. } => "Managed Resource runtime reconciliation started",
    }
}

fn gateway_runtime_resource(resource_name: &str) -> bool {
    matches!(resource_name, "php" | "frankenphp")
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
    let already_recorded = database.recent_jobs()?.into_iter().any(|job| {
        job.kind == "reconcile"
            && job.scope == scope
            && job.status == JobStatus::Failed
            && job.error.as_deref() == Some(error_message.as_str())
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
    use std::io;
    use std::os::unix::fs::PermissionsExt;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use camino::Utf8Path;
    use camino_tempfile::tempdir;
    use futures_util::StreamExt;
    use serde_json::json;
    use state::{Database, JobStatus, LinkProjectInput, PvPaths, StateError, UpdateLock};
    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf, duplex};
    use tokio::sync::oneshot;
    use tokio::time::{Duration, timeout};

    use super::{
        complete_or_fail_background_reconciliation, complete_streamed_job_with_heartbeat,
        enqueue_reconciliation_job, foreground_reconciliation_result,
        reconcile_system_projects_and_resources, record_background_reconciliation_error,
        run_background_reconciliation_job, start_reconciliation_job,
        stream_started_reconciliation_job, write_coalesced_update_response,
    };
    use crate::reconciliation::{EnqueueResult, ReconciliationQueue, ReconciliationScope};

    const OFFLINE_TEST_MANIFEST_URL: &str = "https://127.0.0.1:9/manifest.json";
    const PHP_TEST_TRACK: &str = "8.5";
    const PHP_TEST_ARTIFACT_VERSION: &str = "8.5.0-pv1";
    const PHP_TEST_ARCHIVE_FILE_NAME: &str = "php-8.5.0-pv1-any.tar.gz";
    const FRANKENPHP_TEST_ARCHIVE_FILE_NAME: &str = "frankenphp-8.5.0-pv1-any.tar.gz";

    #[tokio::test]
    async fn system_reconciliation_refreshes_php_extensions_after_missing_php_install()
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
        database.link_project(LinkProjectInput {
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
            );

        reconcile_system_projects_and_resources(&paths, Some(&catalog)).await?;

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

        Ok(())
    }

    #[tokio::test]
    async fn stream_write_error_is_returned_after_job_completion_is_persisted() -> anyhow::Result<()>
    {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
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
        )
        .await;

        assert!(result.is_err());
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
    async fn streamed_job_completion_wins_when_heartbeat_write_blocks() -> anyhow::Result<()> {
        let mut writer = protocol::transport(PendingStream);
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

        tokio::time::sleep(Duration::from_millis(20)).await;
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

    #[tokio::test]
    async fn background_reconciliation_rejects_update_lock_without_job() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let update_lock = UpdateLock::acquire(&paths)?;
        let result = run_background_reconciliation_job(
            paths.clone(),
            ReconciliationQueue::new(),
            ReconciliationScope::System,
            None,
        )
        .await;

        assert!(matches!(
            result,
            Err(crate::DaemonError::State(StateError::UpdateInProgress { path }))
                if path == paths.update_lock()
        ));
        drop(update_lock);

        let database = Database::open(&paths)?;
        assert!(database.recent_jobs()?.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn queued_background_reconciliation_reserves_update_lock() -> anyhow::Result<()> {
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
        let update_lock = UpdateLock::acquire(&paths);

        assert!(matches!(
            update_lock,
            Err(StateError::UpdateInProgress { path }) if path == paths.update_lock()
        ));

        queued_task.abort();
        let _join_result = queued_task.await;
        running.finish();

        Ok(())
    }

    #[tokio::test]
    async fn background_reconciliation_coalesces_under_daemon_update_lock() -> anyhow::Result<()> {
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

    fn seed_cached_php_pair(paths: &PvPaths, tempdir: &Utf8Path) -> anyhow::Result<()> {
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
        let php = cache_artifact(paths, tempdir, php)?;
        let frankenphp = cache_artifact(paths, tempdir, frankenphp)?;
        let manifest = php_pair_manifest(&[php, frankenphp]);

        state::fs::write_sensitive_file(&paths.downloads().join("manifest.json"), &manifest)?;

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
              "upstream_version": "{track}",
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
            track = PHP_TEST_TRACK,
            artifact_version = artifact.artifact_version,
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
        let output = std::process::Command::new(program)
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
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(from, to)?;

        Ok(())
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "daemon jobs tests read fixture archive metadata for manifest size"
    )]
    fn file_size(path: &Utf8Path) -> anyhow::Result<u64> {
        Ok(std::fs::metadata(path)?.len())
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "daemon jobs tests set fixture executable bits directly"
    )]
    fn set_executable(path: &Utf8Path) -> anyhow::Result<()> {
        let mut permissions = std::fs::metadata(path)?.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions)?;

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
