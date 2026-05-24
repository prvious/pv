use thiserror::Error;

#[derive(Debug, Error)]
#[error("daemon error")]
pub struct DaemonError;
