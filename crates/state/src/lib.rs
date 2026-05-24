use thiserror::Error;

#[derive(Debug, Error)]
#[error("state error")]
pub struct StateError;
