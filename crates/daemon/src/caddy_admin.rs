use std::fmt;
use std::io::Read;
use std::sync::Arc;
use std::time::{Duration, Instant};

use thiserror::Error;

pub const LOOPBACK_HOST: &str = "127.0.0.1";
pub const MAX_RESPONSE_DETAIL_BYTES: usize = 4096;

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const DEFAULT_WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_OVERALL_TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// The loopback address and port for one Caddy admin endpoint.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CaddyAdminEndpoint {
    port: u16,
}

impl CaddyAdminEndpoint {
    pub const fn new(port: u16) -> Self {
        Self { port }
    }

    pub const fn host(self) -> &'static str {
        LOOPBACK_HOST
    }

    pub const fn port(self) -> u16 {
        self.port
    }

    pub fn url(self, path: &str) -> String {
        format!("http://{}:{}{}", self.host(), self.port, path)
    }
}

impl fmt::Display for CaddyAdminEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.host(), self.port)
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

    #[error("Caddy admin {operation} request to {endpoint} may have been accepted: {reason}")]
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

pub type CaddyAdminVerifier =
    Arc<dyn Fn(CaddyAdminOperation) -> Result<(), CaddyAdminError> + Send + Sync>;

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
        endpoint: CaddyAdminEndpoint,
        bytes: impl AsRef<[u8]>,
    ) -> Result<(), CaddyAdminError> {
        self.load_caddyfile_with(endpoint, bytes, no_op_verifier())
            .await
    }

    pub async fn load_caddyfile_with(
        self,
        endpoint: CaddyAdminEndpoint,
        bytes: impl AsRef<[u8]>,
        verifier: CaddyAdminVerifier,
    ) -> Result<(), CaddyAdminError> {
        let body = bytes.as_ref().to_vec();
        let operation = CaddyAdminOperation::Load;
        let timeout = self.timeouts.overall;
        let agent = self.agent_for(timeout);

        run_blocking(operation, move || {
            verify_before_request(&verifier, endpoint, operation)?;
            load_caddyfile_blocking(agent, endpoint, body, timeout)
        })
        .await
    }

    pub async fn wait_until_ready(
        self,
        endpoint: CaddyAdminEndpoint,
        readiness_timeout: Duration,
    ) -> Result<(), CaddyAdminError> {
        self.wait_until_ready_with(endpoint, readiness_timeout, no_op_verifier())
            .await
    }

    pub async fn wait_until_ready_with(
        self,
        endpoint: CaddyAdminEndpoint,
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
        endpoint: CaddyAdminEndpoint,
        timeout: Duration,
        verifier: CaddyAdminVerifier,
    ) -> Result<(), CaddyAdminError> {
        let operation = CaddyAdminOperation::Readiness;
        let agent = self.agent_for(timeout);

        run_blocking(operation, move || {
            verify_before_request(&verifier, endpoint, operation)?;
            get_config_blocking(agent, endpoint, timeout)
        })
        .await
    }

    fn agent_for(self, overall_timeout: Duration) -> ureq::Agent {
        let overall_timeout = overall_timeout.min(self.timeouts.overall);
        let connect_timeout = self.timeouts.connect.min(overall_timeout);
        let write_timeout = self.timeouts.write.min(overall_timeout);
        let read_timeout = self.timeouts.read.min(overall_timeout);

        ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_global(Some(overall_timeout))
            .timeout_resolve(Some(connect_timeout))
            .timeout_connect(Some(connect_timeout))
            .timeout_send_request(Some(write_timeout))
            .timeout_send_body(Some(write_timeout))
            .timeout_recv_response(Some(read_timeout))
            .timeout_recv_body(Some(read_timeout))
            .build()
            .into()
    }
}

fn no_op_verifier() -> CaddyAdminVerifier {
    Arc::new(|_operation| Ok(()))
}

fn verify_before_request(
    verifier: &CaddyAdminVerifier,
    endpoint: CaddyAdminEndpoint,
    operation: CaddyAdminOperation,
) -> Result<(), CaddyAdminError> {
    verifier(operation).map_err(|reason| CaddyAdminError::RequestNotSent {
        endpoint,
        operation,
        reason: Box::new(reason),
    })
}

fn is_verifier_failure(error: &CaddyAdminError) -> bool {
    matches!(
        error,
        CaddyAdminError::RequestNotSent { reason, .. }
            if matches!(
                reason.as_ref(),
                CaddyAdminError::RuntimeOwnershipChanged { .. }
                    | CaddyAdminError::TaskFailed { .. }
            )
    )
}

/// Loads a complete active Caddyfile through the loopback admin API.
pub async fn load_caddyfile(
    endpoint: CaddyAdminEndpoint,
    bytes: impl AsRef<[u8]>,
) -> Result<(), CaddyAdminError> {
    CaddyAdminClient::default()
        .load_caddyfile(endpoint, bytes)
        .await
}

/// Waits for the Caddy admin API to expose its configuration endpoint.
pub async fn wait_until_ready(
    endpoint: CaddyAdminEndpoint,
    timeout: Duration,
) -> Result<(), CaddyAdminError> {
    CaddyAdminClient::default()
        .wait_until_ready(endpoint, timeout)
        .await
}

async fn run_blocking<T, Operation>(
    operation: CaddyAdminOperation,
    task: Operation,
) -> Result<T, CaddyAdminError>
where
    T: Send + 'static,
    Operation: FnOnce() -> Result<T, CaddyAdminError> + Send + 'static,
{
    match tokio::task::spawn_blocking(task).await {
        Ok(result) => result,
        Err(error) => Err(CaddyAdminError::TaskFailed {
            operation,
            reason: error.to_string(),
        }),
    }
}

fn load_caddyfile_blocking(
    agent: ureq::Agent,
    endpoint: CaddyAdminEndpoint,
    bytes: Vec<u8>,
    timeout: Duration,
) -> Result<(), CaddyAdminError> {
    let mut response = agent
        .post(endpoint.url("/load"))
        .content_type("text/caddyfile")
        .send(bytes)
        .map_err(|source| {
            map_request_error(endpoint, CaddyAdminOperation::Load, timeout, source)
        })?;
    let status = response.status().as_u16();

    if is_success(status) {
        return Ok(());
    }

    let detail = bounded_response_detail(&mut response);
    Err(CaddyAdminError::LoadRejected {
        endpoint,
        status,
        detail,
    })
}

fn get_config_blocking(
    agent: ureq::Agent,
    endpoint: CaddyAdminEndpoint,
    timeout: Duration,
) -> Result<(), CaddyAdminError> {
    let response = agent
        .get(endpoint.url("/config/"))
        .call()
        .map_err(|source| {
            map_request_error(endpoint, CaddyAdminOperation::Readiness, timeout, source)
        })?;
    let status = response.status().as_u16();

    if is_success(status) {
        Ok(())
    } else {
        Err(CaddyAdminError::UnexpectedReadinessStatus { endpoint, status })
    }
}

fn map_request_error(
    endpoint: CaddyAdminEndpoint,
    operation: CaddyAdminOperation,
    timeout: Duration,
    source: ureq::Error,
) -> CaddyAdminError {
    let reason = source.to_string();
    match source {
        ureq::Error::Timeout(ureq::Timeout::Resolve | ureq::Timeout::Connect) => request_not_sent(
            endpoint,
            operation,
            CaddyAdminError::RequestTimedOut {
                endpoint,
                operation,
                timeout_ms: timeout.as_millis(),
            },
        ),
        ureq::Error::Timeout(timeout_reason) if operation == CaddyAdminOperation::Load => {
            CaddyAdminError::RequestOutcomeUnknown {
                endpoint,
                operation,
                reason: format!("request timed out during {timeout_reason:?}"),
            }
        }
        ureq::Error::Timeout(_) => CaddyAdminError::RequestTimedOut {
            endpoint,
            operation,
            timeout_ms: timeout.as_millis(),
        },
        ureq::Error::Io(error) if error.kind() == std::io::ErrorKind::TimedOut => {
            if operation == CaddyAdminOperation::Load {
                CaddyAdminError::RequestOutcomeUnknown {
                    endpoint,
                    operation,
                    reason: error.to_string(),
                }
            } else {
                CaddyAdminError::RequestTimedOut {
                    endpoint,
                    operation,
                    timeout_ms: timeout.as_millis(),
                }
            }
        }
        ureq::Error::ConnectionFailed | ureq::Error::HostNotFound => request_not_sent(
            endpoint,
            operation,
            CaddyAdminError::EndpointUnavailable { endpoint, reason },
        ),
        ureq::Error::Io(error) if operation == CaddyAdminOperation::Load => match error.kind() {
            std::io::ErrorKind::AddrNotAvailable
            | std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::NetworkDown
            | std::io::ErrorKind::NetworkUnreachable
            | std::io::ErrorKind::NotConnected
            | std::io::ErrorKind::HostUnreachable => request_not_sent(
                endpoint,
                operation,
                CaddyAdminError::EndpointUnavailable {
                    endpoint,
                    reason: error.to_string(),
                },
            ),
            _ => CaddyAdminError::RequestOutcomeUnknown {
                endpoint,
                operation,
                reason: error.to_string(),
            },
        },
        ureq::Error::Io(_) | ureq::Error::Tls(_) => request_not_sent(
            endpoint,
            operation,
            CaddyAdminError::EndpointUnavailable { endpoint, reason },
        ),
        _ if operation == CaddyAdminOperation::Load => CaddyAdminError::RequestOutcomeUnknown {
            endpoint,
            operation,
            reason,
        },
        _ => CaddyAdminError::EndpointUnavailable { endpoint, reason },
    }
}

fn request_not_sent(
    endpoint: CaddyAdminEndpoint,
    operation: CaddyAdminOperation,
    reason: CaddyAdminError,
) -> CaddyAdminError {
    CaddyAdminError::RequestNotSent {
        endpoint,
        operation,
        reason: Box::new(reason),
    }
}

fn bounded_response_detail(response: &mut ureq::http::Response<ureq::Body>) -> String {
    let read_limit = (MAX_RESPONSE_DETAIL_BYTES as u64).saturating_add(1);
    let mut reader = response.body_mut().as_reader().take(read_limit);
    let mut bytes = Vec::with_capacity(MAX_RESPONSE_DETAIL_BYTES.saturating_add(1));

    let Ok(_) = reader.read_to_end(&mut bytes) else {
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

fn readiness_timeout_error(
    endpoint: CaddyAdminEndpoint,
    timeout: Duration,
    last_error: Option<String>,
) -> CaddyAdminError {
    CaddyAdminError::AdminReadinessTimedOut {
        endpoint,
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
