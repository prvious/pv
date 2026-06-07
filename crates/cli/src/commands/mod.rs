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
mod setup;

pub(crate) fn execute(
    cli: Cli,
    environment: &impl Environment,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    match cli.command {
        Command::Env(args) => env::run(args, cli.no_color, environment, stdout),
        Command::Completions(args) => Ok(completions::run(args, stdout)),
        Command::Setup(args) => setup::setup(args, environment, stdout),
        Command::Uninstall(args) => setup::uninstall(args, environment, stdout),
        Command::DaemonEnable => daemon::enable(environment, stdout),
        Command::DaemonDisable => daemon::disable(environment, stdout),
        Command::DaemonRestart => daemon::restart(environment, stdout),
        Command::DaemonRun => daemon::run(),
        Command::ShimPhp(args) => php::shim(args, environment),
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
        Command::PhpUse(args) => php::use_track(args, environment, stdout),
        Command::PhpInstall(args) => php::install(args, environment, stdout),
        Command::PhpUpdate => php::update(environment, stdout),
        Command::PhpUninstall(args) => php::uninstall(args, environment, stdout),
        Command::PhpList => php::list(environment, stdout),
    }
}
