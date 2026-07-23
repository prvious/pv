#[cfg(unix)]
use std::io;

use camino::Utf8Path;
#[cfg(unix)]
use rustix::fs::FlockOperation;

use crate::{PvPaths, StateError, fs};

#[derive(Debug)]
#[expect(
    clippy::disallowed_types,
    reason = "update lock guard owns the OS-locked file handle"
)]
pub struct UpdateLock {
    _file: std::fs::File,
}

impl UpdateLock {
    pub fn acquire(paths: &PvPaths) -> Result<Self, StateError> {
        require_file_locking()?;
        fs::ensure_user_dir(paths.run())?;
        let path = paths.update_lock();
        let file = open_update_lock_file(&path)?;
        fs::secure_sensitive_file(&path)?;

        lock_exclusively(&file, &path)?;

        Ok(Self { _file: file })
    }

    pub fn require_no_update_in_progress(paths: &PvPaths) -> Result<(), StateError> {
        let path = paths.update_lock();
        require_no_update_in_progress_at_path(&path)
    }
}

#[cfg(unix)]
fn require_no_update_in_progress_at_path(path: &Utf8Path) -> Result<(), StateError> {
    let file = match open_existing_update_lock_file(path) {
        Ok(file) => file,
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            return Ok(());
        }
        Err(error) => return Err(error),
    };

    inspect_existing_lock(&file, path)
}

#[cfg(not(unix))]
fn require_no_update_in_progress_at_path(path: &Utf8Path) -> Result<(), StateError> {
    if !fs::path_entry_exists(path)? {
        return Ok(());
    }

    require_file_locking()
}

#[cfg(unix)]
fn lock_exclusively<FileHandle: std::os::fd::AsFd>(
    file: &FileHandle,
    path: &Utf8Path,
) -> Result<(), StateError> {
    match rustix::fs::flock(file, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
            Err(StateError::UpdateInProgress {
                path: path.to_path_buf(),
            })
        }
        Err(error) => Err(StateError::filesystem(
            path.to_path_buf(),
            io::Error::from(error),
        )),
    }
}

#[cfg(not(unix))]
fn lock_exclusively<FileHandle>(_file: &FileHandle, _path: &Utf8Path) -> Result<(), StateError> {
    require_file_locking()
}

#[cfg(unix)]
fn inspect_existing_lock<FileHandle: std::os::fd::AsFd>(
    file: &FileHandle,
    path: &Utf8Path,
) -> Result<(), StateError> {
    match rustix::fs::flock(file, FlockOperation::NonBlockingLockShared) {
        Ok(()) => {
            rustix::fs::flock(file, FlockOperation::Unlock).map_err(|error| {
                StateError::filesystem(path.to_path_buf(), io::Error::from(error))
            })?;

            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
            Err(StateError::UpdateInProgress {
                path: path.to_path_buf(),
            })
        }
        Err(error) => Err(StateError::filesystem(
            path.to_path_buf(),
            io::Error::from(error),
        )),
    }
}

#[cfg(unix)]
const fn require_file_locking() -> Result<(), StateError> {
    Ok(())
}

#[cfg(not(unix))]
fn require_file_locking() -> Result<(), StateError> {
    Err(crate::error::unsupported_current_target(
        crate::StateCapability::FileLocking,
    ))
}

#[expect(
    clippy::disallowed_types,
    reason = "update lock helper owns direct file handles for OS locking"
)]
fn open_update_lock_file(path: &Utf8Path) -> Result<std::fs::File, StateError> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|source| StateError::filesystem(path.to_path_buf(), source))
}

#[cfg(unix)]
#[expect(
    clippy::disallowed_types,
    reason = "update lock helper owns direct file handles for OS lock inspection"
)]
fn open_existing_update_lock_file(path: &Utf8Path) -> Result<std::fs::File, StateError> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(false)
        .truncate(false)
        .open(path)
        .map_err(|source| StateError::filesystem(path.to_path_buf(), source))
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use camino::Utf8Path;
    #[cfg(windows)]
    use camino_tempfile::tempdir;

    #[cfg(windows)]
    use super::UpdateLock;
    #[cfg(windows)]
    use crate::fs::path_entry_exists;
    #[cfg(windows)]
    use crate::{PvPaths, StateCapability, StateError};

    #[cfg(windows)]
    #[test]
    fn unsupported_acquire_does_not_create_run_directory_or_lock_file() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));

        assert!(!path_entry_exists(paths.run())?);
        assert!(!path_entry_exists(&paths.update_lock())?);
        let result = UpdateLock::acquire(&paths);

        assert!(matches!(
            result,
            Err(StateError::UnsupportedPlatform {
                capability: StateCapability::FileLocking,
                target: "windows",
            })
        ));
        assert!(!path_entry_exists(paths.run())?);
        assert!(!path_entry_exists(&paths.update_lock())?);

        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn missing_lock_path_does_not_require_locking_support() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));

        assert!(!path_entry_exists(&paths.update_lock())?);
        assert!(UpdateLock::require_no_update_in_progress(&paths).is_ok());
        assert!(!path_entry_exists(paths.run())?);
        assert!(!path_entry_exists(&paths.update_lock())?);

        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn existing_lock_path_requires_file_locking_support() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let lock_path = paths.update_lock();
        create_lock_fixture(&lock_path)?;

        assert!(path_entry_exists(&lock_path)?);
        let result = UpdateLock::require_no_update_in_progress(&paths);

        assert!(matches!(
            result,
            Err(StateError::UnsupportedPlatform {
                capability: StateCapability::FileLocking,
                target: "windows",
            })
        ));
        assert!(path_entry_exists(&lock_path)?);

        Ok(())
    }

    #[cfg(windows)]
    #[expect(
        clippy::disallowed_methods,
        reason = "Windows update lock test creates an isolated existing-lock fixture"
    )]
    fn create_lock_fixture(path: &Utf8Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, [])?;

        Ok(())
    }
}
