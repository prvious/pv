#[cfg(target_os = "macos")]
mod unix;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod unsupported;

use platform::{PlatformCapability, PlatformError, PlatformTarget};

#[cfg(target_os = "macos")]
pub(crate) use self::unix::{
    LocalListener, LocalStream, bind, connect, prepare_endpoint, remove_endpoint,
};
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(crate) use self::unsupported::{
    LocalListener, LocalStream, bind, connect, prepare_endpoint, remove_endpoint,
};

pub(crate) fn require_ipc_for(target: PlatformTarget) -> Result<(), crate::DaemonError> {
    match target {
        PlatformTarget::Macos => Ok(()),
        PlatformTarget::Linux | PlatformTarget::Windows => Err(PlatformError::Unsupported {
            capability: PlatformCapability::DaemonIpc,
            target,
        }
        .into()),
    }
}
