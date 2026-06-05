use futures_util::SinkExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::{Framed, LinesCodec, LinesCodecError};

pub const PROTOCOL_VERSION: u16 = 1;

const MAX_PROTOCOL_LINE_BYTES: usize = 64 * 1024;
const RESPONSE_LINE_TYPE: &str = "response";

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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DaemonResponse {
    #[serde(rename = "type")]
    line_type: String,
    protocol_version: u16,
    status: ResponseStatus,
    message: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    job_id: Option<String>,
}

impl DaemonResponse {
    pub fn ok(message: impl Into<String>) -> Self {
        Self::new(ResponseStatus::Ok, message, None)
    }

    pub fn accepted(message: impl Into<String>, job_id: impl Into<String>) -> Self {
        Self::new(ResponseStatus::Accepted, message, Some(job_id.into()))
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self::new(ResponseStatus::Error, message, None)
    }

    pub fn line_type(&self) -> &str {
        &self.line_type
    }

    pub fn protocol_version(&self) -> u16 {
        self.protocol_version
    }

    pub fn status(&self) -> ResponseStatus {
        self.status
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn job_id(&self) -> Option<&str> {
        self.job_id.as_deref()
    }

    fn new(status: ResponseStatus, message: impl Into<String>, job_id: Option<String>) -> Self {
        Self {
            line_type: RESPONSE_LINE_TYPE.to_string(),
            protocol_version: PROTOCOL_VERSION,
            status,
            message: message.into(),
            job_id,
        }
    }
}

#[derive(Copy, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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

    #[test]
    fn response_envelope_round_trips_through_protocol_type() -> anyhow::Result<()> {
        let response = DaemonResponse::accepted("job accepted", "job-1");
        let encoded = serde_json::to_value(&response)?;

        assert_eq!(
            encoded,
            json!({
                "type": "response",
                "protocol_version": PROTOCOL_VERSION,
                "status": "accepted",
                "message": "job accepted",
                "job_id": "job-1",
            })
        );

        let decoded = serde_json::from_value::<DaemonResponse>(encoded)?;

        assert_eq!(decoded.status(), ResponseStatus::Accepted);
        assert_eq!(decoded.message(), "job accepted");
        assert_eq!(decoded.job_id(), Some("job-1"));

        Ok(())
    }

    #[tokio::test]
    async fn transport_frames_generic_async_streams() -> anyhow::Result<()> {
        let (client, server) = duplex(1024);
        let mut writer = transport(client);
        let mut reader = transport(server);

        write_line(&mut writer, &DaemonResponse::ok("daemon healthy")).await?;

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
