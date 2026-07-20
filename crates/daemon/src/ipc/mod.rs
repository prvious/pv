#[cfg(target_os = "macos")]
mod unix;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod unsupported;

#[cfg(any(test, target_os = "linux", target_os = "windows"))]
use platform::{PlatformCapability, PlatformError, PlatformTarget};

#[cfg(target_os = "macos")]
pub(crate) use self::unix::{
    LocalListener, LocalStream, bind, connect, prepare_endpoint, remove_endpoint,
};
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(crate) use self::unsupported::{
    LocalListener, LocalStream, bind, connect, prepare_endpoint, remove_endpoint,
};

#[cfg(any(test, target_os = "linux", target_os = "windows"))]
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

#[cfg(test)]
mod tests {
    use platform::{PlatformCapability, PlatformError, PlatformTarget};

    use super::require_ipc_for;
    use crate::DaemonError;

    #[test]
    fn ipc_support_rejects_linux() {
        assert!(matches!(
            require_ipc_for(PlatformTarget::Linux),
            Err(DaemonError::Platform(PlatformError::Unsupported {
                capability: PlatformCapability::DaemonIpc,
                target: PlatformTarget::Linux,
            }))
        ));
    }
}
