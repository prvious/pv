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
    RunJob { kind: String, scope: String },
    ManagedResourceUpdateCheck,
}
