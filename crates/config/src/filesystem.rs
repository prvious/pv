use std::io::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};

use camino::{Utf8Path, Utf8PathBuf};

#[cfg(not(unix))]
use crate::ConfigCapability;
use crate::ConfigError;

static TEMPORARY_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[expect(
    clippy::disallowed_methods,
    reason = "Project config discovery owns symlink-aware config file probing"
)]
pub(crate) fn path_present(path: &Utf8Path) -> Result<bool, ConfigError> {
    match std::fs::symlink_metadata(path) {
        Ok(_metadata) => Ok(true),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(ConfigError::Filesystem {
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "Project config parser owns canonical path validation"
)]
pub(crate) fn canonicalize_utf8(path: &Utf8Path) -> Result<Utf8PathBuf, ConfigError> {
    let path = std::fs::canonicalize(path).map_err(|source| ConfigError::Filesystem {
        path: path.to_path_buf(),
        source,
    })?;

    Utf8PathBuf::from_path_buf(path).map_err(|path| ConfigError::NonUtf8Path { path })
}

#[expect(
    clippy::disallowed_methods,
    reason = "Project config parser owns config file reads"
)]
pub(crate) fn read_to_string(path: &Utf8Path) -> Result<String, ConfigError> {
    std::fs::read_to_string(path).map_err(|source| ConfigError::Filesystem {
        path: path.to_path_buf(),
        source,
    })
}

#[expect(
    clippy::disallowed_methods,
    reason = "Project env writer owns optional env file reads"
)]
pub(crate) fn read_optional_to_string(path: &Utf8Path) -> Result<Option<String>, ConfigError> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(ConfigError::Filesystem {
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "Project config parser owns project root and document root validation"
)]
pub(crate) fn is_directory(path: &Utf8Path) -> Result<bool, ConfigError> {
    let metadata = std::fs::metadata(path).map_err(|source| ConfigError::Filesystem {
        path: path.to_path_buf(),
        source,
    })?;

    Ok(metadata.is_dir())
}

pub(crate) fn write_string_atomically_with_mode(
    path: &Utf8Path,
    content: &str,
    mode: u32,
) -> Result<(), ConfigError> {
    require_permission_preserving_write()?;
    let temporary_path = temporary_path_for(path);
    let result = write_temporary_file(&temporary_path, content, mode);

    match result {
        Ok(()) => {
            if let Err(error) = rename(&temporary_path, path) {
                if let Err(_cleanup_error) = remove_file_if_exists(&temporary_path) {}

                return Err(error);
            }

            sync_parent_directory(path)
        }
        Err(error) => {
            if let Err(_cleanup_error) = remove_file_if_exists(&temporary_path) {}

            Err(error)
        }
    }
}

#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "Project env writer owns file permission reads"
)]
pub(crate) fn file_mode(path: &Utf8Path) -> Result<u32, ConfigError> {
    let metadata = std::fs::metadata(path).map_err(|source| ConfigError::Filesystem {
        path: path.to_path_buf(),
        source,
    })?;

    Ok(metadata.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
pub(crate) fn file_mode(_path: &Utf8Path) -> Result<u32, ConfigError> {
    Err(crate::error::unsupported_current_target(
        ConfigCapability::PermissionPreservingWrite,
    ))
}

fn temporary_path_for(path: &Utf8Path) -> Utf8PathBuf {
    let file_name = path.file_name().unwrap_or("pv");
    let process_id = std::process::id();
    let counter = TEMPORARY_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);

    path.with_file_name(format!("{file_name}.{process_id}.{counter}.tmp"))
}

fn write_temporary_file(path: &Utf8Path, content: &str, mode: u32) -> Result<(), ConfigError> {
    let mut file = create_file_with_mode(path, mode)?;
    set_file_mode(path, mode)?;
    file.write_all(content.as_bytes())
        .map_err(|source| ConfigError::Filesystem {
            path: path.to_path_buf(),
            source,
        })?;
    file.sync_all().map_err(|source| ConfigError::Filesystem {
        path: path.to_path_buf(),
        source,
    })
}

fn sync_parent_directory(path: &Utf8Path) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        let directory = open_file(parent)?;
        directory
            .sync_all()
            .map_err(|source| ConfigError::Filesystem {
                path: parent.to_path_buf(),
                source,
            })?;
    }

    Ok(())
}

#[cfg(unix)]
#[expect(
    clippy::disallowed_types,
    reason = "Project env writer owns direct temporary file handles"
)]
fn create_file_with_mode(path: &Utf8Path, mode: u32) -> Result<std::fs::File, ConfigError> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)
        .map_err(|source| ConfigError::Filesystem {
            path: path.to_path_buf(),
            source,
        })
}

#[cfg(not(unix))]
#[expect(
    clippy::disallowed_types,
    reason = "Project env writer owns direct temporary file handles"
)]
fn create_file_with_mode(_path: &Utf8Path, _mode: u32) -> Result<std::fs::File, ConfigError> {
    Err(crate::error::unsupported_current_target(
        ConfigCapability::PermissionPreservingWrite,
    ))
}

#[expect(
    clippy::disallowed_types,
    reason = "Project env writer owns direct directory file handles for fsync"
)]
fn open_file(path: &Utf8Path) -> Result<std::fs::File, ConfigError> {
    std::fs::File::open(path).map_err(|source| ConfigError::Filesystem {
        path: path.to_path_buf(),
        source,
    })
}

#[expect(
    clippy::disallowed_methods,
    reason = "Project env writer owns atomic renames"
)]
fn rename(from: &Utf8Path, to: &Utf8Path) -> Result<(), ConfigError> {
    std::fs::rename(from, to).map_err(|source| ConfigError::Filesystem {
        path: to.to_path_buf(),
        source,
    })
}

fn remove_file_if_exists(path: &Utf8Path) -> Result<(), ConfigError> {
    match remove_file(path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(ConfigError::Filesystem {
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "Project env writer owns temporary file cleanup"
)]
fn remove_file(path: &Utf8Path) -> io::Result<()> {
    std::fs::remove_file(path)
}

#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "Project env writer owns file permission updates"
)]
fn set_file_mode(path: &Utf8Path, mode: u32) -> Result<(), ConfigError> {
    let permissions = std::fs::Permissions::from_mode(mode);
    std::fs::set_permissions(path, permissions).map_err(|source| ConfigError::Filesystem {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(not(unix))]
fn set_file_mode(_path: &Utf8Path, _mode: u32) -> Result<(), ConfigError> {
    require_permission_preserving_write()
}

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

#[cfg(unix)]
const fn require_permission_preserving_write() -> Result<(), ConfigError> {
    Ok(())
}

#[cfg(not(unix))]
fn require_permission_preserving_write() -> Result<(), ConfigError> {
    Err(crate::error::unsupported_current_target(
        ConfigCapability::PermissionPreservingWrite,
    ))
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use camino::Utf8Path;
    #[cfg(windows)]
    use camino_tempfile::tempdir;

    #[cfg(windows)]
    use super::{path_present, write_string_atomically_with_mode};
    #[cfg(windows)]
    use crate::{ConfigCapability, ConfigError};

    #[cfg(windows)]
    #[test]
    fn unsupported_permission_write_does_not_create_temporary_file() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let target_path = tempdir.path().join("pv.yml");

        assert!(!path_present(&target_path)?);
        assert!(directory_is_empty(tempdir.path())?);
        let result = write_string_atomically_with_mode(&target_path, "php: 8.4\n", 0o600);

        assert!(matches!(
            result,
            Err(ConfigError::UnsupportedPlatform {
                capability: ConfigCapability::PermissionPreservingWrite,
                target: "windows",
            })
        ));
        assert!(!path_present(&target_path)?);
        assert!(directory_is_empty(tempdir.path())?);

        Ok(())
    }

    #[cfg(windows)]
    #[expect(
        clippy::disallowed_methods,
        reason = "Windows filesystem policy test inspects its isolated temporary directory"
    )]
    fn directory_is_empty(path: &Utf8Path) -> anyhow::Result<bool> {
        let mut entries = std::fs::read_dir(path)?;

        Ok(entries.next().transpose()?.is_none())
    }
}
