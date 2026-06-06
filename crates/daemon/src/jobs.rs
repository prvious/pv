use crate::DaemonError;
use crate::gateway::reconcile_gateway_runtimes;
use crate::ipc::LocalStream;
use crate::project_env::reconcile_project_env;
use crate::reconciliation::{EnqueueResult, ReconciliationQueue, ReconciliationScope};
use protocol::{DaemonEvent, DaemonResponse, DaemonTransport, write_line};
use state::{Database, JobStatus, PvPaths};
use tokio::io::AsyncWrite;

pub(crate) async fn run_job(
    paths: PvPaths,
    queue: ReconciliationQueue,
    transport: DaemonTransport<LocalStream>,
    kind: &str,
    scope: &str,
) -> Result<(), DaemonError> {
    let parsed_scope = scope.parse::<ReconciliationScope>();
    if kind == "reconcile" {
        return match parsed_scope {
            Ok(parsed_scope) => run_reconciliation_job(paths, queue, transport, parsed_scope).await,
            Err(error) => {
                run_invalid_reconciliation_scope_job(paths, transport, scope, error).await
            }
        };
    }

    run_started_job(paths, transport, kind, scope).await
}

pub(crate) async fn run_background_reconciliation_job(
    paths: PvPaths,
    queue: ReconciliationQueue,
    scope: ReconciliationScope,
) -> Result<(), DaemonError> {
    let result = enqueue_reconciliation_job(&paths, &queue, scope)?;
    let EnqueueResult::Queued(queued) = result else {
        return Ok(());
    };
    let running = queued.wait_for_turn().await;
    let job_id = running.job_id().to_string();
    let scope = running.scope().clone();
    let result = complete_reconciliation_job(&paths, &job_id, &scope)
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
) -> Result<(), DaemonError> {
    let result = enqueue_reconciliation_job(&paths, &queue, scope)?;

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

    queue.enqueue_with_abandon(
        scope,
        || start_reconciliation_job(paths, &scope_text),
        move |job_id| {
            let _result = abandon_reconciliation_job(&abandon_paths, job_id);
        },
    )
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

    let reconciliation_result = complete_reconciliation_job(&paths, job_id, &scope).await;
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

fn start_reconciliation_job(paths: &PvPaths, scope: &str) -> Result<String, DaemonError> {
    let mut database = Database::open(paths)?;
    let job = database.start_job("reconcile", scope)?;

    Ok(job.id)
}

fn abandon_reconciliation_job(paths: &PvPaths, job_id: &str) -> Result<(), DaemonError> {
    let mut database = Database::open(paths)?;
    database.fail_job(job_id, "reconciliation was abandoned before completion")?;

    Ok(())
}

fn complete_stub_reconciliation(
    paths: &PvPaths,
    job_id: &str,
) -> Result<&'static str, DaemonError> {
    let mut database = Database::open(paths)?;
    let summary = "stub job completed";

    database.complete_job(job_id, summary)?;

    Ok(summary)
}

async fn complete_reconciliation_job(
    paths: &PvPaths,
    job_id: &str,
    scope: &ReconciliationScope,
) -> Result<String, DaemonError> {
    let result = match scope {
        ReconciliationScope::System => complete_gateway_reconciliation(paths, job_id).await,
        ReconciliationScope::Resource { name, .. } if gateway_runtime_resource(name.as_str()) => {
            complete_gateway_reconciliation(paths, job_id).await
        }
        ReconciliationScope::Resource { .. } => {
            complete_stub_reconciliation(paths, job_id).map(str::to_string)
        }
        ReconciliationScope::Project { id } => {
            complete_project_reconciliation(paths, job_id, id).await
        }
    };

    if let Err(error) = &result {
        let error_message = error.to_string();
        let mut database = Database::open(paths)?;
        database.fail_job(job_id, &error_message)?;
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

async fn complete_project_reconciliation(
    paths: &PvPaths,
    job_id: &str,
    id: &crate::reconciliation::ReconciliationScopeComponent,
) -> Result<String, DaemonError> {
    let project_env_summary = reconcile_project_env(paths, id.as_str())?;
    let gateway_summary = reconcile_gateway_runtimes(paths).await?;
    let summary = format!("{}; {gateway_summary}", project_env_summary.as_str());
    let mut database = Database::open(paths)?;

    database.complete_job(job_id, &summary)?;

    Ok(summary)
}

fn reconciliation_progress_message(scope: &ReconciliationScope, summary: &str) -> String {
    match scope {
        ReconciliationScope::Resource { name, .. } if !gateway_runtime_resource(name.as_str()) => {
            "stub job completed without reconciliation work".to_string()
        }
        ReconciliationScope::System
        | ReconciliationScope::Resource { .. }
        | ReconciliationScope::Project { .. } => summary.to_string(),
    }
}

fn reconciliation_started_message(scope: &ReconciliationScope) -> &'static str {
    match scope {
        ReconciliationScope::Project { .. } => "Project env reconciliation started",
        ReconciliationScope::System => "Gateway runtime reconciliation started",
        ReconciliationScope::Resource { name, .. } if gateway_runtime_resource(name.as_str()) => {
            "Gateway runtime reconciliation started"
        }
        ReconciliationScope::Resource { .. } => "stub job started",
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
    database.fail_job(&job.id, &error_message)?;

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

    use camino_tempfile::tempdir;
    use state::{Database, JobStatus, PvPaths, StateError};
    use tokio::io::duplex;

    use super::{
        complete_or_fail_background_reconciliation, enqueue_reconciliation_job,
        foreground_reconciliation_result, record_background_reconciliation_error,
        start_reconciliation_job, stream_started_reconciliation_job,
    };
    use crate::reconciliation::{EnqueueResult, ReconciliationQueue, ReconciliationScope};

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
}
