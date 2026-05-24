use futures_util::SinkExt;
use serde::{Deserialize, Serialize};
use tokio::net::UnixStream;
use tokio_util::codec::{Framed, LinesCodec};

use crate::DaemonError;

pub const PROTOCOL_VERSION: u16 = 1;
const MAX_PROTOCOL_LINE_BYTES: usize = 64 * 1024;

pub(crate) type DaemonTransport = Framed<UnixStream, LinesCodec>;

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
    Log {
        job_id: &'message str,
        message: &'message str,
    },
    JobCompleted {
        job_id: &'message str,
        summary: &'message str,
    },
    JobFailed {
        job_id: &'message str,
        error: &'message str,
    },
}

pub(crate) fn transport(stream: UnixStream) -> DaemonTransport {
    Framed::new(
        stream,
        LinesCodec::new_with_max_length(MAX_PROTOCOL_LINE_BYTES),
    )
}

pub(crate) async fn write_line(
    transport: &mut DaemonTransport,
    line: &impl Serialize,
) -> Result<(), DaemonError> {
    let encoded = serde_json::to_string(line)?;

    transport.send(encoded).await?;

    Ok(())
}
