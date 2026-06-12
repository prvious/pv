use std::io;

use camino::Utf8Path;
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
        fs::ensure_user_dir(paths.run())?;
        let path = paths.update_lock();
        let file = open_update_lock_file(&path)?;
        fs::secure_sensitive_file(&path)?;

        match rustix::fs::flock(&file, FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => Ok(Self { _file: file }),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                Err(StateError::UpdateInProgress { path })
            }
            Err(error) => Err(StateError::filesystem(path, io::Error::from(error))),
        }
    }

    pub fn require_no_update_in_progress(paths: &PvPaths) -> Result<(), StateError> {
        let path = paths.update_lock();
        let file = match open_existing_update_lock_file(&path) {
            Ok(file) => file,
            Err(StateError::Filesystem { source, .. })
                if source.kind() == io::ErrorKind::NotFound =>
            {
                return Ok(());
            }
            Err(error) => return Err(error),
        };

        match rustix::fs::flock(&file, FlockOperation::NonBlockingLockShared) {
            Ok(()) => {
                rustix::fs::flock(&file, FlockOperation::Unlock).map_err(|error| {
                    StateError::filesystem(path.clone(), io::Error::from(error))
                })?;

                Ok(())
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                Err(StateError::UpdateInProgress { path })
            }
            Err(error) => Err(StateError::filesystem(path, io::Error::from(error))),
        }
    }
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
