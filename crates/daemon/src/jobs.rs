use crate::DaemonError;
use crate::protocol::{
    DaemonEvent, DaemonResponse, DaemonTransport, PROTOCOL_VERSION, ResponseStatus, write_line,
};
use state::{Database, PvPaths};

pub(crate) async fn run_job(
    paths: PvPaths,
    mut transport: DaemonTransport,
    kind: &str,
    scope: &str,
) -> Result<(), DaemonError> {
    let mut database = Database::open(&paths)?;
    let job = database.start_job(kind, scope)?;
    let summary = "stub job completed";

    let write_result = async {
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
        &mut transport,
        &DaemonEvent::JobCompleted {
            job_id: &job.id,
            summary,
        },
    )
    .await
}
