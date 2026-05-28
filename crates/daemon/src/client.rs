use futures_util::StreamExt;
use serde::Deserialize;
use state::PvPaths;
use tokio::net::UnixStream;

use crate::DaemonError;
use crate::protocol::{DaemonCommand, DaemonRequest, PROTOCOL_VERSION, ResponseStatus, write_line};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmittedJob {
    pub id: String,
}

#[derive(Debug, Deserialize)]
struct ClientResponse {
    status: ResponseStatus,
    job_id: Option<String>,
    message: String,
}

pub fn submit_job_blocking(
    paths: PvPaths,
    kind: &str,
    scope: &str,
) -> Result<SubmittedJob, DaemonError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()?;

    runtime.block_on(submit_job(paths, kind, scope))
}

async fn submit_job(paths: PvPaths, kind: &str, scope: &str) -> Result<SubmittedJob, DaemonError> {
    let stream = UnixStream::connect(paths.daemon_socket()).await?;
    let mut transport = crate::protocol::transport(stream);
    let request = DaemonRequest {
        protocol_version: PROTOCOL_VERSION,
        command: DaemonCommand::RunJob {
            kind: kind.to_string(),
            scope: scope.to_string(),
        },
    };

    write_line(&mut transport, &request).await?;
    let Some(line) = transport.next().await else {
        return Err(DaemonError::Protocol(
            "daemon closed before sending a response".to_string(),
        ));
    };
    let response = serde_json::from_str::<ClientResponse>(&line?)?;

    match response.status {
        ResponseStatus::Accepted => response
            .job_id
            .map(|id| SubmittedJob { id })
            .ok_or_else(|| DaemonError::Protocol(response.message)),
        ResponseStatus::Ok | ResponseStatus::Error => Err(DaemonError::Protocol(response.message)),
    }
}
