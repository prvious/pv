use thiserror::Error;

#[derive(Debug, Error)]
#[error("Project config error")]
pub struct ConfigError;
