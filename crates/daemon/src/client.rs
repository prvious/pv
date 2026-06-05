use futures_util::StreamExt;
use serde_json::Value;
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
const DAEMON_EVENT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmittedJob {
    pub id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletedJob {
    pub id: String,
    pub summary: String,
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

pub fn run_job_blocking(
    paths: PvPaths,
    kind: &str,
    scope: &str,
) -> Result<CompletedJob, DaemonError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?;

    runtime.block_on(run_job(paths, kind, scope))
}

async fn submit_job(paths: PvPaths, kind: &str, scope: &str) -> Result<SubmittedJob, DaemonError> {
    let mut transport = connect_transport(&paths).await?;

    write_job_request(&mut transport, kind, scope).await?;
    let response = read_response(&mut transport).await?;
    validate_response_contract(&response)?;

    accepted_job_id(&response).map(|id| SubmittedJob { id })
}

async fn run_job(paths: PvPaths, kind: &str, scope: &str) -> Result<CompletedJob, DaemonError> {
    let mut transport = connect_transport(&paths).await?;

    write_job_request(&mut transport, kind, scope).await?;
    let response = read_response(&mut transport).await?;
    validate_response_contract(&response)?;
    let id = accepted_job_id(&response)?;
    let summary = read_job_completion(&mut transport, &id).await?;

    Ok(CompletedJob { id, summary })
}

async fn connect_transport(
    paths: &PvPaths,
) -> Result<protocol::DaemonTransport<UnixStream>, DaemonError> {
    let stream = timeout(
        DAEMON_CONNECT_TIMEOUT,
        UnixStream::connect(paths.daemon_socket()),
    )
    .await
    .map_err(|_| DaemonError::ProtocolTimedOut {
        phase: "connection",
    })??;

    Ok(protocol::transport(stream))
}

async fn write_job_request(
    transport: &mut protocol::DaemonTransport<UnixStream>,
    kind: &str,
    scope: &str,
) -> Result<(), DaemonError> {
    let request = DaemonRequest {
        protocol_version: PROTOCOL_VERSION,
        command: DaemonCommand::RunJob {
            kind: kind.to_string(),
            scope: scope.to_string(),
        },
    };

    timeout(DAEMON_WRITE_TIMEOUT, write_line(transport, &request))
        .await
        .map_err(|_| DaemonError::ProtocolTimedOut { phase: "write" })?
        .map_err(DaemonError::from)
}

async fn read_response(
    transport: &mut protocol::DaemonTransport<UnixStream>,
) -> Result<DaemonResponse, DaemonError> {
    let Some(line) = timeout(DAEMON_RESPONSE_TIMEOUT, transport.next())
        .await
        .map_err(|_| DaemonError::ProtocolTimedOut { phase: "response" })?
    else {
        return Err(DaemonError::UnexpectedProtocolResponse {
            reason: "daemon closed before sending a response".to_string(),
        });
    };

    serde_json::from_str::<DaemonResponse>(&line?).map_err(DaemonError::from)
}

async fn read_job_completion(
    transport: &mut protocol::DaemonTransport<UnixStream>,
    expected_job_id: &str,
) -> Result<String, DaemonError> {
    loop {
        let Some(line) = timeout(DAEMON_EVENT_TIMEOUT, transport.next())
            .await
            .map_err(|_| DaemonError::ProtocolTimedOut { phase: "job event" })?
        else {
            return Err(DaemonError::UnexpectedProtocolResponse {
                reason: "daemon closed before completing the job".to_string(),
            });
        };

        if let Some(summary) = parse_job_event(&line?, expected_job_id)? {
            return Ok(summary);
        }
    }
}

fn accepted_job_id(response: &DaemonResponse) -> Result<String, DaemonError> {
    match response.status() {
        ResponseStatus::Accepted => response.job_id().map(ToString::to_string).ok_or_else(|| {
            DaemonError::UnexpectedProtocolResponse {
                reason: response.message().to_string(),
            }
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

fn parse_job_event(line: &str, expected_job_id: &str) -> Result<Option<String>, DaemonError> {
    let value = serde_json::from_str::<Value>(line)?;
    let line_type = value.get("type").and_then(Value::as_str).ok_or_else(|| {
        DaemonError::UnexpectedProtocolResponse {
            reason: "daemon sent event without a type".to_string(),
        }
    })?;

    match line_type {
        "job_started" | "progress" | "log" => {
            validate_job_event_id(&value, expected_job_id)?;

            Ok(None)
        }
        "job_completed" => {
            validate_job_event_id(&value, expected_job_id)?;
            let summary = value
                .get("summary")
                .and_then(Value::as_str)
                .ok_or_else(|| DaemonError::UnexpectedProtocolResponse {
                    reason: "daemon sent job_completed without a summary".to_string(),
                })?;

            Ok(Some(summary.to_string()))
        }
        "job_failed" => {
            validate_job_event_id(&value, expected_job_id)?;
            let error = value.get("error").and_then(Value::as_str).ok_or_else(|| {
                DaemonError::UnexpectedProtocolResponse {
                    reason: "daemon sent job_failed without an error".to_string(),
                }
            })?;

            Err(DaemonError::DaemonRejected {
                message: error.to_string(),
            })
        }
        _ => Err(DaemonError::UnexpectedProtocolResponse {
            reason: format!("daemon sent unexpected `{line_type}` line"),
        }),
    }
}

fn validate_job_event_id(value: &Value, expected_job_id: &str) -> Result<(), DaemonError> {
    let actual_job_id = value.get("job_id").and_then(Value::as_str).ok_or_else(|| {
        DaemonError::UnexpectedProtocolResponse {
            reason: "daemon sent job event without a job_id".to_string(),
        }
    })?;

    if actual_job_id == expected_job_id {
        return Ok(());
    }

    Err(DaemonError::UnexpectedProtocolResponse {
        reason: format!(
            "daemon sent event for job `{actual_job_id}` while waiting for `{expected_job_id}`"
        ),
    })
}
