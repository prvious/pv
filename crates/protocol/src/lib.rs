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
    ManagedResourceUpdateCheck,
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

    #[serde(skip_serializing_if = "Option::is_none")]
    update_check: Option<ManagedResourceUpdateCheck>,
}

impl DaemonResponse {
    pub fn ok(message: impl Into<String>) -> Self {
        Self::new(ResponseStatus::Ok, message, None, None)
    }

    pub fn accepted(message: impl Into<String>, job_id: impl Into<String>) -> Self {
        Self::new(ResponseStatus::Accepted, message, Some(job_id.into()), None)
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self::new(ResponseStatus::Error, message, None, None)
    }

    pub fn ok_update_check(
        message: impl Into<String>,
        update_check: ManagedResourceUpdateCheck,
    ) -> Self {
        Self::new(ResponseStatus::Ok, message, None, Some(update_check))
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

    pub fn update_check(&self) -> Option<&ManagedResourceUpdateCheck> {
        self.update_check.as_ref()
    }

    fn new(
        status: ResponseStatus,
        message: impl Into<String>,
        job_id: Option<String>,
        update_check: Option<ManagedResourceUpdateCheck>,
    ) -> Self {
        Self {
            line_type: RESPONSE_LINE_TYPE.to_string(),
            protocol_version: PROTOCOL_VERSION,
            status,
            message: message.into(),
            job_id,
            update_check,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ManagedResourceUpdateCheck {
    pub managed_resources: Vec<ManagedResourceUpdateCheckTrack>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ManagedResourceUpdateCheckTrack {
    pub status: ManagedResourceUpdateStatus,
    pub resource: String,
    pub track: String,
    pub current_artifact_version: String,
    pub current_artifact_path: String,
    pub latest_artifact_version: Option<String>,
    pub current_revocation: Option<ManagedResourceUpdateRevocation>,
    pub latest_revocation: Option<ManagedResourceUpdateRevocation>,
    pub blocked_by: Option<ManagedResourceUpdateBlocker>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ManagedResourceUpdateRevocation {
    pub artifact_version: String,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ManagedResourceUpdateBlocker {
    pub minimum_pv_version: String,
    pub current_pv_version: String,
}

impl std::fmt::Display for ManagedResourceUpdateBlocker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "requires PV {}, current PV {}",
            self.minimum_pv_version, self.current_pv_version
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedResourceUpdateStatus {
    Current,
    UpdateAvailable,
    Blocked,
    Revoked,
    Unavailable,
}

impl From<resources::ManagedResourceUpdateStatus> for ManagedResourceUpdateStatus {
    fn from(status: resources::ManagedResourceUpdateStatus) -> Self {
        match status {
            resources::ManagedResourceUpdateStatus::Current => Self::Current,
            resources::ManagedResourceUpdateStatus::UpdateAvailable => Self::UpdateAvailable,
            resources::ManagedResourceUpdateStatus::Blocked => Self::Blocked,
            resources::ManagedResourceUpdateStatus::Revoked => Self::Revoked,
            resources::ManagedResourceUpdateStatus::Unavailable => Self::Unavailable,
        }
    }
}

impl From<resources::ManagedResourceUpdateCheckTrack> for ManagedResourceUpdateCheckTrack {
    fn from(track: resources::ManagedResourceUpdateCheckTrack) -> Self {
        Self {
            status: track.status().into(),
            resource: track.resource_name().as_str().to_string(),
            track: track.track().as_str().to_string(),
            current_artifact_version: track.current_artifact_version().as_str().to_string(),
            current_artifact_path: track.current_artifact_path().to_string(),
            latest_artifact_version: track
                .latest_artifact_version()
                .map(|version| version.as_str().to_string()),
            current_revocation: track.current_revocation().map(Into::into),
            latest_revocation: track.latest_revocation().map(Into::into),
            blocked_by: track.blocked_by().map(Into::into),
            reason: track.reason().map(ToString::to_string),
        }
    }
}

impl From<&resources::ManagedResourceUpdateRevocation> for ManagedResourceUpdateRevocation {
    fn from(revocation: &resources::ManagedResourceUpdateRevocation) -> Self {
        Self {
            artifact_version: revocation.artifact_version().as_str().to_string(),
            reason: revocation.reason().to_string(),
        }
    }
}

impl From<&resources::ManagedResourceUpdateBlocker> for ManagedResourceUpdateBlocker {
    fn from(blocker: &resources::ManagedResourceUpdateBlocker) -> Self {
        Self {
            minimum_pv_version: blocker.minimum_pv_version().to_string(),
            current_pv_version: blocker.current_pv_version().to_string(),
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

    use super::{
        DaemonResponse, ManagedResourceUpdateCheck, ManagedResourceUpdateCheckTrack,
        ManagedResourceUpdateStatus, PROTOCOL_VERSION, ResponseStatus, transport, write_line,
    };

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

    #[test]
    fn update_check_response_round_trips_with_managed_resources() -> anyhow::Result<()> {
        let response = DaemonResponse::ok_update_check(
            "Managed Resource update check completed",
            ManagedResourceUpdateCheck {
                managed_resources: vec![ManagedResourceUpdateCheckTrack {
                    status: ManagedResourceUpdateStatus::UpdateAvailable,
                    resource: "redis".to_string(),
                    track: "8.8".to_string(),
                    current_artifact_version: "8.8.0-pv1".to_string(),
                    current_artifact_path: "/Users/me/.pv/resources/redis/8.8/releases/8.8.0-pv1"
                        .to_string(),
                    latest_artifact_version: Some("8.8.1-pv1".to_string()),
                    current_revocation: None,
                    latest_revocation: None,
                    blocked_by: None,
                    reason: None,
                }],
            },
        );
        let encoded = serde_json::to_value(&response)?;

        assert_eq!(
            encoded,
            json!({
                "type": "response",
                "protocol_version": PROTOCOL_VERSION,
                "status": "ok",
                "message": "Managed Resource update check completed",
                "update_check": {
                    "managed_resources": [
                        {
                            "status": "update_available",
                            "resource": "redis",
                            "track": "8.8",
                            "current_artifact_version": "8.8.0-pv1",
                            "current_artifact_path": "/Users/me/.pv/resources/redis/8.8/releases/8.8.0-pv1",
                            "latest_artifact_version": "8.8.1-pv1",
                            "current_revocation": null,
                            "latest_revocation": null,
                            "blocked_by": null,
                            "reason": null
                        }
                    ]
                }
            })
        );

        let decoded = serde_json::from_value::<DaemonResponse>(encoded)?;

        assert_eq!(decoded.status(), ResponseStatus::Ok);
        assert_eq!(
            decoded
                .update_check()
                .map(|check| check.managed_resources.len()),
            Some(1)
        );

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
