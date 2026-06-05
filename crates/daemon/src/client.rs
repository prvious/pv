use futures_util::StreamExt;
use state::PvPaths;
use tokio::net::UnixStream;
use tokio::time::{Duration, timeout};

use crate::DaemonError;
use protocol::{
    DaemonCommand, DaemonRequest, DaemonResponse, PROTOCOL_VERSION, ResponseStatus, write_line,
};

const DAEMON_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const DAEMON_WRITE_TIMEOUT: Duration = Duration::from_secs(3);
const DAEMON_RESPONSE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmittedJob {
    pub id: String,
}

pub fn submit_job_blocking(
    paths: PvPaths,
    kind: &str,
    scope: &str,
) -> Result<SubmittedJob, DaemonError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?;

    runtime.block_on(submit_job(paths, kind, scope))
}

async fn submit_job(paths: PvPaths, kind: &str, scope: &str) -> Result<SubmittedJob, DaemonError> {
    let stream = timeout(
        DAEMON_CONNECT_TIMEOUT,
        UnixStream::connect(paths.daemon_socket()),
    )
    .await
    .map_err(|_| DaemonError::ProtocolTimedOut {
        phase: "connection",
    })??;
    let mut transport = protocol::transport(stream);
    let request = DaemonRequest {
        protocol_version: PROTOCOL_VERSION,
        command: DaemonCommand::RunJob {
            kind: kind.to_string(),
            scope: scope.to_string(),
        },
    };

    timeout(DAEMON_WRITE_TIMEOUT, write_line(&mut transport, &request))
        .await
        .map_err(|_| DaemonError::ProtocolTimedOut { phase: "write" })??;
    let Some(line) = timeout(DAEMON_RESPONSE_TIMEOUT, transport.next())
        .await
        .map_err(|_| DaemonError::ProtocolTimedOut { phase: "response" })?
    else {
        return Err(DaemonError::UnexpectedProtocolResponse {
            reason: "daemon closed before sending a response".to_string(),
        });
    };
    let response = serde_json::from_str::<DaemonResponse>(&line?)?;
    validate_response_contract(&response)?;

    match response.status() {
        ResponseStatus::Accepted => response
            .job_id()
            .map(|id| SubmittedJob { id: id.to_string() })
            .ok_or_else(|| DaemonError::UnexpectedProtocolResponse {
                reason: response.message().to_string(),
            }),
        ResponseStatus::Ok | ResponseStatus::Error => Err(DaemonError::DaemonRejected {
            message: response.message().to_string(),
        }),
    }
}

fn validate_response_contract(response: &DaemonResponse) -> Result<(), DaemonError> {
    if response.line_type() != "response" {
        return Err(DaemonError::UnexpectedProtocolResponse {
            reason: format!("daemon sent unexpected `{}` line", response.line_type()),
        });
    }
    if response.protocol_version() != PROTOCOL_VERSION {
        return Err(DaemonError::ProtocolMismatch {
            expected: PROTOCOL_VERSION,
            actual: response.protocol_version(),
        });
    }

    Ok(())
}
