use state::{Database, PvPaths};
use tokio::net::UnixStream;

use crate::DaemonError;
use crate::protocol::{DaemonEvent, DaemonResponse, PROTOCOL_VERSION, ResponseStatus, write_line};

pub(crate) async fn run_job(
    paths: PvPaths,
    mut stream: UnixStream,
    kind: &str,
    scope: &str,
) -> Result<(), DaemonError> {
    let mut database = Database::open(&paths)?;
    let job = database.start_job(kind, scope)?;
    let summary = "stub job completed";

    let write_result = async {
        write_line(
            &mut stream,
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
            &mut stream,
            &DaemonEvent::JobStarted {
                job_id: &job.id,
                kind,
                scope,
            },
        )
        .await?;
        write_line(
            &mut stream,
            &DaemonEvent::Progress {
                job_id: &job.id,
                message: "stub job completed without reconciliation work",
            },
        )
        .await?;

        Ok::<(), DaemonError>(())
    }
    .await;

    database.complete_job(&job.id, summary)?;
    write_result?;

    write_line(
        &mut stream,
        &DaemonEvent::JobCompleted {
            job_id: &job.id,
            summary,
        },
    )
    .await
}
