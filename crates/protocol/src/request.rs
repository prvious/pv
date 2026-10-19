use serde::{Deserialize, Serialize};

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
    RunJob {
        kind: String,
        scope: String,
        /// The terminal owns setup for this Project even when the job coalesces or promotes.
        #[serde(default, skip_serializing_if = "is_false")]
        requested_by_link: bool,
    },
    ManagedResourceUpdateCheck,
}

fn is_false(value: &bool) -> bool {
    !value
}
