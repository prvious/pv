use thiserror::Error;

#[derive(Debug, Error)]
#[error("Managed Resource error")]
pub struct ResourcesError;
