use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(any(test, feature = "test-support"))]
use std::sync::{Mutex, mpsc::Sender};
use std::time::SystemTime;

use camino::{Utf8Path, Utf8PathBuf};

#[cfg(not(unix))]
use crate::StateCapability;
use crate::{PvPaths, StateError, backup};

const USER_ONLY_DIR_MODE: u32 = 0o700;
const SENSITIVE_FILE_MODE: u32 = 0o600;
const EXECUTABLE_FILE_MODE: u32 = 0o700;
static TEMPORARY_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[cfg(any(test, feature = "test-support"))]
struct DatabaseAuxiliaryHardeningTestHook {
    path: Utf8PathBuf,
    sender: Sender<()>,
}

#[cfg(any(test, feature = "test-support"))]
static DATABASE_AUXILIARY_HARDENING_TEST_HOOK: Mutex<Option<DatabaseAuxiliaryHardeningTestHook>> =
    Mutex::new(None);
#[cfg(any(test, feature = "test-support"))]
static SENSITIVE_WRITE_FAILURE_TEST_HOOK: Mutex<Option<Utf8PathBuf>> = Mutex::new(None);

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
    require_owner_only_filesystem()?;
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
    #[cfg(any(test, feature = "test-support"))]
    fail_sensitive_write_for_test(path)?;

    ensure_parent_dir(path)?;
    write_atomically(path, content)?;
    secure_sensitive_file(path)
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn fail_next_sensitive_write(path: Utf8PathBuf) {
    let mut hook = match SENSITIVE_WRITE_FAILURE_TEST_HOOK.lock() {
        Ok(hook) => hook,
        Err(poisoned) => poisoned.into_inner(),
    };
    *hook = Some(path);
}

#[cfg(any(test, feature = "test-support"))]
fn fail_sensitive_write_for_test(path: &Utf8Path) -> Result<(), StateError> {
    let should_fail = {
        let mut hook = match SENSITIVE_WRITE_FAILURE_TEST_HOOK.lock() {
            Ok(hook) => hook,
            Err(poisoned) => poisoned.into_inner(),
        };
        if hook.as_deref() == Some(path) {
            hook.take();
            true
        } else {
            false
        }
    };
    if should_fail {
        return Err(StateError::filesystem(
            path.to_path_buf(),
            io::Error::other("injected sensitive write failure"),
        ));
    }

    Ok(())
}

pub fn copy_file_atomically(source: &Utf8Path, target: &Utf8Path) -> Result<(), StateError> {
    ensure_parent_dir(target)?;
    let temporary_path = temporary_path_for(target);
    let result =
        copy_file(source, &temporary_path).and_then(|_bytes| rename(&temporary_path, target));

    match result {
        Ok(()) => {
            sync_parent_directory(target)?;
            Ok(())
        }
        Err(error) => {
            if let Err(_cleanup_error) = remove_file_if_exists(&temporary_path) {}

            Err(error)
        }
    }
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

#[expect(
    clippy::disallowed_types,
    reason = "PV filesystem helper owns direct file handles"
)]
pub fn create_new_file(path: &Utf8Path) -> Result<std::fs::File, StateError> {
    ensure_parent_dir(path)?;
    create_new_file_handle(path)
}

pub fn read_to_string(path: &Utf8Path) -> Result<String, StateError> {
    read_utf8_file(path)
}

pub fn read_dir_paths(path: &Utf8Path) -> Result<Vec<Utf8PathBuf>, StateError> {
    read_dir_utf8_paths(path)
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
    require_owner_only_filesystem()?;
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
    secure_sensitive_file(paths.db())?;

    for path in database_auxiliary_files(paths) {
        secure_database_auxiliary_file(&path)?;
    }

    Ok(())
}

fn secure_database_auxiliary_file(path: &Utf8Path) -> Result<(), StateError> {
    #[cfg(any(test, feature = "test-support"))]
    run_database_auxiliary_hardening_test_hook(path)?;

    match secure_sensitive_file(path) {
        Ok(()) => Ok(()),
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(())
        }
        Err(error) => Err(error),
    }
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn remove_database_auxiliary_file_before_hardening(
    path: Utf8PathBuf,
    sender: Sender<()>,
) {
    let mut hook = match DATABASE_AUXILIARY_HARDENING_TEST_HOOK.lock() {
        Ok(hook) => hook,
        Err(poisoned) => poisoned.into_inner(),
    };
    *hook = Some(DatabaseAuxiliaryHardeningTestHook { path, sender });
}

#[cfg(any(test, feature = "test-support"))]
fn run_database_auxiliary_hardening_test_hook(path: &Utf8Path) -> Result<(), StateError> {
    let hook = {
        let mut hook = match DATABASE_AUXILIARY_HARDENING_TEST_HOOK.lock() {
            Ok(hook) => hook,
            Err(poisoned) => poisoned.into_inner(),
        };
        if hook.as_ref().is_some_and(|hook| hook.path == path) {
            hook.take()
        } else {
            None
        }
    };

    if let Some(hook) = hook {
        remove_file(path)?;
        let _notification = hook.sender.send(());
    }

    Ok(())
}

pub(crate) fn secure_sensitive_file(path: &Utf8Path) -> Result<(), StateError> {
    require_owner_only_filesystem()?;
    set_file_mode(path, SENSITIVE_FILE_MODE)?;
    validate_mode(path, SENSITIVE_FILE_MODE)?;
    validate_owner(path)
}

pub(crate) fn secure_executable_file(path: &Utf8Path) -> Result<(), StateError> {
    require_owner_only_filesystem()?;
    set_file_mode(path, EXECUTABLE_FILE_MODE)?;
    validate_mode(path, EXECUTABLE_FILE_MODE)?;
    validate_owner(path)
}

fn database_files(paths: &PvPaths) -> [(&'static str, Utf8PathBuf); 3] {
    [
        ("database", paths.db().to_path_buf()),
        ("wal", paths.root().join("pv.db-wal")),
        ("shared_memory", paths.root().join("pv.db-shm")),
    ]
}

fn database_auxiliary_files(paths: &PvPaths) -> [Utf8PathBuf; 2] {
    [
        paths.root().join("pv.db-wal"),
        paths.root().join("pv.db-shm"),
    ]
}

pub fn ensure_user_dir(path: &Utf8Path) -> Result<(), StateError> {
    require_owner_only_filesystem()?;
    create_dir_all(path)?;
    set_dir_mode(path, USER_ONLY_DIR_MODE)?;
    validate_mode(path, USER_ONLY_DIR_MODE)?;
    validate_owner(path)
}

fn ensure_parent_dir(path: &Utf8Path) -> Result<(), StateError> {
    require_owner_only_filesystem()?;

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

#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns atomic file copies"
)]
fn copy_file(source: &Utf8Path, target: &Utf8Path) -> Result<u64, StateError> {
    std::fs::copy(source, target).map_err(|source| StateError::filesystem(target, source))
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
    clippy::disallowed_types,
    reason = "PV filesystem helper owns direct file handles"
)]
fn create_new_file_handle(path: &Utf8Path) -> Result<std::fs::File, StateError> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
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
pub fn rename(from: &Utf8Path, to: &Utf8Path) -> Result<(), StateError> {
    std::fs::rename(from, to).map_err(|source| StateError::filesystem(to.to_path_buf(), source))
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct filesystem access"
)]
pub fn remove_file(path: &Utf8Path) -> Result<(), StateError> {
    std::fs::remove_file(path).map_err(|source| StateError::filesystem(path.to_path_buf(), source))
}

pub fn remove_file_if_exists(path: &Utf8Path) -> Result<(), StateError> {
    match remove_file(path) {
        Ok(()) => Ok(()),
        Err(StateError::Filesystem { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            Ok(())
        }
        Err(error) => Err(error),
    }
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

#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct filesystem access"
)]
fn set_dir_mode(path: &Utf8Path, mode: u32) -> Result<(), StateError> {
    let permissions = std::fs::Permissions::from_mode(mode);
    std::fs::set_permissions(path, permissions)
        .map_err(|source| StateError::filesystem(path.to_path_buf(), source))
}

#[cfg(not(unix))]
fn set_dir_mode(_path: &Utf8Path, _mode: u32) -> Result<(), StateError> {
    require_owner_only_filesystem()
}

#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct filesystem access"
)]
fn set_file_mode(path: &Utf8Path, mode: u32) -> Result<(), StateError> {
    let permissions = std::fs::Permissions::from_mode(mode);
    std::fs::set_permissions(path, permissions)
        .map_err(|source| StateError::filesystem(path.to_path_buf(), source))
}

#[cfg(not(unix))]
fn set_file_mode(_path: &Utf8Path, _mode: u32) -> Result<(), StateError> {
    require_owner_only_filesystem()
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
    let expected = current_uid()?;
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
    Ok(owner_uid(path)? == current_uid()?)
}

#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct filesystem access"
)]
fn mode(path: &Utf8Path) -> Result<u32, StateError> {
    let metadata = std::fs::metadata(path)
        .map_err(|source| StateError::filesystem(path.to_path_buf(), source))?;

    Ok(metadata.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn mode(_path: &Utf8Path) -> Result<u32, StateError> {
    Err(crate::error::unsupported_current_target(
        StateCapability::OwnerOnlyFilesystem,
    ))
}

#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct filesystem access"
)]
fn owner_uid(path: &Utf8Path) -> Result<u32, StateError> {
    let metadata = std::fs::metadata(path)
        .map_err(|source| StateError::filesystem(path.to_path_buf(), source))?;

    Ok(metadata.uid())
}

#[cfg(not(unix))]
fn owner_uid(_path: &Utf8Path) -> Result<u32, StateError> {
    Err(crate::error::unsupported_current_target(
        StateCapability::OwnerOnlyFilesystem,
    ))
}

pub fn path_exists(path: &Utf8Path) -> bool {
    path.exists()
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct filesystem access"
)]
pub fn path_entry_exists(path: &Utf8Path) -> Result<bool, StateError> {
    match std::fs::symlink_metadata(path) {
        Ok(_metadata) => Ok(true),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(StateError::filesystem(path.to_path_buf(), source)),
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct filesystem access"
)]
pub fn path_is_file(path: &Utf8Path) -> Result<bool, StateError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.is_file()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(StateError::filesystem(path.to_path_buf(), source)),
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct filesystem access"
)]
pub fn path_is_directory(path: &Utf8Path) -> Result<bool, StateError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.is_dir()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(StateError::filesystem(path.to_path_buf(), source)),
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct filesystem access"
)]
pub fn read_link(path: &Utf8Path) -> Result<Utf8PathBuf, StateError> {
    let target = std::fs::read_link(path)
        .map_err(|source| StateError::filesystem(path.to_path_buf(), source))?;

    Utf8PathBuf::from_path_buf(target).map_err(|path| {
        StateError::filesystem(
            path.to_string_lossy().as_ref(),
            io::Error::new(
                io::ErrorKind::InvalidData,
                "symlink target is not valid UTF-8",
            ),
        )
    })
}

#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct symlink updates"
)]
pub fn symlink_file(target: &Utf8Path, link: &Utf8Path) -> Result<(), StateError> {
    std::os::unix::fs::symlink(target, link)
        .map_err(|source| StateError::filesystem(link.to_path_buf(), source))
}

#[cfg(not(unix))]
pub fn symlink_file(_target: &Utf8Path, _link: &Utf8Path) -> Result<(), StateError> {
    Err(crate::error::unsupported_current_target(
        StateCapability::SymbolicLinks,
    ))
}

pub fn sync_parent_directory(path: &Utf8Path) -> Result<(), StateError> {
    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }

    Ok(())
}

#[expect(
    clippy::disallowed_types,
    reason = "PV filesystem helper owns direct file handles"
)]
pub fn sync_directory(path: &Utf8Path) -> Result<(), StateError> {
    let directory = std::fs::File::open(path)
        .map_err(|source| StateError::filesystem(path.to_path_buf(), source))?;
    directory
        .sync_all()
        .map_err(|source| StateError::filesystem(path.to_path_buf(), source))
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct directory reads"
)]
fn read_dir_utf8_paths(path: &Utf8Path) -> Result<Vec<Utf8PathBuf>, StateError> {
    let entries = match std::fs::read_dir(path) {
        Ok(entries) => entries,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(StateError::filesystem(path.to_path_buf(), source)),
    };
    let mut paths = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|source| StateError::filesystem(path.to_path_buf(), source))?;
        let path = entry.path();
        let path = Utf8PathBuf::from_path_buf(path).map_err(|path| {
            StateError::filesystem(
                path.to_string_lossy().as_ref(),
                io::Error::new(io::ErrorKind::InvalidData, "path is not valid UTF-8"),
            )
        })?;
        paths.push(path);
    }

    Ok(paths)
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
fn current_uid() -> Result<u32, StateError> {
    Ok(rustix::process::getuid().as_raw())
}

#[cfg(not(unix))]
fn current_uid() -> Result<u32, StateError> {
    Err(crate::error::unsupported_current_target(
        StateCapability::OwnerOnlyFilesystem,
    ))
}

#[cfg(unix)]
const fn require_owner_only_filesystem() -> Result<(), StateError> {
    Ok(())
}

#[cfg(not(unix))]
fn require_owner_only_filesystem() -> Result<(), StateError> {
    Err(crate::error::unsupported_current_target(
        StateCapability::OwnerOnlyFilesystem,
    ))
}

#[cfg(test)]
mod tests {
    use camino::Utf8Path;
    use camino_tempfile::tempdir;

    use super::temporary_path_for;
    #[cfg(windows)]
    use super::{ensure_user_dir, path_exists};
    #[cfg(unix)]
    use super::{
        path_exists, secure_database_auxiliary_file, secure_database_files, write_sensitive_file,
    };
    #[cfg(unix)]
    use crate::PvPaths;
    #[cfg(windows)]
    use crate::StateCapability;
    use crate::StateError;

    #[cfg(unix)]
    #[test]
    fn missing_persistent_database_file_is_not_ignored() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));

        let result = secure_database_files(&paths);

        assert!(matches!(
            result,
            Err(StateError::Filesystem { path, source })
                if path == paths.db() && source.kind() == std::io::ErrorKind::NotFound
        ));

        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn disappearing_database_auxiliary_file_is_ignored() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let auxiliary_path = paths.root().join("pv.db-wal");
        write_sensitive_file(paths.db(), "")?;
        write_sensitive_file(&auxiliary_path, "")?;
        let removal =
            crate::testing::remove_database_auxiliary_file_before_hardening(auxiliary_path.clone());

        secure_database_files(&paths)?;

        removal.try_recv()?;
        assert!(!path_exists(&auxiliary_path));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn database_auxiliary_file_security_errors_are_preserved() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let blocking_file = paths.root().join("blocking-file");
        write_sensitive_file(&blocking_file, "")?;
        let auxiliary_path = blocking_file.join("pv.db-wal");

        let result = secure_database_auxiliary_file(&auxiliary_path);

        assert!(matches!(
            result,
            Err(StateError::Filesystem { path, source })
                if path == auxiliary_path && source.kind() == std::io::ErrorKind::NotADirectory
        ));

        Ok(())
    }

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

    #[cfg(windows)]
    #[test]
    fn unsupported_owner_only_directory_does_not_create_path() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let requested_path = tempdir.path().join("owner-only");

        assert!(!path_exists(&requested_path));
        let result = ensure_user_dir(&requested_path);

        assert!(matches!(
            result,
            Err(StateError::UnsupportedPlatform {
                capability: StateCapability::OwnerOnlyFilesystem,
                target: "windows",
            })
        ));
        assert!(!path_exists(&requested_path));

        Ok(())
    }
}
