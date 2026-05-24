use std::process::ExitCode;

use crate::args::PhpInstallArgs;
use crate::error::{CliError, ExecuteError};

pub(crate) fn install(args: PhpInstallArgs) -> Result<ExitCode, ExecuteError> {
    let _track = args.track;

    Err(CliError::DeferredCommand {
        command: "php:install",
    }
    .into())
}
