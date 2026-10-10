use crate::capability::unsupported;
use crate::{
    BootSessionId, PlatformCapability, PlatformError, ProcessIdentity, ProcessStartIdentity,
};

pub(super) fn current_boot_session_id() -> Result<BootSessionId, PlatformError> {
    Err(unsupported(PlatformCapability::ProcessInspection)?)
}

#[derive(Debug)]
pub struct ProcessExitWatch;

impl ProcessExitWatch {
    pub fn new(_pid: u32) -> Result<Self, PlatformError> {
        Err(unsupported(PlatformCapability::ProcessInspection)?)
    }

    pub fn try_exit_status(&mut self) -> Result<Option<std::process::ExitStatus>, PlatformError> {
        Err(unsupported(PlatformCapability::ProcessInspection)?)
    }

    pub fn with_exec(_pid: u32) -> Result<Self, PlatformError> {
        Err(unsupported(PlatformCapability::ProcessInspection)?)
    }

    pub fn try_events(&mut self) -> Result<crate::ProcessEvents, PlatformError> {
        Err(unsupported(PlatformCapability::ProcessInspection)?)
    }
}

pub(super) fn inspect_process_identity(
    _pid: u32,
) -> Result<Option<ProcessIdentity>, PlatformError> {
    Err(unsupported(PlatformCapability::ProcessInspection)?)
}

pub(super) fn inspect_process_start_identity(
    _pid: u32,
) -> Result<Option<ProcessStartIdentity>, PlatformError> {
    Err(unsupported(PlatformCapability::ProcessInspection)?)
}

pub(super) fn process_is_zombie(_pid: u32) -> Result<bool, PlatformError> {
    Err(unsupported(PlatformCapability::ProcessInspection)?)
}

pub(super) fn process_group_has_live_members(_process_group: u32) -> Result<bool, PlatformError> {
    Err(unsupported(PlatformCapability::ProcessInspection)?)
}
