use futures_util::StreamExt;
use serde_json::Value;
use state::PvPaths;
use tokio::time::{Duration, Instant, sleep, timeout};

use crate::{DaemonError, ipc};
use protocol::{
    DaemonCommand, DaemonRequest, DaemonResponse, ManagedResourceUpdateCheck, PROTOCOL_VERSION,
    ResponseStatus, write_line,
};

const DAEMON_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const DAEMON_WRITE_TIMEOUT: Duration = Duration::from_secs(3);
const DAEMON_RESPONSE_TIMEOUT: Duration = Duration::from_secs(3);
const DAEMON_EVENT_TIMEOUT: Duration = Duration::from_secs(30);
const DAEMON_HEALTH_TIMEOUT: Duration = Duration::from_secs(15);
const DAEMON_HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmittedJob {
    pub id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletedJob {
    pub id: String,
    pub summary: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobDownloadProgress {
    pub resource: String,
    pub track: String,
    pub artifact_version: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
}

pub trait JobEventHandler {
    fn download_progress(&mut self, _progress: JobDownloadProgress) {}
}

#[derive(Debug, Default)]
struct NoJobEvents;

impl JobEventHandler for NoJobEvents {}

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
    let mut events = NoJobEvents;

    run_job_with_events_blocking(paths, kind, scope, &mut events)
}

pub fn run_job_with_events_blocking(
    paths: PvPaths,
    kind: &str,
    scope: &str,
    events: &mut impl JobEventHandler,
) -> Result<CompletedJob, DaemonError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?;

    runtime.block_on(run_job(paths, kind, scope, events))
}

pub fn wait_until_healthy_blocking(paths: PvPaths) -> Result<(), DaemonError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?;

    runtime.block_on(wait_until_healthy(paths))
}

pub fn wait_until_healthy_allowing_protocol_mismatch_blocking(
    paths: PvPaths,
) -> Result<(), DaemonError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?;

    runtime.block_on(wait_until_healthy_allowing_protocol_mismatch(paths))
}

pub fn health_blocking(paths: PvPaths) -> Result<(), DaemonError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?;

    runtime.block_on(health(paths))
}

pub fn managed_resource_update_check_blocking(
    paths: PvPaths,
) -> Result<ManagedResourceUpdateCheck, DaemonError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?;

    runtime.block_on(managed_resource_update_check(paths))
}

async fn submit_job(paths: PvPaths, kind: &str, scope: &str) -> Result<SubmittedJob, DaemonError> {
    let mut transport = connect_transport(&paths).await?;

    write_job_request(&mut transport, kind, scope).await?;
    let response = read_response(&mut transport).await?;
    validate_response_contract(&response)?;

    accepted_job_id(&response).map(|id| SubmittedJob { id })
}

async fn run_job(
    paths: PvPaths,
    kind: &str,
    scope: &str,
    events: &mut impl JobEventHandler,
) -> Result<CompletedJob, DaemonError> {
    let mut transport = connect_transport(&paths).await?;

    write_job_request(&mut transport, kind, scope).await?;
    let response = read_response(&mut transport).await?;
    validate_response_contract(&response)?;
    let id = accepted_job_id(&response)?;
    let summary = read_job_completion(&mut transport, &id, events).await?;

    Ok(CompletedJob { id, summary })
}

async fn wait_until_healthy(paths: PvPaths) -> Result<(), DaemonError> {
    let started_at = Instant::now();

    loop {
        match health(paths.clone()).await {
            Ok(()) => return Ok(()),
            Err(error) if health_error_is_retryable(&error, started_at) => {
                sleep(DAEMON_HEALTH_POLL_INTERVAL).await;
            }
            Err(error) => return Err(error),
        }
    }
}

async fn wait_until_healthy_allowing_protocol_mismatch(paths: PvPaths) -> Result<(), DaemonError> {
    let started_at = Instant::now();

    loop {
        match health_allowing_protocol_mismatch(paths.clone()).await {
            Ok(()) => return Ok(()),
            Err(error) if health_error_is_retryable(&error, started_at) => {
                sleep(DAEMON_HEALTH_POLL_INTERVAL).await;
            }
            Err(error) => return Err(error),
        }
    }
}

async fn health(paths: PvPaths) -> Result<(), DaemonError> {
    let mut transport = connect_transport(&paths).await?;

    write_health_request(&mut transport).await?;
    let response = read_response(&mut transport).await?;
    validate_response_contract(&response)?;

    match response.status() {
        ResponseStatus::Ok => Ok(()),
        ResponseStatus::Accepted | ResponseStatus::Error => Err(DaemonError::DaemonRejected {
            message: response.message().to_string(),
        }),
    }
}

async fn health_allowing_protocol_mismatch(paths: PvPaths) -> Result<(), DaemonError> {
    let mut transport = connect_transport(&paths).await?;

    write_health_request(&mut transport).await?;
    let response = read_response(&mut transport).await?;
    validate_response_line_type(&response)?;

    match response.status() {
        ResponseStatus::Ok => Ok(()),
        ResponseStatus::Accepted | ResponseStatus::Error => Err(DaemonError::DaemonRejected {
            message: response.message().to_string(),
        }),
    }
}

async fn managed_resource_update_check(
    paths: PvPaths,
) -> Result<ManagedResourceUpdateCheck, DaemonError> {
    let mut transport = connect_transport(&paths).await?;

    write_managed_resource_update_check_request(&mut transport).await?;
    let response = read_response(&mut transport).await?;
    validate_response_contract(&response)?;

    match response.status() {
        ResponseStatus::Ok => response.update_check().cloned().ok_or_else(|| {
            DaemonError::UnexpectedProtocolResponse {
                reason: "daemon accepted update check without an update_check payload".to_string(),
            }
        }),
        ResponseStatus::Accepted | ResponseStatus::Error => Err(DaemonError::DaemonRejected {
            message: response.message().to_string(),
        }),
    }
}

async fn connect_transport(
    paths: &PvPaths,
) -> Result<protocol::DaemonTransport<ipc::LocalStream>, DaemonError> {
    let stream = timeout(DAEMON_CONNECT_TIMEOUT, ipc::connect(paths))
        .await
        .map_err(|_| DaemonError::ProtocolTimedOut {
            phase: "connection",
        })??;

    Ok(protocol::transport(stream))
}

async fn write_health_request(
    transport: &mut protocol::DaemonTransport<ipc::LocalStream>,
) -> Result<(), DaemonError> {
    let request = DaemonRequest {
        protocol_version: PROTOCOL_VERSION,
        command: DaemonCommand::Health,
    };

    timeout(DAEMON_WRITE_TIMEOUT, write_line(transport, &request))
        .await
        .map_err(|_| DaemonError::ProtocolTimedOut { phase: "write" })?
        .map_err(DaemonError::from)
}

async fn write_job_request(
    transport: &mut protocol::DaemonTransport<ipc::LocalStream>,
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

async fn write_managed_resource_update_check_request(
    transport: &mut protocol::DaemonTransport<ipc::LocalStream>,
) -> Result<(), DaemonError> {
    let request = DaemonRequest {
        protocol_version: PROTOCOL_VERSION,
        command: DaemonCommand::ManagedResourceUpdateCheck,
    };

    timeout(DAEMON_WRITE_TIMEOUT, write_line(transport, &request))
        .await
        .map_err(|_| DaemonError::ProtocolTimedOut { phase: "write" })?
        .map_err(DaemonError::from)
}

fn health_error_is_retryable(error: &DaemonError, started_at: Instant) -> bool {
    if started_at.elapsed() >= DAEMON_HEALTH_TIMEOUT {
        return false;
    }

    matches!(
        error,
        DaemonError::Io(source)
            if matches!(
                source.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            )
    ) || matches!(
        error,
        DaemonError::ProtocolTimedOut {
            phase: "connection" | "response"
        }
    )
}

async fn read_response(
    transport: &mut protocol::DaemonTransport<ipc::LocalStream>,
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
    transport: &mut protocol::DaemonTransport<ipc::LocalStream>,
    expected_job_id: &str,
    events: &mut impl JobEventHandler,
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

        if let Some(summary) = parse_job_event(&line?, expected_job_id, events)? {
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
    validate_response_line_type(response)?;
    validate_response_protocol(response)
}

fn validate_response_line_type(response: &DaemonResponse) -> Result<(), DaemonError> {
    if response.line_type() != "response" {
        return Err(DaemonError::UnexpectedProtocolResponse {
            reason: format!("daemon sent unexpected `{}` line", response.line_type()),
        });
    }

    Ok(())
}

fn validate_response_protocol(response: &DaemonResponse) -> Result<(), DaemonError> {
    if response.protocol_version() != PROTOCOL_VERSION {
        return Err(DaemonError::ProtocolMismatch {
            expected: PROTOCOL_VERSION,
            actual: response.protocol_version(),
        });
    }

    Ok(())
}

fn parse_job_event(
    line: &str,
    expected_job_id: &str,
    events: &mut impl JobEventHandler,
) -> Result<Option<String>, DaemonError> {
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
        "download_progress" => {
            validate_job_event_id(&value, expected_job_id)?;
            events.download_progress(parse_download_progress_event(&value)?);

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

fn parse_download_progress_event(value: &Value) -> Result<JobDownloadProgress, DaemonError> {
    let string_field = |field: &str| {
        value
            .get(field)
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .ok_or_else(|| DaemonError::UnexpectedProtocolResponse {
                reason: format!("daemon sent download_progress without a {field}"),
            })
    };
    let u64_field = |field: &str| {
        value.get(field).and_then(Value::as_u64).ok_or_else(|| {
            DaemonError::UnexpectedProtocolResponse {
                reason: format!("daemon sent download_progress without a {field}"),
            }
        })
    };

    Ok(JobDownloadProgress {
        resource: string_field("resource")?,
        track: string_field("track")?,
        artifact_version: string_field("artifact_version")?,
        downloaded_bytes: u64_field("downloaded_bytes")?,
        total_bytes: u64_field("total_bytes")?,
    })
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

#[cfg(test)]
mod tests {
    use super::{JobDownloadProgress, JobEventHandler, parse_job_event};

    #[derive(Default)]
    struct RecordingEvents {
        downloads: Vec<JobDownloadProgress>,
    }

    impl JobEventHandler for RecordingEvents {
        fn download_progress(&mut self, progress: JobDownloadProgress) {
            self.downloads.push(progress);
        }
    }

    #[test]
    fn parse_job_event_reports_download_progress_events() -> anyhow::Result<()> {
        let mut events = RecordingEvents::default();
        let completed = parse_job_event(
            r#"{"type":"download_progress","job_id":"job-1","resource":"redis","track":"8.8","artifact_version":"8.8.1-pv1","downloaded_bytes":42,"total_bytes":100}"#,
            "job-1",
            &mut events,
        )?;

        assert_eq!(completed, None);
        assert_eq!(
            events.downloads,
            vec![JobDownloadProgress {
                resource: "redis".to_string(),
                track: "8.8".to_string(),
                artifact_version: "8.8.1-pv1".to_string(),
                downloaded_bytes: 42,
                total_bytes: 100,
            }]
        );

        Ok(())
    }
}
