use std::io;

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

    #[error("daemon protocol error: {0}")]
    Protocol(String),

    #[error("state error: {0}")]
    State(#[from] StateError),

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

    #[error("time formatting failed: {0}")]
    TimeFormat(#[from] time::error::Format),
}
