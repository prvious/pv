use std::io::Write;
use std::process::ExitCode;

use crate::args::{Cli, Command};
use crate::environment::Environment;
use crate::error::ExecuteError;

mod ca;
mod completions;
mod daemon;
mod dns;
mod env;
mod php;
mod ports;
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
        Command::DnsStatus => dns::status(environment, stdout),
        Command::DnsInstall => dns::install(environment, stdout),
        Command::DnsUninstall => dns::uninstall(environment, stdout),
        Command::PortsStatus => ports::status(environment, stdout),
        Command::PortsInstall => ports::install(environment, stdout),
        Command::PortsUninstall => ports::uninstall(environment, stdout),
        Command::CaStatus => ca::status(environment, stdout),
        Command::CaTrust => ca::trust(environment, stdout),
        Command::CaUntrust => ca::untrust(environment, stdout),
        Command::Link(args) => project::link(args, environment, stdout),
        Command::Unlink(args) => project::unlink(args, environment, stdout),
        Command::Open(args) => project::open(args, environment, stdout),
        Command::ProjectEnv(args) => project::env(args, environment, stdout, stderr),
        Command::List => project::list(environment, stdout),
        Command::PhpInstall(args) => php::install(args),
    }
}
