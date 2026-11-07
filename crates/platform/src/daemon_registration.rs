use std::io;
use std::path::{Path, PathBuf};

use camino::{Utf8Path, Utf8PathBuf};

use crate::capability::require_capability_for;
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
use crate::capability::unsupported;
use crate::{PlatformCapability, PlatformError, PlatformTarget};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(not(target_os = "linux"))]
mod macos;

#[cfg(target_os = "linux")]
use self::linux as native;
#[cfg(not(target_os = "linux"))]
use self::macos as native;

use native::PV_MARKER;
pub const DAEMON_REGISTRATION_NAME: &str = if cfg!(target_os = "linux") {
    "systemd user service"
} else {
    "LaunchAgent"
};

pub use native::{
    DAEMON_REGISTRATION_FILE_NAME, DAEMON_REGISTRATION_LABEL, DaemonRegistrationConfig,
};

#[cfg(target_os = "linux")]
pub(crate) use native::check_user_manager;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DaemonRegistrationFileState {
    Missing {
        path: Utf8PathBuf,
    },
    Current {
        path: Utf8PathBuf,
        value: DaemonRegistrationConfig,
    },
    Stale {
        path: Utf8PathBuf,
        expected: Option<DaemonRegistrationConfig>,
        actual: Option<DaemonRegistrationConfig>,
    },
    Conflict {
        path: Utf8PathBuf,
    },
    Unreadable {
        path: Utf8PathBuf,
        message: String,
    },
}

pub fn inspect_daemon_registration_file(
    path: &Utf8Path,
    expected: Option<&DaemonRegistrationConfig>,
) -> DaemonRegistrationFileState {
    let content = match state::fs::read_to_string(path) {
        Ok(content) => content,
        Err(state::StateError::Filesystem { source, .. })
            if source.kind() == io::ErrorKind::NotFound =>
        {
            return DaemonRegistrationFileState::Missing {
                path: path.to_path_buf(),
            };
        }
        Err(error) => {
            return DaemonRegistrationFileState::Unreadable {
                path: path.to_path_buf(),
                message: error.to_string(),
            };
        }
    };

    if !content.lines().any(|line| line.trim() == PV_MARKER) {
        return DaemonRegistrationFileState::Conflict {
            path: path.to_path_buf(),
        };
    }

    let actual = DaemonRegistrationConfig::parse(&content);

    match (expected, actual) {
        (Some(expected), Some(actual)) if expected == &actual => {
            DaemonRegistrationFileState::Current {
                path: path.to_path_buf(),
                value: actual,
            }
        }
        (Some(expected), actual) => DaemonRegistrationFileState::Stale {
            path: path.to_path_buf(),
            expected: Some(expected.clone()),
            actual,
        },
        (None, Some(actual)) => DaemonRegistrationFileState::Current {
            path: path.to_path_buf(),
            value: actual,
        },
        (None, None) => DaemonRegistrationFileState::Stale {
            path: path.to_path_buf(),
            expected: None,
            actual: None,
        },
    }
}

/// The registration file under `home`. On Linux this is the per-user unit directory systemd
/// searches unless its own login environment moves `XDG_CONFIG_HOME`, which a shell profile
/// can't do, so PV doesn't follow the shell's value.
pub fn daemon_registration_path(home: &Path) -> PathBuf {
    let directory = if cfg!(target_os = "linux") {
        ".config/systemd/user"
    } else {
        "Library/LaunchAgents"
    };
    home.join(directory).join(DAEMON_REGISTRATION_FILE_NAME)
}

/// Whether the user's service manager starts the registration at login. launchd loads every
/// LaunchAgent file; systemd also needs the unit's `default.target.wants` link. An inspection
/// error counts as enabled, so doctor never reports a problem it couldn't see.
pub fn daemon_registration_enabled(path: &Utf8Path) -> bool {
    !cfg!(target_os = "linux")
        || path.parent().is_none_or(|directory| {
            state::fs::path_entry_exists(
                &directory
                    .join("default.target.wants")
                    .join(DAEMON_REGISTRATION_FILE_NAME),
            )
            .unwrap_or(true)
        })
}

pub fn write_daemon_registration_file(
    path: &Utf8Path,
    config: &DaemonRegistrationConfig,
) -> Result<(), PlatformError> {
    write_daemon_registration_file_for(PlatformTarget::current()?, path, config)
}

fn write_daemon_registration_file_for(
    target: PlatformTarget,
    path: &Utf8Path,
    config: &DaemonRegistrationConfig,
) -> Result<(), PlatformError> {
    require_capability_for(target, PlatformCapability::DaemonRegistration)?;
    state::fs::write_sensitive_file(path, &config.render()?).map_err(registration_file_error)
}

pub fn remove_daemon_registration(path: &Utf8Path) -> Result<(), PlatformError> {
    #[cfg(target_os = "linux")]
    {
        require_capability_for(
            PlatformTarget::current()?,
            PlatformCapability::DaemonRegistration,
        )?;
        native::remove(path)
    }

    #[cfg(not(target_os = "linux"))]
    remove_daemon_registration_for(PlatformTarget::current()?, path)
}

#[cfg(not(target_os = "linux"))]
fn remove_daemon_registration_for(
    target: PlatformTarget,
    path: &Utf8Path,
) -> Result<(), PlatformError> {
    require_capability_for(target, PlatformCapability::DaemonRegistration)?;
    delete_registration_file(path)
}

fn delete_registration_file(path: &Utf8Path) -> Result<(), PlatformError> {
    match state::fs::delete_file(path) {
        Ok(()) => Ok(()),
        Err(state::StateError::Filesystem { source, .. })
            if source.kind() == io::ErrorKind::NotFound =>
        {
            Ok(())
        }
        Err(error) => Err(registration_file_error(error)),
    }
}

fn registration_file_error(error: state::StateError) -> PlatformError {
    #[cfg(target_os = "linux")]
    return PlatformError::DaemonRegistrationFile(error);

    #[cfg(not(target_os = "linux"))]
    PlatformError::LaunchAgent(error.to_string())
}

pub fn enable_daemon_registration(path: &Utf8Path) -> Result<(), PlatformError> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        native::enable(path)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = path;
        Err(unsupported(PlatformCapability::DaemonRegistration)?)
    }
}

pub fn stop_daemon_registration() -> Result<(), PlatformError> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        native::stop()
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err(unsupported(PlatformCapability::DaemonRegistration)?)
    }
}

pub fn restart_daemon_registration() -> Result<(), PlatformError> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        native::restart()
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err(unsupported(PlatformCapability::DaemonRegistration)?)
    }
}
