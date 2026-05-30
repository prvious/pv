use std::io;

use camino::{Utf8Path, Utf8PathBuf};

use crate::ConfigError;

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
    reason = "Project config parser owns project root and document root validation"
)]
pub(crate) fn is_directory(path: &Utf8Path) -> Result<bool, ConfigError> {
    let metadata = std::fs::metadata(path).map_err(|source| ConfigError::Filesystem {
        path: path.to_path_buf(),
        source,
    })?;

    Ok(metadata.is_dir())
}
