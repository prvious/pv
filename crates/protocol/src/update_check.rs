use serde::{Deserialize, Serialize};

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
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
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
