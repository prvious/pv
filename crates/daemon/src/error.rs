use std::io;

use serde_json::Error as JsonError;
use state::StateError;
use thiserror::Error;
use tokio::task::JoinError;

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("daemon protocol JSON error: {0}")]
    Json(#[from] JsonError),

    #[error("state error: {0}")]
    State(#[from] StateError),

    #[error("daemon task failed: {0}")]
    Task(#[from] JoinError),
}
