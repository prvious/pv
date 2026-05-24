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
    mut transport: DaemonTransport<LocalStream>,
    kind: &str,
    scope: &str,
) -> Result<(), DaemonError> {
    let parsed_scope = scope.parse::<ReconciliationScope>();
    if kind == "reconcile"
        && let Ok(parsed_scope) = parsed_scope
    {
        let EnqueueResult::Queued(queued) = queue.enqueue(parsed_scope).await else {
            return write_line(
                &mut transport,
                &DaemonResponse {
                    line_type: "response",
                    protocol_version: PROTOCOL_VERSION,
                    status: ResponseStatus::Accepted,
                    message: "reconciliation already queued or running",
                    job_id: None,
                },
            )
            .await;
        };
        let running = queued.wait_for_turn().await;
        let active_scope = running.scope().to_string();
        let result = run_started_job(paths, transport, kind, &active_scope).await;

        running.finish().await;

        return result;
    }

    run_started_job(paths, transport, kind, scope).await
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
