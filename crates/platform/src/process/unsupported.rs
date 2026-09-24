use crate::capability::unsupported;
use crate::{PlatformCapability, PlatformError, ProcessIdentity, ProcessStartIdentity};

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
