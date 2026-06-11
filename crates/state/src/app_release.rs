use std::sync::atomic::{AtomicU64, Ordering};

use camino::{Utf8Path, Utf8PathBuf};

use crate::{PvPaths, StateError, fs};

static APP_RELEASE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppReleaseLayout {
    paths: PvPaths,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppReleaseInstall {
    version: String,
    binary_path: Utf8PathBuf,
}

impl AppReleaseLayout {
    pub fn new(paths: PvPaths) -> Self {
        Self { paths }
    }

    pub fn install_release_binary(
        &self,
        version: &str,
        source: &Utf8Path,
    ) -> Result<AppReleaseInstall, StateError> {
        validate_app_release_version(version)?;
        let binary_path = self.paths.app_release_binary(version);

        fs::copy_file_atomically(source, &binary_path)?;
        fs::secure_executable_file(&binary_path)?;

        Ok(AppReleaseInstall {
            version: version.to_string(),
            binary_path,
        })
    }

    pub fn activate_release(&self, version: &str) -> Result<(), StateError> {
        validate_app_release_version(version)?;
        let release_binary = self.paths.app_release_binary(version);
        if !fs::path_is_file(&release_binary)? {
            return Err(StateError::AppReleaseMissing {
                version: version.to_string(),
                path: release_binary,
            });
        }

        let active_path = self.paths.active_pv_binary();
        let temporary_path = temporary_active_symlink(&active_path);
        fs::remove_file_if_exists(&temporary_path)?;
        fs::symlink_file(
            Utf8Path::new(&format!("releases/{version}/pv")),
            &temporary_path,
        )?;
        fs::rename(&temporary_path, &active_path)?;
        fs::sync_directory(self.paths.bin())?;

        Ok(())
    }

    pub fn active_release(&self) -> Result<Option<String>, StateError> {
        let active_path = self.paths.active_pv_binary();
        if !fs::path_entry_exists(&active_path)? {
            return Ok(None);
        }

        let target = fs::read_link(&active_path)?;
        let target_text = target.as_str();
        let Some(version) = target_text
            .strip_prefix("releases/")
            .and_then(|value| value.strip_suffix("/pv"))
        else {
            return Err(invalid_pointer(&active_path, target_text));
        };
        validate_app_release_version(version)?;

        let release_binary = self.paths.bin().join(target_text);
        if !fs::path_is_file(&release_binary)? {
            return Err(invalid_pointer(&active_path, target_text));
        }

        Ok(Some(version.to_string()))
    }
}

impl AppReleaseInstall {
    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn binary_path(&self) -> &Utf8Path {
        &self.binary_path
    }
}

fn temporary_active_symlink(active_path: &Utf8Path) -> Utf8PathBuf {
    let file_name = active_path.file_name().unwrap_or("pv");
    let process_id = std::process::id();
    let counter = APP_RELEASE_COUNTER.fetch_add(1, Ordering::Relaxed);

    active_path.with_file_name(format!("{file_name}.{process_id}.{counter}.tmp"))
}

fn validate_app_release_version(version: &str) -> Result<(), StateError> {
    if version.contains('/') || version.contains('\\') {
        return invalid_version(version);
    }

    let mut parts = version.split('.');
    let Some(major) = parts.next() else {
        return invalid_version(version);
    };
    let Some(minor) = parts.next() else {
        return invalid_version(version);
    };
    let Some(patch) = parts.next() else {
        return invalid_version(version);
    };
    if parts.next().is_some() {
        return invalid_version(version);
    }

    parse_version_component(major, version)?;
    parse_version_component(minor, version)?;
    parse_version_component(patch, version)?;

    Ok(())
}

fn parse_version_component(component: &str, version: &str) -> Result<(), StateError> {
    if component.is_empty() || component.len() > 1 && component.starts_with('0') {
        return invalid_version(version);
    }

    component
        .parse::<u64>()
        .map(|_component| ())
        .map_err(|_error| StateError::InvalidAppReleaseVersion {
            version: version.to_string(),
        })
}

fn invalid_version<T>(version: &str) -> Result<T, StateError> {
    Err(StateError::InvalidAppReleaseVersion {
        version: version.to_string(),
    })
}

fn invalid_pointer(path: &Utf8Path, target: &str) -> StateError {
    StateError::InvalidAppReleasePointer {
        path: path.to_path_buf(),
        target: target.to_string(),
    }
}
