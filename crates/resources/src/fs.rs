use std::io::{Read, Write};
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
            set_file_mode(&temporary_path, CACHE_FILE_MODE)?;
            rename(&temporary_path, path)?;
            set_file_mode(path, CACHE_FILE_MODE)?;

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
    if !path.exists() {
        return Ok(());
    }

    remove_file(path)
}

pub(crate) fn path_exists(path: &Utf8Path) -> bool {
    path.exists()
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
    let value = operation(&mut file)?;
    file.flush()
        .map_err(|source| filesystem_error(path, source))?;

    Ok(value)
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
fn create_dir_all(path: &Utf8Path) -> Result<()> {
    std::fs::create_dir_all(path).map_err(|source| filesystem_error(path, source))
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct filesystem access"
)]
fn rename(from: &Utf8Path, to: &Utf8Path) -> Result<()> {
    std::fs::rename(from, to).map_err(|source| filesystem_error(to, source))
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct filesystem access"
)]
fn remove_file(path: &Utf8Path) -> Result<()> {
    std::fs::remove_file(path).map_err(|source| filesystem_error(path, source))
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
