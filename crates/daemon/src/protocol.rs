use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;

use crate::DaemonError;

pub const PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Deserialize)]
pub(crate) struct DaemonRequest {
    pub(crate) protocol_version: u16,

    #[serde(flatten)]
    pub(crate) command: DaemonCommand,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub(crate) enum DaemonCommand {
    Health,
    RunJob { kind: String, scope: String },
}

#[derive(Debug, Serialize)]
pub(crate) struct DaemonResponse<'message> {
    #[serde(rename = "type")]
    pub(crate) line_type: &'static str,
    pub(crate) protocol_version: u16,
    pub(crate) status: ResponseStatus,
    pub(crate) message: &'message str,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) job_id: Option<&'message str>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResponseStatus {
    Ok,
    Accepted,
    Error,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum DaemonEvent<'message> {
    JobStarted {
        job_id: &'message str,
        kind: &'message str,
        scope: &'message str,
    },
    Progress {
        job_id: &'message str,
        message: &'message str,
    },
    JobCompleted {
        job_id: &'message str,
        summary: &'message str,
    },
}

pub(crate) async fn write_line(
    stream: &mut UnixStream,
    line: &impl Serialize,
) -> Result<(), DaemonError> {
    let encoded = serde_json::to_string(line)?;

    stream.write_all(encoded.as_bytes()).await?;
    stream.write_all(b"\n").await?;

    Ok(())
}
