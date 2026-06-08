use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use camino::{Utf8Path, Utf8PathBuf};

use crate::{PvPaths, StateError, backup};

const USER_ONLY_DIR_MODE: u32 = 0o700;
const SENSITIVE_FILE_MODE: u32 = 0o600;
static TEMPORARY_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

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

pub fn write_sensitive_file(path: &Utf8Path, content: &str) -> Result<(), StateError> {
    ensure_parent_dir(path)?;
    write_atomically(path, content)?;
    secure_sensitive_file(path)
}

#[expect(
    clippy::disallowed_types,
    reason = "PV filesystem helper owns direct file handles"
)]
pub fn open_append_file(path: &Utf8Path) -> Result<std::fs::File, StateError> {
    ensure_parent_dir(path)?;
    let file = open_append_file_handle(path)?;
    secure_sensitive_file(path)?;

    Ok(file)
}

pub fn read_to_string(path: &Utf8Path) -> Result<String, StateError> {
    read_utf8_file(path)
}

pub fn modified_at(path: &Utf8Path) -> Result<Option<SystemTime>, StateError> {
    match file_modified_at(path) {
        Ok(modified_at) => Ok(Some(modified_at)),
        Err(StateError::Filesystem { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
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

pub fn ensure_user_dir(path: &Utf8Path) -> Result<(), StateError> {
    create_dir_all(path)?;
    set_dir_mode(path, USER_ONLY_DIR_MODE)?;
    validate_mode(path, USER_ONLY_DIR_MODE)?;
    validate_owner(path)
}

fn ensure_parent_dir(path: &Utf8Path) -> Result<(), StateError> {
    if let Some(parent) = path.parent() {
        ensure_user_dir(parent)?;
    }

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns atomic file writes"
)]
fn write_atomically(path: &Utf8Path, content: &str) -> Result<(), StateError> {
    let temporary_path = temporary_path_for(path);

    std::fs::write(&temporary_path, content)
        .map_err(|source| StateError::filesystem(temporary_path.clone(), source))?;
    secure_sensitive_file(&temporary_path)?;
    std::fs::rename(&temporary_path, path)
        .map_err(|source| StateError::filesystem(path.to_path_buf(), source))
}

fn temporary_path_for(path: &Utf8Path) -> Utf8PathBuf {
    let file_name = path.file_name().unwrap_or("pv");
    let process_id = std::process::id();
    let counter = TEMPORARY_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);

    path.with_file_name(format!("{file_name}.{process_id}.{counter}.tmp"))
}

#[expect(
    clippy::disallowed_types,
    reason = "PV filesystem helper owns direct file handles"
)]
fn open_append_file_handle(path: &Utf8Path) -> Result<std::fs::File, StateError> {
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|source| StateError::filesystem(path.to_path_buf(), source))
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct file reads"
)]
fn read_utf8_file(path: &Utf8Path) -> Result<String, StateError> {
    std::fs::read_to_string(path)
        .map_err(|source| StateError::filesystem(path.to_path_buf(), source))
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct file metadata reads"
)]
fn file_modified_at(path: &Utf8Path) -> Result<SystemTime, StateError> {
    let metadata = std::fs::metadata(path)
        .map_err(|source| StateError::filesystem(path.to_path_buf(), source))?;

    metadata
        .modified()
        .map_err(|source| StateError::filesystem(path.to_path_buf(), source))
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
pub fn remove_file(path: &Utf8Path) -> Result<(), StateError> {
    std::fs::remove_file(path).map_err(|source| StateError::filesystem(path.to_path_buf(), source))
}

pub fn delete_file(path: &Utf8Path) -> Result<(), StateError> {
    remove_file(path)
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct filesystem access"
)]
pub fn delete_dir_all(path: &Utf8Path) -> Result<(), StateError> {
    std::fs::remove_dir_all(path)
        .map_err(|source| StateError::filesystem(path.to_path_buf(), source))
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

#[cfg(test)]
mod tests {
    use camino::Utf8Path;

    use super::temporary_path_for;

    #[test]
    fn temporary_paths_keep_the_target_extension_in_the_derived_name() {
        let pid_temporary_path = temporary_path_for(Utf8Path::new("/tmp/pv/runtime.pid"));
        let metadata_temporary_path = temporary_path_for(Utf8Path::new("/tmp/pv/runtime.json"));

        assert_ne!(pid_temporary_path, metadata_temporary_path);
        assert!(
            pid_temporary_path
                .file_name()
                .is_some_and(|name| name.starts_with("runtime.pid."))
        );
        assert!(
            metadata_temporary_path
                .file_name()
                .is_some_and(|name| name.starts_with("runtime.json."))
        );
    }
}
