use camino::{Utf8Path, Utf8PathBuf};

use crate::{PvPaths, StateError, backup};

const USER_ONLY_DIR_MODE: u32 = 0o700;
const SENSITIVE_FILE_MODE: u32 = 0o600;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LayoutInspection {
    pub name: &'static str,
    pub path: String,
    pub mode: String,
    pub owned_by_current_user: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DatabaseFileInspection {
    pub name: &'static str,
    pub path: String,
    pub mode: String,
    pub owned_by_current_user: bool,
}

pub fn ensure_layout(paths: &PvPaths) -> Result<(), StateError> {
    for (_, directory) in paths.layout_directories() {
        ensure_user_dir(directory)?;
    }

    Ok(())
}

pub fn inspect_layout(paths: &PvPaths) -> Result<Vec<LayoutInspection>, StateError> {
    let mut entries = Vec::new();

    for (name, directory) in paths.layout_directories() {
        let mode = mode(directory)?;
        entries.push(LayoutInspection {
            name,
            path: display_path(paths, directory),
            mode: format!("{mode:o}"),
            owned_by_current_user: is_owned_by_current_user(directory)?,
        });
    }

    Ok(entries)
}

pub fn migration_backups(paths: &PvPaths) -> Result<Vec<String>, StateError> {
    backup::migration_backups(paths)
}

pub fn remove_daemon_socket(paths: &PvPaths) -> Result<(), StateError> {
    let path = paths.daemon_socket();

    if !path_exists(&path) {
        return Ok(());
    }

    remove_file(&path)
}

pub fn inspect_database_files(paths: &PvPaths) -> Result<Vec<DatabaseFileInspection>, StateError> {
    let mut entries = Vec::new();

    for (name, path) in database_files(paths) {
        if !path_exists(&path) {
            continue;
        }

        let mode = mode(&path)?;
        entries.push(DatabaseFileInspection {
            name,
            path: display_path(paths, &path),
            mode: format!("{mode:o}"),
            owned_by_current_user: is_owned_by_current_user(&path)?,
        });
    }

    Ok(entries)
}

pub(crate) fn database_exists(paths: &PvPaths) -> bool {
    path_exists(paths.db())
}

pub(crate) fn secure_database_files(paths: &PvPaths) -> Result<(), StateError> {
    for (_, path) in database_files(paths) {
        if !path_exists(&path) {
            continue;
        }

        secure_sensitive_file(&path)?;
    }

    Ok(())
}

pub(crate) fn secure_sensitive_file(path: &Utf8Path) -> Result<(), StateError> {
    set_file_mode(path, SENSITIVE_FILE_MODE)?;
    validate_mode(path, SENSITIVE_FILE_MODE)?;
    validate_owner(path)
}

fn database_files(paths: &PvPaths) -> [(&'static str, Utf8PathBuf); 3] {
    [
        ("database", paths.db().to_path_buf()),
        ("wal", paths.root().join("pv.db-wal")),
        ("shared_memory", paths.root().join("pv.db-shm")),
    ]
}

fn ensure_user_dir(path: &Utf8Path) -> Result<(), StateError> {
    create_dir_all(path)?;
    set_dir_mode(path, USER_ONLY_DIR_MODE)?;
    validate_mode(path, USER_ONLY_DIR_MODE)?;
    validate_owner(path)
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct filesystem access"
)]
fn create_dir_all(path: &Utf8Path) -> Result<(), StateError> {
    std::fs::create_dir_all(path)
        .map_err(|source| StateError::filesystem(path.to_path_buf(), source))
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct filesystem access"
)]
pub(crate) fn remove_file(path: &Utf8Path) -> Result<(), StateError> {
    std::fs::remove_file(path).map_err(|source| StateError::filesystem(path.to_path_buf(), source))
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct filesystem access"
)]
fn set_dir_mode(path: &Utf8Path, mode: u32) -> Result<(), StateError> {
    let permissions = std::fs::Permissions::from_mode(mode);
    std::fs::set_permissions(path, permissions)
        .map_err(|source| StateError::filesystem(path.to_path_buf(), source))
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct filesystem access"
)]
fn set_file_mode(path: &Utf8Path, mode: u32) -> Result<(), StateError> {
    let permissions = std::fs::Permissions::from_mode(mode);
    std::fs::set_permissions(path, permissions)
        .map_err(|source| StateError::filesystem(path.to_path_buf(), source))
}

fn validate_mode(path: &Utf8Path, expected: u32) -> Result<(), StateError> {
    let actual = mode(path)?;

    if actual == expected {
        return Ok(());
    }

    Err(StateError::UnsafePermissions {
        path: path.to_path_buf(),
        expected,
        actual,
    })
}

fn validate_owner(path: &Utf8Path) -> Result<(), StateError> {
    let expected = current_uid();
    let actual = owner_uid(path)?;

    if actual == expected {
        return Ok(());
    }

    Err(StateError::UnexpectedOwner {
        path: path.to_path_buf(),
        expected,
        actual,
    })
}

fn is_owned_by_current_user(path: &Utf8Path) -> Result<bool, StateError> {
    Ok(owner_uid(path)? == current_uid())
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct filesystem access"
)]
fn mode(path: &Utf8Path) -> Result<u32, StateError> {
    let metadata = std::fs::metadata(path)
        .map_err(|source| StateError::filesystem(path.to_path_buf(), source))?;

    Ok(metadata.permissions().mode() & 0o777)
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct filesystem access"
)]
fn owner_uid(path: &Utf8Path) -> Result<u32, StateError> {
    let metadata = std::fs::metadata(path)
        .map_err(|source| StateError::filesystem(path.to_path_buf(), source))?;

    Ok(metadata.uid())
}

pub(crate) fn path_exists(path: &Utf8Path) -> bool {
    path.exists()
}

fn display_path(paths: &PvPaths, path: &Utf8Path) -> String {
    if path == paths.root() {
        return "~/.pv".to_string();
    }

    match path.strip_prefix(paths.root()) {
        Ok(relative) => relative.to_string(),
        Err(_) => path.to_string(),
    }
}

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

#[cfg(unix)]
fn current_uid() -> u32 {
    rustix::process::getuid().as_raw()
}

#[cfg(not(unix))]
compile_error!("PV v1 targets macOS and requires Unix filesystem permissions");
