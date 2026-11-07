use rustix::fs::{Access, FileType, access, stat};

use crate::PlatformError;
use crate::daemon_registration::check_user_manager;
use crate::helper::SUDO_EXECUTABLE;

mod resolved;

/// Checks that systemd-resolved is active and that `/etc/resolv.conf` sends lookups to it first.
pub fn check_linux_resolved_stub() -> Result<(), PlatformError> {
    resolved::check_stub()
}

/// Checks what Linux setup needs and a plain host may lack: sudo, certutil, the user's systemd
/// session and systemd-resolved.
pub fn check_linux_setup_host() -> Result<(), PlatformError> {
    require_executable(SUDO_EXECUTABLE).map_err(|error| {
        PlatformError::SystemIntegration(format!(
            "{error}; install the sudo package before running `pv setup`"
        ))
    })?;
    require_executable(crate::trust::linux::CERTUTIL).map_err(|error| {
        PlatformError::SystemIntegration(format!(
            "{error}; install libnss3-tools (Debian/Ubuntu) or nss-tools (Fedora) before running `pv setup`"
        ))
    })?;
    check_user_manager().map_err(|error| {
        PlatformError::SystemIntegration(format!(
            "{error}; run `pv setup` from a logged-in systemd user session"
        ))
    })?;
    resolved::check_active()
}

fn require_executable(path: &str) -> Result<(), PlatformError> {
    let metadata = stat(path).map_err(|error| {
        PlatformError::SystemIntegration(format!("could not inspect executable {path}: {error}"))
    })?;
    if FileType::from_raw_mode(metadata.st_mode) != FileType::RegularFile {
        return Err(PlatformError::SystemIntegration(format!(
            "required executable {path} is not a regular file"
        )));
    }
    access(path, Access::EXEC_OK).map_err(|error| {
        PlatformError::SystemIntegration(format!("could not execute {path}: {error}"))
    })
}
