use std::io;
use std::path::{Path, PathBuf};

use camino::Utf8Path;
use tokio::sync::watch;

use super::{HookCommandContext, HookEnvironment, HookError};

pub fn login_shell() -> Result<PathBuf, HookError> {
    Err(HookError::Unsupported)
}

pub async fn capture_environment(
    _shell: &Path,
    _home: &Utf8Path,
    _project_path: &Utf8Path,
    _shutdown: &mut watch::Receiver<bool>,
) -> Result<HookEnvironment, HookError> {
    Err(HookError::Unsupported)
}

pub async fn run_commands(
    _context: &HookCommandContext<'_>,
    _phase: &str,
    _commands: &[String],
    _shutdown: &mut watch::Receiver<bool>,
    _announce: impl FnMut(&str) -> io::Result<()>,
) -> Result<(), HookError> {
    Err(HookError::Unsupported)
}
