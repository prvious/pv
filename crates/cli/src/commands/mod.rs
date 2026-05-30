use std::io::Write;
use std::process::ExitCode;

use crate::args::{Cli, Command};
use crate::environment::Environment;
use crate::error::ExecuteError;

mod completions;
mod daemon;
mod env;
mod php;
mod project;

pub(crate) fn execute(
    cli: Cli,
    environment: &impl Environment,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    match cli.command {
        Command::Env(args) => env::run(args, cli.no_color, environment, stdout),
        Command::Completions(args) => Ok(completions::run(args, stdout)),
        Command::DaemonEnable => daemon::enable(),
        Command::DaemonDisable => daemon::disable(),
        Command::DaemonRestart => daemon::restart(),
        Command::DaemonRun => daemon::run(),
        Command::Link(args) => project::link(args, environment, stdout),
        Command::Unlink(args) => project::unlink(args, environment, stdout),
        Command::Open(args) => project::open(args, environment, stdout),
        Command::ProjectEnv(args) => project::env(args, environment, stdout, stderr),
        Command::List => project::list(environment, stdout),
        Command::PhpInstall(args) => php::install(args),
    }
}
