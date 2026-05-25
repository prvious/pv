use crate::DaemonError;
use crate::ipc::LocalStream;
use crate::protocol::{
    DaemonEvent, DaemonResponse, DaemonTransport, PROTOCOL_VERSION, ResponseStatus, write_line,
};
use crate::reconciliation::{EnqueueResult, ReconciliationQueue, ReconciliationScope};
use state::{Database, PvPaths};

pub(crate) async fn run_job(
    paths: PvPaths,
    queue: ReconciliationQueue,
    transport: DaemonTransport<LocalStream>,
    kind: &str,
    scope: &str,
) -> Result<(), DaemonError> {
    let parsed_scope = scope.parse::<ReconciliationScope>();
    if kind == "reconcile"
        && let Ok(parsed_scope) = parsed_scope
    {
        return run_reconciliation_job(paths, queue, transport, parsed_scope).await;
    }

    run_started_job(paths, transport, kind, scope).await
}

pub(crate) async fn run_background_reconciliation_job(
    paths: PvPaths,
    queue: ReconciliationQueue,
    scope: ReconciliationScope,
) -> Result<(), DaemonError> {
    let scope_text = scope.to_string();
    let result = queue.enqueue(scope, || start_reconciliation_job(&paths, &scope_text))?;
    let EnqueueResult::Queued(queued) = result else {
        return Ok(());
    };
    let running = queued.wait_for_turn().await;
    let job_id = running.job_id().to_string();
    let result = complete_stub_reconciliation(&paths, &job_id);

    running.finish();

    result.map(|_summary| ())
}

async fn run_reconciliation_job(
    paths: PvPaths,
    queue: ReconciliationQueue,
    mut transport: DaemonTransport<LocalStream>,
    scope: ReconciliationScope,
) -> Result<(), DaemonError> {
    let scope_text = scope.to_string();
    let result = queue.enqueue(scope, || start_reconciliation_job(&paths, &scope_text))?;

    match result {
        EnqueueResult::Queued(queued) => {
            let job_id = queued.job_id().to_string();
            let stream_is_open = write_line(
                &mut transport,
                &DaemonResponse {
                    line_type: "response",
                    protocol_version: PROTOCOL_VERSION,
                    status: ResponseStatus::Accepted,
                    message: "job accepted",
                    job_id: Some(&job_id),
                },
            )
            .await
            .is_ok();
            let running = queued.wait_for_turn().await;
            let result = stream_started_reconciliation_job(
                paths,
                transport,
                stream_is_open,
                running.job_id(),
                &scope_text,
            )
            .await;

            running.finish();

            result
        }
        EnqueueResult::Coalesced(job) => {
            write_line(
                &mut transport,
                &DaemonResponse {
                    line_type: "response",
                    protocol_version: PROTOCOL_VERSION,
                    status: ResponseStatus::Accepted,
                    message: "reconciliation already queued or running",
                    job_id: Some(job.job_id()),
                },
            )
            .await
        }
    }
}

async fn stream_started_reconciliation_job(
    paths: PvPaths,
    mut transport: DaemonTransport<LocalStream>,
    stream_is_open: bool,
    job_id: &str,
    scope: &str,
) -> Result<(), DaemonError> {
    let stream_is_open = stream_is_open
        && async {
            write_line(
                &mut transport,
                &DaemonEvent::JobStarted {
                    job_id,
                    kind: "reconcile",
                    scope,
                },
            )
            .await?;
            write_line(
                &mut transport,
                &DaemonEvent::Log {
                    job_id,
                    message: "stub job started",
                },
            )
            .await?;

            Ok::<(), DaemonError>(())
        }
        .await
        .is_ok();

    let summary = complete_stub_reconciliation(&paths, job_id)?;
    if !stream_is_open {
        return Ok(());
    }

    async {
        write_line(
            &mut transport,
            &DaemonEvent::Progress {
                job_id,
                message: "stub job completed without reconciliation work",
            },
        )
        .await?;
        write_line(
            &mut transport,
            &DaemonEvent::JobCompleted { job_id, summary },
        )
        .await?;

        Ok::<(), DaemonError>(())
    }
    .await
}

fn start_reconciliation_job(paths: &PvPaths, scope: &str) -> Result<String, DaemonError> {
    let mut database = Database::open(paths)?;
    let job = database.start_job("reconcile", scope)?;

    Ok(job.id)
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
            &DaemonResponse {
                line_type: "response",
                protocol_version: PROTOCOL_VERSION,
                status: ResponseStatus::Accepted,
                message: "job accepted",
                job_id: Some(&job.id),
            },
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
    .await
}
