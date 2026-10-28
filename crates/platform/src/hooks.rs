//! Project hook environment capture and foreground/background command execution.

#[cfg(unix)]
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io;

use camino::Utf8Path;
use state::{PvPaths, StateError};
use thiserror::Error;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::{CAPTURE_TIMEOUT, capture_environment, login_shell, run_commands};
#[cfg(not(unix))]
mod unsupported;
#[cfg(not(unix))]
pub use unsupported::{capture_environment, login_shell, run_commands};

/// A run's transient environment. Deliberately has no debug or serialization implementation.
pub struct HookEnvironment {
    #[cfg(unix)]
    values: BTreeMap<OsString, OsString>,
}

#[derive(Clone, Copy)]
pub enum HookOutput<'a> {
    Terminal,
    Log(&'a Utf8Path),
}

pub struct HookCommandContext<'a> {
    pub paths: &'a PvPaths,
    pub project_path: &'a Utf8Path,
    pub source_path: Option<&'a Utf8Path>,
    pub environment: &'a HookEnvironment,
    pub output: HookOutput<'a>,
}

#[derive(Debug, Error)]
pub enum HookError {
    #[error("couldn't load your shell environment: {reason}{stderr}")]
    Environment { reason: String, stderr: String },
    #[error("hook command `{command}` exited with {status}")]
    CommandFailed { command: String, status: String },
    #[error("hook run interrupted")]
    Interrupted,
    #[error("hook process error: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    State(#[from] StateError),
    #[error("{original}; hook process-group cleanup failed: {cleanup}")]
    Cleanup {
        #[source]
        original: Box<HookError>,
        cleanup: io::Error,
    },
    #[error("hook process-group cleanup failed: {0}")]
    CleanupFailed(#[source] io::Error),
    #[error("Project hooks are unsupported on this platform")]
    Unsupported,
}

/// The Composer shim and Project hooks use the same home, cache and ordered PATH overlay.
pub fn composer_env_overlay(
    paths: &PvPaths,
    path: Option<&OsStr>,
) -> io::Result<Vec<(OsString, OsString)>> {
    let pv_bin = paths.bin().as_std_path().to_path_buf();
    let composer_bin = paths.composer().join("vendor/bin").into_std_path_buf();
    let mut entries = vec![pv_bin.clone(), composer_bin.clone()];
    if let Some(path) = path {
        entries.extend(
            std::env::split_paths(path).filter(|entry| entry != &pv_bin && entry != &composer_bin),
        );
    }
    let path = std::env::join_paths(entries)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    Ok(vec![
        (
            OsString::from("COMPOSER_HOME"),
            paths.composer().as_os_str().to_os_string(),
        ),
        (
            OsString::from("COMPOSER_CACHE_DIR"),
            paths
                .composer()
                .join("cache")
                .into_std_path_buf()
                .into_os_string(),
        ),
        (OsString::from("PATH"), path),
    ])
}
