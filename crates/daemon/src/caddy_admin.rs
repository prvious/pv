use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use camino::{Utf8Path, Utf8PathBuf};
use http_body_util::{BodyExt as _, Full};
use hyper::client::conn::http1;
use hyper::header::HOST;
use hyper::{Method, Request};
use hyper_util::rt::TokioIo;
#[cfg(unix)]
use rustix::process::{Pid, getpgid};
use thiserror::Error;
#[cfg(unix)]
use tokio::net::UnixStream;

pub const MAX_RESPONSE_DETAIL_BYTES: usize = 4096;

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const DEFAULT_WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_OVERALL_TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// The Unix-domain socket path for one Caddy admin endpoint.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CaddyAdminEndpoint {
    path: Utf8PathBuf,
}

impl CaddyAdminEndpoint {
    pub fn new(path: impl Into<Utf8PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Utf8Path {
        &self.path
    }
}

impl fmt::Display for CaddyAdminEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.path.fmt(formatter)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaddyAdminOperation {
    Load,
    Readiness,
    Rollback,
}

impl fmt::Display for CaddyAdminOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Load => formatter.write_str("load"),
            Self::Readiness => formatter.write_str("readiness"),
            Self::Rollback => formatter.write_str("rollback"),
        }
    }
}

#[derive(Debug, Error)]
pub enum CaddyAdminError {
    #[error("Caddy admin endpoint {endpoint} is unavailable: {reason}")]
    EndpointUnavailable {
        endpoint: CaddyAdminEndpoint,
        reason: String,
    },

    #[error("Caddy admin {operation} request to {endpoint} timed out after {timeout_ms}ms")]
    RequestTimedOut {
        endpoint: CaddyAdminEndpoint,
        operation: CaddyAdminOperation,
        timeout_ms: u128,
    },

    #[error(
        "Caddy admin {operation} request to {endpoint} may still be executing or may have been accepted: {reason}"
    )]
    RequestOutcomeUnknown {
        endpoint: CaddyAdminEndpoint,
        operation: CaddyAdminOperation,
        reason: String,
    },

    #[error("Caddy admin {operation} request to {endpoint} was not sent: {reason}")]
    RequestNotSent {
        endpoint: CaddyAdminEndpoint,
        operation: CaddyAdminOperation,
        reason: Box<Self>,
    },

    #[error(
        "Caddy admin endpoint {endpoint} belongs to process group {actual_process_group}, expected managed process group {expected_process_group}"
    )]
    PeerProcessGroupMismatch {
        endpoint: CaddyAdminEndpoint,
        expected_process_group: i32,
        actual_process_group: i32,
    },

    #[error("Caddy admin load at {endpoint} was rejected with HTTP {status}: {detail}")]
    LoadRejected {
        endpoint: CaddyAdminEndpoint,
        status: u16,
        detail: String,
    },

    #[error(
        "Caddy admin endpoint {endpoint} did not become ready within {timeout_ms}ms; last error: {last_error:?}"
    )]
    AdminReadinessTimedOut {
        endpoint: CaddyAdminEndpoint,
        timeout_ms: u128,
        last_error: Option<String>,
    },

    #[error("runtime `{runtime}` ownership changed before Caddy admin load")]
    RuntimeOwnershipChanged { runtime: String },

    #[error(
        "Caddy admin rollback failed after `{original_error}`, and restored-config reload also failed: {restored_error}"
    )]
    RestoredConfigReloadFailed {
        original_error: Box<Self>,
        restored_error: Box<Self>,
    },

    #[error("Caddy admin {operation} task failed: {reason}")]
    TaskFailed {
        operation: CaddyAdminOperation,
        reason: String,
    },

    #[error("Caddy admin endpoint {endpoint} returned HTTP {status} while checking readiness")]
    UnexpectedReadinessStatus {
        endpoint: CaddyAdminEndpoint,
        status: u16,
    },
}

/// Verifies runtime ownership and returns its root process id when peer validation is required.
pub type CaddyAdminVerifier =
    Arc<dyn Fn(CaddyAdminOperation) -> Result<Option<u32>, CaddyAdminError> + Send + Sync>;

impl CaddyAdminError {
    pub fn runtime_ownership_changed(runtime: impl Into<String>) -> Self {
        Self::RuntimeOwnershipChanged {
            runtime: runtime.into(),
        }
    }

    pub fn restored_config_reload_failed(original_error: Self, restored_error: Self) -> Self {
        Self::RestoredConfigReloadFailed {
            original_error: Box::new(original_error),
            restored_error: Box::new(restored_error),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaddyAdminTimeouts {
    pub connect: Duration,
    pub write: Duration,
    pub read: Duration,
    pub overall: Duration,
    pub poll_interval: Duration,
}

impl CaddyAdminTimeouts {
    pub const fn new(
        connect: Duration,
        write: Duration,
        read: Duration,
        overall: Duration,
        poll_interval: Duration,
    ) -> Self {
        Self {
            connect,
            write,
            read,
            overall,
            poll_interval,
        }
    }
}

impl Default for CaddyAdminTimeouts {
    fn default() -> Self {
        Self {
            connect: DEFAULT_CONNECT_TIMEOUT,
            write: DEFAULT_WRITE_TIMEOUT,
            read: DEFAULT_READ_TIMEOUT,
            overall: DEFAULT_OVERALL_TIMEOUT,
            poll_interval: DEFAULT_POLL_INTERVAL,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CaddyAdminClient {
    timeouts: CaddyAdminTimeouts,
}

impl CaddyAdminClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub const fn with_timeouts(timeouts: CaddyAdminTimeouts) -> Self {
        Self { timeouts }
    }

    pub const fn timeouts(self) -> CaddyAdminTimeouts {
        self.timeouts
    }

    pub fn with_timeout(self, timeout: Duration) -> Self {
        let current = self.timeouts;
        Self::with_timeouts(CaddyAdminTimeouts::new(
            current.connect.min(timeout),
            current.write.min(timeout),
            current.read.min(timeout),
            current.overall.min(timeout),
            current.poll_interval.min(timeout),
        ))
    }

    pub async fn load_caddyfile(
        self,
        endpoint: &CaddyAdminEndpoint,
        bytes: impl AsRef<[u8]>,
    ) -> Result<(), CaddyAdminError> {
        self.load_caddyfile_with(endpoint, bytes, no_op_verifier())
            .await
    }

    pub async fn load_caddyfile_with(
        self,
        endpoint: &CaddyAdminEndpoint,
        bytes: impl AsRef<[u8]>,
        verifier: CaddyAdminVerifier,
    ) -> Result<(), CaddyAdminError> {
        let request = Request::builder()
            .method(Method::POST)
            .uri("/load")
            .header(HOST, "localhost")
            .header("content-type", "text/caddyfile")
            .body(Full::new(Bytes::copy_from_slice(bytes.as_ref())))
            .map_err(|error| CaddyAdminError::TaskFailed {
                operation: CaddyAdminOperation::Load,
                reason: error.to_string(),
            })?;
        let response = self
            .send(endpoint, CaddyAdminOperation::Load, request, verifier, true)
            .await?;

        if is_success(response.status) {
            return Ok(());
        }

        Err(CaddyAdminError::LoadRejected {
            endpoint: endpoint.clone(),
            status: response.status,
            detail: response.detail,
        })
    }

    pub async fn wait_until_ready(
        self,
        endpoint: &CaddyAdminEndpoint,
        readiness_timeout: Duration,
    ) -> Result<(), CaddyAdminError> {
        self.wait_until_ready_with(endpoint, readiness_timeout, no_op_verifier())
            .await
    }

    pub async fn wait_until_ready_with(
        self,
        endpoint: &CaddyAdminEndpoint,
        readiness_timeout: Duration,
        verifier: CaddyAdminVerifier,
    ) -> Result<(), CaddyAdminError> {
        let started_at = Instant::now();
        let mut last_error = None;

        loop {
            let elapsed = started_at.elapsed();
            if elapsed >= readiness_timeout {
                return Err(readiness_timeout_error(
                    endpoint,
                    readiness_timeout,
                    last_error,
                ));
            }

            let remaining = readiness_timeout.saturating_sub(elapsed);
            match self
                .get_config(endpoint, remaining, Arc::clone(&verifier))
                .await
            {
                Ok(()) => return Ok(()),
                Err(error) if is_verifier_failure(&error) => return Err(error),
                Err(error) => last_error = Some(readiness_error_detail(&error)),
            }

            let elapsed = started_at.elapsed();
            if elapsed >= readiness_timeout {
                return Err(readiness_timeout_error(
                    endpoint,
                    readiness_timeout,
                    last_error,
                ));
            }

            let remaining = readiness_timeout.saturating_sub(elapsed);
            let poll_delay = self.timeouts.poll_interval.min(remaining);
            if !poll_delay.is_zero() {
                tokio::time::sleep(poll_delay).await;
            }
        }
    }

    async fn get_config(
        self,
        endpoint: &CaddyAdminEndpoint,
        timeout: Duration,
        verifier: CaddyAdminVerifier,
    ) -> Result<(), CaddyAdminError> {
        let request = Request::builder()
            .method(Method::GET)
            .uri("/config/")
            .header(HOST, "localhost")
            .body(Full::new(Bytes::new()))
            .map_err(|error| CaddyAdminError::TaskFailed {
                operation: CaddyAdminOperation::Readiness,
                reason: error.to_string(),
            })?;
        let response = self
            .with_timeout(timeout)
            .send(
                endpoint,
                CaddyAdminOperation::Readiness,
                request,
                verifier,
                false,
            )
            .await?;

        if is_success(response.status) {
            Ok(())
        } else {
            Err(CaddyAdminError::UnexpectedReadinessStatus {
                endpoint: endpoint.clone(),
                status: response.status,
            })
        }
    }

    #[cfg(unix)]
    async fn send(
        self,
        endpoint: &CaddyAdminEndpoint,
        operation: CaddyAdminOperation,
        request: Request<Full<Bytes>>,
        verifier: CaddyAdminVerifier,
        include_response_detail: bool,
    ) -> Result<AdminResponse, CaddyAdminError> {
        let started_at = Instant::now();
        let connect_timeout = self.timeouts.connect.min(self.timeouts.overall);
        let stream = match tokio::time::timeout(
            connect_timeout,
            UnixStream::connect(endpoint.path().as_std_path()),
        )
        .await
        {
            Ok(Ok(stream)) => stream,
            Ok(Err(error)) => {
                return Err(request_not_sent(
                    endpoint,
                    operation,
                    CaddyAdminError::EndpointUnavailable {
                        endpoint: endpoint.clone(),
                        reason: error.to_string(),
                    },
                ));
            }
            Err(_) => {
                return Err(request_not_sent(
                    endpoint,
                    operation,
                    CaddyAdminError::RequestTimedOut {
                        endpoint: endpoint.clone(),
                        operation,
                        timeout_ms: connect_timeout.as_millis(),
                    },
                ));
            }
        };

        let expected_pid = verify_before_request(&verifier, endpoint, operation)?;
        if let Some(expected_pid) = expected_pid {
            verify_peer_process_group(&stream, endpoint, operation, expected_pid)?;
        }

        let remaining = self.timeouts.overall.saturating_sub(started_at.elapsed());
        let exchange_timeout =
            remaining.min(self.timeouts.write.saturating_add(self.timeouts.read));
        match tokio::time::timeout(
            exchange_timeout,
            exchange_request(request, stream, include_response_detail, self.timeouts.read),
        )
        .await
        {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(error)) => Err(map_exchange_error(endpoint, operation, error)),
            Err(_) if operation == CaddyAdminOperation::Load => {
                Err(CaddyAdminError::RequestOutcomeUnknown {
                    endpoint: endpoint.clone(),
                    operation,
                    reason: format!("request timed out after {}ms", exchange_timeout.as_millis()),
                })
            }
            Err(_) => Err(CaddyAdminError::RequestTimedOut {
                endpoint: endpoint.clone(),
                operation,
                timeout_ms: exchange_timeout.as_millis(),
            }),
        }
    }

    #[cfg(not(unix))]
    async fn send(
        self,
        endpoint: &CaddyAdminEndpoint,
        operation: CaddyAdminOperation,
        _request: Request<Full<Bytes>>,
        _verifier: CaddyAdminVerifier,
        _include_response_detail: bool,
    ) -> Result<AdminResponse, CaddyAdminError> {
        Err(request_not_sent(
            endpoint,
            operation,
            CaddyAdminError::EndpointUnavailable {
                endpoint: endpoint.clone(),
                reason: "Unix-domain Caddy admin sockets are unsupported on this platform"
                    .to_owned(),
            },
        ))
    }
}

#[derive(Debug)]
struct AdminResponse {
    status: u16,
    detail: String,
}

#[cfg(unix)]
async fn exchange_request(
    request: Request<Full<Bytes>>,
    stream: UnixStream,
    include_response_detail: bool,
    read_timeout: Duration,
) -> Result<AdminResponse, hyper::Error> {
    let (mut sender, connection) = http1::handshake(TokioIo::new(stream)).await?;
    let connection_task = tokio::spawn(connection);
    let response = match sender.send_request(request).await {
        Ok(response) => response,
        Err(error) => {
            connection_task.abort();
            return Err(error);
        }
    };
    let status = response.status().as_u16();
    let detail = if include_response_detail && !is_success(status) {
        bounded_response_detail(response.into_body(), read_timeout).await
    } else {
        String::new()
    };
    connection_task.abort();

    Ok(AdminResponse { status, detail })
}

#[cfg(unix)]
async fn bounded_response_detail(mut body: hyper::body::Incoming, timeout: Duration) -> String {
    let read = async {
        let mut bytes = Vec::with_capacity(MAX_RESPONSE_DETAIL_BYTES.saturating_add(1));

        while bytes.len() <= MAX_RESPONSE_DETAIL_BYTES {
            let Some(frame) = body.frame().await else {
                break;
            };
            let Ok(frame) = frame else {
                return None;
            };
            let Some(data) = frame.data_ref() else {
                continue;
            };
            let remaining = MAX_RESPONSE_DETAIL_BYTES
                .saturating_add(1)
                .saturating_sub(bytes.len());
            bytes.extend_from_slice(&data[..data.len().min(remaining)]);
        }

        Some(bytes)
    };
    let Ok(Some(mut bytes)) = tokio::time::timeout(timeout, read).await else {
        return "<response detail unavailable>".to_owned();
    };
    let truncated = bytes.len() > MAX_RESPONSE_DETAIL_BYTES;
    bytes.truncate(MAX_RESPONSE_DETAIL_BYTES);
    let mut detail = match String::from_utf8(bytes) {
        Ok(detail) => detail,
        Err(_) => "<non-UTF-8 response detail>".to_owned(),
    };

    if truncated {
        detail.push_str("...");
    }

    detail
}

#[cfg(unix)]
fn verify_peer_process_group(
    stream: &UnixStream,
    endpoint: &CaddyAdminEndpoint,
    operation: CaddyAdminOperation,
    expected_pid: u32,
) -> Result<(), CaddyAdminError> {
    let peer_pid = stream
        .peer_cred()
        .map_err(|error| {
            request_not_sent(
                endpoint,
                operation,
                CaddyAdminError::EndpointUnavailable {
                    endpoint: endpoint.clone(),
                    reason: format!("failed to inspect peer credentials: {error}"),
                },
            )
        })?
        .pid()
        .ok_or_else(|| {
            request_not_sent(
                endpoint,
                operation,
                CaddyAdminError::EndpointUnavailable {
                    endpoint: endpoint.clone(),
                    reason: "peer process id is unavailable".to_owned(),
                },
            )
        })?;
    let expected_pid = process_pid(expected_pid).map_err(|reason| {
        request_not_sent(
            endpoint,
            operation,
            CaddyAdminError::EndpointUnavailable {
                endpoint: endpoint.clone(),
                reason,
            },
        )
    })?;
    let peer_pid = Pid::from_raw(peer_pid).ok_or_else(|| {
        request_not_sent(
            endpoint,
            operation,
            CaddyAdminError::EndpointUnavailable {
                endpoint: endpoint.clone(),
                reason: "peer process id must be positive".to_owned(),
            },
        )
    })?;
    let expected_process_group = getpgid(Some(expected_pid)).map_err(|error| {
        request_not_sent(
            endpoint,
            operation,
            CaddyAdminError::EndpointUnavailable {
                endpoint: endpoint.clone(),
                reason: format!("failed to inspect managed process group: {error}"),
            },
        )
    })?;
    let actual_process_group = getpgid(Some(peer_pid)).map_err(|error| {
        request_not_sent(
            endpoint,
            operation,
            CaddyAdminError::EndpointUnavailable {
                endpoint: endpoint.clone(),
                reason: format!("failed to inspect peer process group: {error}"),
            },
        )
    })?;

    if actual_process_group != expected_process_group {
        return Err(request_not_sent(
            endpoint,
            operation,
            CaddyAdminError::PeerProcessGroupMismatch {
                endpoint: endpoint.clone(),
                expected_process_group: expected_process_group.as_raw_pid(),
                actual_process_group: actual_process_group.as_raw_pid(),
            },
        ));
    }

    Ok(())
}

#[cfg(unix)]
fn process_pid(pid: u32) -> Result<Pid, String> {
    let raw_pid = i32::try_from(pid).map_err(|error| error.to_string())?;

    Pid::from_raw(raw_pid).ok_or_else(|| "process id must be positive".to_owned())
}

fn no_op_verifier() -> CaddyAdminVerifier {
    Arc::new(|_operation| Ok(None))
}

fn verify_before_request(
    verifier: &CaddyAdminVerifier,
    endpoint: &CaddyAdminEndpoint,
    operation: CaddyAdminOperation,
) -> Result<Option<u32>, CaddyAdminError> {
    verifier(operation).map_err(|reason| request_not_sent(endpoint, operation, reason))
}

fn is_verifier_failure(error: &CaddyAdminError) -> bool {
    matches!(
        error,
        CaddyAdminError::RequestNotSent { reason, .. }
            if matches!(
                reason.as_ref(),
                CaddyAdminError::RuntimeOwnershipChanged { .. }
                    | CaddyAdminError::PeerProcessGroupMismatch { .. }
                    | CaddyAdminError::TaskFailed { .. }
            )
    )
}

fn map_exchange_error(
    endpoint: &CaddyAdminEndpoint,
    operation: CaddyAdminOperation,
    error: hyper::Error,
) -> CaddyAdminError {
    if operation == CaddyAdminOperation::Load {
        CaddyAdminError::RequestOutcomeUnknown {
            endpoint: endpoint.clone(),
            operation,
            reason: error.to_string(),
        }
    } else {
        CaddyAdminError::EndpointUnavailable {
            endpoint: endpoint.clone(),
            reason: error.to_string(),
        }
    }
}

fn request_not_sent(
    endpoint: &CaddyAdminEndpoint,
    operation: CaddyAdminOperation,
    reason: CaddyAdminError,
) -> CaddyAdminError {
    CaddyAdminError::RequestNotSent {
        endpoint: endpoint.clone(),
        operation,
        reason: Box::new(reason),
    }
}

fn readiness_timeout_error(
    endpoint: &CaddyAdminEndpoint,
    timeout: Duration,
    last_error: Option<String>,
) -> CaddyAdminError {
    CaddyAdminError::AdminReadinessTimedOut {
        endpoint: endpoint.clone(),
        timeout_ms: timeout.as_millis(),
        last_error,
    }
}

fn readiness_error_detail(error: &CaddyAdminError) -> String {
    match error {
        CaddyAdminError::UnexpectedReadinessStatus { status, .. } => {
            format!("HTTP status {status}")
        }
        _ => error.to_string(),
    }
}

fn is_success(status: u16) -> bool {
    (200..300).contains(&status)
}

/// Loads a complete active Caddyfile through its Unix-domain admin socket.
pub async fn load_caddyfile(
    endpoint: &CaddyAdminEndpoint,
    bytes: impl AsRef<[u8]>,
) -> Result<(), CaddyAdminError> {
    CaddyAdminClient::default()
        .load_caddyfile(endpoint, bytes)
        .await
}

/// Waits for the Caddy admin API to expose its configuration endpoint.
pub async fn wait_until_ready(
    endpoint: &CaddyAdminEndpoint,
    timeout: Duration,
) -> Result<(), CaddyAdminError> {
    CaddyAdminClient::default()
        .wait_until_ready(endpoint, timeout)
        .await
}
