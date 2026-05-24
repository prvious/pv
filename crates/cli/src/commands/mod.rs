use std::io::Write;
use std::process::ExitCode;

use crate::args::{Cli, Command};
use crate::environment::Environment;
use crate::error::ExecuteError;

mod completions;
mod env;
mod php;

pub(crate) fn execute(
    cli: Cli,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    match cli.command {
        Command::Env(args) => env::run(args, cli.no_color, environment, stdout),
        Command::Completions(args) => Ok(completions::run(args, stdout)),
        Command::PhpInstall(args) => php::install(args),
    }
}
