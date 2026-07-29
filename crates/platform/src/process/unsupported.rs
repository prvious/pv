use crate::capability::unsupported;
use crate::{PlatformCapability, PlatformError, ProcessIdentity};

pub(super) fn inspect_process_identity(
    _pid: u32,
) -> Result<Option<ProcessIdentity>, PlatformError> {
    Err(unsupported(PlatformCapability::ProcessInspection)?)
}
