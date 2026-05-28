use std::io::{ErrorKind, Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};

use camino::{Utf8Path, Utf8PathBuf};

use crate::{ResourcesError, Result};

const USER_ONLY_DIR_MODE: u32 = 0o700;
const CACHE_FILE_MODE: u32 = 0o600;
static TEMPORARY_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) fn read_to_string(path: &Utf8Path) -> Result<String> {
    read_utf8_file(path)
}

pub(crate) fn write_string_atomically(path: &Utf8Path, content: &str) -> Result<()> {
    write_bytes_atomically(path, content.as_bytes())
}

pub(crate) fn write_bytes_atomically(path: &Utf8Path, content: &[u8]) -> Result<()> {
    write_atomically_with(path, |writer| {
        writer
            .write_all(content)
            .map_err(|source| filesystem_error(path, source))
    })
}

pub(crate) fn write_atomically_with<T>(
    path: &Utf8Path,
    operation: impl FnOnce(&mut dyn Write) -> Result<T>,
) -> Result<T> {
    ensure_parent_dir(path)?;
    let temporary_path = temporary_path_for(path);
    let result = write_temporary_file(&temporary_path, operation);

    match result {
        Ok(value) => {
            if let Err(error) = rename(&temporary_path, path) {
                if let Err(_cleanup_error) = remove_file_if_exists(&temporary_path) {}

                return Err(error);
            }
            sync_parent_directory(path)?;

            Ok(value)
        }
        Err(error) => {
            if let Err(_cleanup_error) = remove_file_if_exists(&temporary_path) {}

            Err(error)
        }
    }
}

pub(crate) fn read_with<T>(
    path: &Utf8Path,
    operation: impl FnOnce(&mut dyn Read) -> Result<T>,
) -> Result<T> {
    let mut file = open_file(path)?;

    operation(&mut file)
}

pub(crate) fn remove_file_if_exists(path: &Utf8Path) -> Result<()> {
    match remove_file(path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == ErrorKind::NotFound => Ok(()),
        Err(source) => Err(filesystem_error(path, source)),
    }
}

pub(crate) fn path_exists(path: &Utf8Path) -> bool {
    path.exists()
}

pub(crate) fn path_entry_exists(path: &Utf8Path) -> Result<bool> {
    match symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(source) if source.kind() == ErrorKind::NotFound => Ok(false),
        Err(source) => Err(filesystem_error(path, source)),
    }
}

pub(crate) fn path_is_directory(path: &Utf8Path) -> Result<bool> {
    symlink_metadata(path)
        .map(|metadata| metadata.is_dir())
        .map_err(|source| filesystem_error(path, source))
}

fn ensure_parent_dir(path: &Utf8Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        create_dir_all(parent)?;
        set_dir_mode(parent, USER_ONLY_DIR_MODE)?;
    }

    Ok(())
}

fn temporary_path_for(path: &Utf8Path) -> Utf8PathBuf {
    let file_name = path.file_name().unwrap_or("pv");
    let process_id = std::process::id();
    let counter = TEMPORARY_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);

    path.with_file_name(format!("{file_name}.{process_id}.{counter}.tmp"))
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct file reads"
)]
fn read_utf8_file(path: &Utf8Path) -> Result<String> {
    std::fs::read_to_string(path).map_err(|source| filesystem_error(path, source))
}

fn write_temporary_file<T>(
    path: &Utf8Path,
    operation: impl FnOnce(&mut dyn Write) -> Result<T>,
) -> Result<T> {
    let mut file = create_file(path)?;
    set_file_mode(path, CACHE_FILE_MODE)?;
    let value = operation(&mut file)?;
    file.sync_all()
        .map_err(|source| filesystem_error(path, source))?;

    Ok(value)
}

fn sync_parent_directory(path: &Utf8Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        let directory = open_file(parent)?;
        directory
            .sync_all()
            .map_err(|source| filesystem_error(parent, source))?;
    }

    Ok(())
}

#[expect(
    clippy::disallowed_types,
    reason = "PV filesystem helper owns direct file handles"
)]
fn open_file(path: &Utf8Path) -> Result<std::fs::File> {
    std::fs::File::open(path).map_err(|source| filesystem_error(path, source))
}

#[expect(
    clippy::disallowed_types,
    reason = "PV filesystem helper owns direct file handles"
)]
fn create_file(path: &Utf8Path) -> Result<std::fs::File> {
    std::fs::File::create(path).map_err(|source| filesystem_error(path, source))
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct filesystem access"
)]
pub(crate) fn create_dir_all(path: &Utf8Path) -> Result<()> {
    std::fs::create_dir_all(path).map_err(|source| filesystem_error(path, source))
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct filesystem access"
)]
pub(crate) fn rename(from: &Utf8Path, to: &Utf8Path) -> Result<()> {
    std::fs::rename(from, to).map_err(|source| filesystem_error(to, source))
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct filesystem access"
)]
fn remove_file(path: &Utf8Path) -> std::io::Result<()> {
    std::fs::remove_file(path)
}

pub(crate) fn remove_dir_all_if_exists(path: &Utf8Path) -> Result<()> {
    match remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == ErrorKind::NotFound => Ok(()),
        Err(source) => Err(filesystem_error(path, source)),
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct filesystem access"
)]
fn remove_dir_all(path: &Utf8Path) -> std::io::Result<()> {
    std::fs::remove_dir_all(path)
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct filesystem access"
)]
fn symlink_metadata(path: &Utf8Path) -> std::io::Result<std::fs::Metadata> {
    std::fs::symlink_metadata(path)
}

pub(crate) fn sync_directory(path: &Utf8Path) -> Result<()> {
    let directory = open_file(path)?;
    directory
        .sync_all()
        .map_err(|source| filesystem_error(path, source))
}

#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct permission updates"
)]
fn set_file_mode(path: &Utf8Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let permissions = std::fs::Permissions::from_mode(mode);
    std::fs::set_permissions(path, permissions).map_err(|source| filesystem_error(path, source))
}

#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct permission updates"
)]
fn set_dir_mode(path: &Utf8Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let permissions = std::fs::Permissions::from_mode(mode);
    std::fs::set_permissions(path, permissions).map_err(|source| filesystem_error(path, source))
}

#[cfg(not(unix))]
fn set_file_mode(path: &Utf8Path, _mode: u32) -> Result<()> {
    Err(ResourcesError::Filesystem {
        path: path.to_string(),
        reason: "PV resources filesystem cache requires Unix permissions".to_string(),
    })
}

#[cfg(not(unix))]
fn set_dir_mode(path: &Utf8Path, _mode: u32) -> Result<()> {
    Err(ResourcesError::Filesystem {
        path: path.to_string(),
        reason: "PV resources filesystem cache requires Unix permissions".to_string(),
    })
}

fn filesystem_error(path: &Utf8Path, source: std::io::Error) -> ResourcesError {
    ResourcesError::Filesystem {
        path: path.to_string(),
        reason: source.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use camino::Utf8Path;
    use camino_tempfile::tempdir;

    use super::{create_dir_all, filesystem_error, write_atomically_with};

    #[test]
    fn write_atomically_with_removes_temporary_file_when_rename_fails() -> Result<()> {
        let tempdir = tempdir()?;
        let target = tempdir.path().join("manifest.json");
        create_dir_all(&target)?;

        let result = write_atomically_with(&target, |writer| {
            writer
                .write_all(b"manifest")
                .map_err(|source| filesystem_error(&target, source))
        });
        assert!(result.is_err());

        let file_names = sorted_file_names(tempdir.path())?;
        assert_eq!(file_names, vec!["manifest.json"]);

        Ok(())
    }

    fn sorted_file_names(path: &Utf8Path) -> Result<Vec<String>> {
        let mut file_names = path
            .read_dir_utf8()?
            .map(|entry| {
                entry
                    .map(|entry| entry.file_name().to_string())
                    .map_err(anyhow::Error::from)
            })
            .collect::<Result<Vec<_>>>()?;
        file_names.sort();

        Ok(file_names)
    }
}
