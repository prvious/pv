use thiserror::Error;
use tokio_util::codec::LinesCodecError;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("daemon protocol JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("daemon protocol frame error: {0}")]
    Frame(#[from] LinesCodecError),
}
