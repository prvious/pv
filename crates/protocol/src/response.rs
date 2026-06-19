use serde::{Deserialize, Serialize};

use crate::PROTOCOL_VERSION;
use crate::update_check::ManagedResourceUpdateCheck;

const RESPONSE_LINE_TYPE: &str = "response";

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

#[derive(Copy, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseStatus {
    Ok,
    Accepted,
    Error,
}
