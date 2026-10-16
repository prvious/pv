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
pub fn daemon_registration_name(target: PlatformTarget) -> &'static str {
    match target {
        PlatformTarget::Linux => "systemd user service",
        _ => "LaunchAgent",
    }
}

pub use native::{
    DAEMON_REGISTRATION_FILE_NAME, DAEMON_REGISTRATION_LABEL, DaemonRegistrationConfig,
};

#[cfg(target_os = "linux")]
pub use native::{
    DaemonRegistrationStatus, daemon_registration_session_address, inspect_daemon_registration,
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

pub fn daemon_registration_path(home: &Path, config_home: Option<&Path>) -> PathBuf {
    let directory = if cfg!(target_os = "linux") {
        config_home
            .filter(|path| path.is_absolute())
            .map_or_else(|| home.join(".config"), Path::to_path_buf)
            .join("systemd/user")
    } else {
        home.join("Library/LaunchAgents")
    };
    directory.join(DAEMON_REGISTRATION_FILE_NAME)
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
    #[cfg(target_os = "linux")]
    native::require_owned_file(path)?;
    state::fs::write_sensitive_file(path, &config.render()?).map_err(registration_file_error)
}

pub fn remove_daemon_registration(
    path: &Utf8Path,
    #[cfg(target_os = "linux")] session_address: &str,
) -> Result<(), PlatformError> {
    #[cfg(target_os = "linux")]
    {
        require_capability_for(
            PlatformTarget::current()?,
            PlatformCapability::DaemonRegistration,
        )?;
        native::remove(path, session_address)
    }

    #[cfg(not(target_os = "linux"))]
    remove_daemon_registration_for(PlatformTarget::current()?, path)
}

#[cfg(any(not(target_os = "linux"), test))]
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

#[cfg(target_os = "linux")]
pub fn reload_daemon_registration(session_address: &str) -> Result<(), PlatformError> {
    native::reload(session_address)
}

pub fn enable_daemon_registration(
    path: &Utf8Path,
    #[cfg(target_os = "linux")] session_address: &str,
) -> Result<(), PlatformError> {
    #[cfg(target_os = "macos")]
    {
        native::enable(path)
    }

    #[cfg(target_os = "linux")]
    {
        native::enable(path, session_address)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = path;
        Err(unsupported(PlatformCapability::DaemonRegistration)?)
    }
}

pub fn stop_daemon_registration(
    #[cfg(target_os = "linux")] session_address: &str,
) -> Result<(), PlatformError> {
    #[cfg(target_os = "macos")]
    {
        native::stop()
    }

    #[cfg(target_os = "linux")]
    {
        native::stop(session_address)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err(unsupported(PlatformCapability::DaemonRegistration)?)
    }
}

pub fn restart_daemon_registration(
    #[cfg(target_os = "linux")] session_address: &str,
) -> Result<(), PlatformError> {
    #[cfg(target_os = "macos")]
    {
        native::restart()
    }

    #[cfg(target_os = "linux")]
    {
        native::restart(session_address)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err(unsupported(PlatformCapability::DaemonRegistration)?)
    }
}

#[cfg(test)]
mod tests {
    use camino::Utf8Path;
    use camino_tempfile::tempdir;

    use super::{
        DaemonRegistrationConfig, remove_daemon_registration_for,
        write_daemon_registration_file_for,
    };
    use crate::{PlatformCapability, PlatformError, PlatformTarget};

    #[test]
    fn unsupported_launch_agent_write_leaves_home_untouched() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let path = home.join("Library/LaunchAgents/com.prvious.pv.daemon.plist");
        let config = DaemonRegistrationConfig::for_paths(&state::PvPaths::for_home(&home));

        let result = write_daemon_registration_file_for(PlatformTarget::Windows, &path, &config);

        assert!(matches!(
            result,
            Err(PlatformError::Unsupported {
                capability: PlatformCapability::DaemonRegistration,
                target: PlatformTarget::Windows,
            })
        ));
        assert!(!home.exists());

        Ok(())
    }

    #[test]
    fn unsupported_launch_agent_removal_preserves_existing_file() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let path = tempdir.path().join("com.prvious.pv.daemon.plist");
        let original = "existing launch agent\n";
        write_test_file(&path, original)?;

        let result = remove_daemon_registration_for(PlatformTarget::Windows, &path);

        assert!(matches!(
            result,
            Err(PlatformError::Unsupported {
                capability: PlatformCapability::DaemonRegistration,
                target: PlatformTarget::Windows,
            })
        ));
        assert_eq!(state::fs::read_to_string(&path)?, original);

        Ok(())
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "platform tests seed portable LaunchAgent fixtures directly"
    )]
    fn write_test_file(path: &Utf8Path, content: &str) -> anyhow::Result<()> {
        std::fs::write(path, content)?;

        Ok(())
    }
}
