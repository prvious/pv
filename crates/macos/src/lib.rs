use thiserror::Error;

#[derive(Debug, Error)]
#[error("macOS integration error")]
pub struct MacosError;
