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
