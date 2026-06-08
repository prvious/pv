use std::io;

use config::ConfigError;
use hickory_proto::ProtoError;
use hickory_proto::serialize::binary::DecodeError;
use protocol::ProtocolError;
use resources::{ManagedResourceCommandError, ResourcesError};
use serde_json::Error as JsonError;
use state::StateError;
use thiserror::Error;
use tokio::task::JoinError;
use tokio_util::codec::LinesCodecError;

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("daemon socket is already in use at {path}")]
    SocketInUse { path: String },

    #[error("daemon protocol JSON error: {0}")]
    Json(#[from] JsonError),

    #[error("daemon protocol frame error: {0}")]
    Frame(#[from] LinesCodecError),

    #[error(transparent)]
    Protocol(#[from] ProtocolError),

    #[error("daemon protocol error: daemon {phase} timed out")]
    ProtocolTimedOut { phase: &'static str },

    #[error("daemon protocol error: daemon protocol mismatch; run `pv daemon:restart`")]
    ProtocolMismatch { expected: u16, actual: u16 },

    #[error("daemon protocol error: {reason}")]
    UnexpectedProtocolResponse { reason: String },

    #[error("daemon protocol error: {message}")]
    DaemonRejected { message: String },

    #[error("DNS request decode error: {0}")]
    DnsDecode(#[from] DecodeError),

    #[error("DNS response encode error: {0}")]
    DnsEncode(#[from] ProtoError),

    #[error("DNS resolver failed to bind {protocol} on 127.0.0.1:{port}: {source}")]
    DnsBind {
        protocol: &'static str,
        port: u16,
        #[source]
        source: io::Error,
    },

    #[error("state error: {0}")]
    State(#[from] StateError),

    #[error("Project config error: {0}")]
    Config(#[from] ConfigError),

    #[error("Managed Resource error: {0}")]
    Resources(#[from] ResourcesError),

    #[error("Managed Resource command failed: {0}")]
    ManagedResourceCommand(#[from] ManagedResourceCommandError),

    #[error("daemon task failed: {0}")]
    Task(#[from] JoinError),

    #[error("process `{name}` started without an observable pid")]
    MissingProcessId { name: String },

    #[error("readiness check `{check}` timed out after {timeout_ms}ms; last error: {last_error:?}")]
    ReadinessTimedOut {
        check: String,
        timeout_ms: u128,
        last_error: Option<String>,
    },

    #[error("Managed Resource runtime `{resource}` is not supported yet")]
    UnsupportedManagedResourceRuntime { resource: String },

    #[error(
        "Managed Resource runtime `{resource}` track `{track}` is missing installed artifact path"
    )]
    ManagedResourceArtifactMissing { resource: String, track: String },

    #[error("Managed Resource runtime `{resource}` track `{track}` is missing port `{port}`")]
    ManagedResourcePortMissing {
        resource: String,
        track: String,
        port: String,
    },

    #[error("time formatting failed: {0}")]
    TimeFormat(#[from] time::error::Format),
}
