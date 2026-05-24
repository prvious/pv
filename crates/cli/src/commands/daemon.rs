use std::process::ExitCode;

use state::PvPaths;

use crate::error::{CliError, ExecuteError};

pub(crate) fn enable() -> Result<ExitCode, ExecuteError> {
    Err(CliError::DeferredDaemonLifecycle {
        command: "daemon:enable",
    }
    .into())
}

pub(crate) fn disable() -> Result<ExitCode, ExecuteError> {
    Err(CliError::DeferredDaemonLifecycle {
        command: "daemon:disable",
    }
    .into())
}

pub(crate) fn restart() -> Result<ExitCode, ExecuteError> {
    Err(CliError::DeferredDaemonLifecycle {
        command: "daemon:restart",
    }
    .into())
}

pub(crate) fn run() -> Result<ExitCode, ExecuteError> {
    let paths = PvPaths::default_home()?;

    ::daemon::run_blocking(paths)?;

    Ok(ExitCode::SUCCESS)
}
