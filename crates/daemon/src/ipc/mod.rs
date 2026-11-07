#[cfg(unix)]
mod unix;
#[cfg(not(unix))]
mod unsupported;

use platform::{PlatformCapability, PlatformError, PlatformTarget};

#[cfg(unix)]
pub(crate) use self::unix::{
    LocalListener, LocalStream, bind, connect, prepare_endpoint, remove_endpoint,
};
#[cfg(not(unix))]
pub(crate) use self::unsupported::{
    LocalListener, LocalStream, bind, connect, prepare_endpoint, remove_endpoint,
};

pub(crate) fn require_ipc_for(target: PlatformTarget) -> Result<(), crate::DaemonError> {
    match target {
        PlatformTarget::Macos | PlatformTarget::Linux => Ok(()),
        PlatformTarget::Windows => Err(PlatformError::Unsupported {
            capability: PlatformCapability::DaemonIpc,
            target,
        }
        .into()),
    }
}
