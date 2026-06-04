use futures_util::SinkExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::{Framed, LinesCodec, LinesCodecError};

pub const PROTOCOL_VERSION: u16 = 1;

const MAX_PROTOCOL_LINE_BYTES: usize = 64 * 1024;

pub type DaemonTransport<Stream> = Framed<Stream, LinesCodec>;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("daemon protocol JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("daemon protocol frame error: {0}")]
    Frame(#[from] LinesCodecError),
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DaemonRequest {
    pub protocol_version: u16,

    #[serde(flatten)]
    pub command: DaemonCommand,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum DaemonCommand {
    Health,
    RunJob { kind: String, scope: String },
}

#[derive(Debug, Serialize)]
pub struct DaemonResponse<'message> {
    #[serde(rename = "type")]
    pub line_type: &'static str,
    pub protocol_version: u16,
    pub status: ResponseStatus,
    pub message: &'message str,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<&'message str>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseStatus {
    Ok,
    Accepted,
    Error,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonEvent<'message> {
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

pub fn transport<Stream>(stream: Stream) -> DaemonTransport<Stream>
where
    Stream: AsyncRead + AsyncWrite,
{
    Framed::new(
        stream,
        LinesCodec::new_with_max_length(MAX_PROTOCOL_LINE_BYTES),
    )
}

pub async fn write_line<Stream>(
    transport: &mut DaemonTransport<Stream>,
    line: &impl Serialize,
) -> Result<(), ProtocolError>
where
    Stream: AsyncWrite + Unpin,
{
    let encoded = serde_json::to_string(line)?;

    transport.send(encoded).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use futures_util::StreamExt;
    use serde_json::json;
    use tokio::io::duplex;

    use super::{DaemonResponse, PROTOCOL_VERSION, ResponseStatus, transport, write_line};

    #[tokio::test]
    async fn transport_frames_generic_async_streams() -> anyhow::Result<()> {
        let (client, server) = duplex(1024);
        let mut writer = transport(client);
        let mut reader = transport(server);

        write_line(
            &mut writer,
            &DaemonResponse {
                line_type: "response",
                protocol_version: PROTOCOL_VERSION,
                status: ResponseStatus::Ok,
                message: "daemon healthy",
                job_id: None,
            },
        )
        .await?;

        let Some(line) = reader.next().await else {
            anyhow::bail!("reader closed before receiving a protocol line");
        };

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&line?)?,
            json!({
                "type": "response",
                "protocol_version": PROTOCOL_VERSION,
                "status": "ok",
                "message": "daemon healthy",
            })
        );

        Ok(())
    }
}
