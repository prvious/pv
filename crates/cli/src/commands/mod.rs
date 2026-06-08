use std::io::Write;
use std::process::ExitCode;

use crate::args::{Cli, Command};
use crate::environment::Environment;
use crate::error::ExecuteError;
use crate::output::Output;

mod artifact_resource;
mod ca;
mod completions;
mod composer;
mod daemon;
mod dns;
mod env;
mod mailpit;
mod php;
mod ports;
mod project;
mod redis;
mod rustfs;
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
        Command::ShimComposer(args) => composer::shim(args, environment),
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
        Command::ComposerInstall => composer::install(environment, stdout),
        Command::ComposerUpdate => composer::update(environment, stdout),
        Command::ComposerUninstall(args) => composer::uninstall(args, environment, stdout),
        Command::MailpitInstall(args) | Command::MailInstall(args) => {
            mailpit::install(args, environment, stdout)
        }
        Command::MailpitUpdate | Command::MailUpdate => mailpit::update(environment, stdout),
        Command::MailpitUninstall(args) | Command::MailUninstall(args) => {
            mailpit::uninstall(args, environment, stdout)
        }
        Command::MailpitList | Command::MailList => mailpit::list(environment, stdout),
        Command::MailpitOpen | Command::MailOpen => mailpit::open(environment, stdout),
        Command::RedisInstall(args) => redis::install(args, environment, stdout),
        Command::RedisUpdate => redis::update(environment, stdout),
        Command::RedisUninstall(args) => redis::uninstall(args, environment, stdout),
        Command::RedisList => redis::list(environment, stdout),
        Command::RustfsInstall(args) | Command::S3Install(args) => {
            rustfs::install(args, environment, stdout)
        }
        Command::RustfsUpdate | Command::S3Update => rustfs::update(environment, stdout),
        Command::RustfsUninstall(args) | Command::S3Uninstall(args) => {
            rustfs::uninstall(args, environment, stdout)
        }
        Command::RustfsList | Command::S3List => rustfs::list(environment, stdout),
        Command::RustfsOpen | Command::S3Open => rustfs::open(environment, stdout),
    }
}

fn write_revoked_latest_warnings(
    installs: &[resources::ManagedResourceInstall],
    output: &mut Output<'_, impl Write>,
) -> Result<(), ExecuteError> {
    for install in installs {
        write_revoked_latest_warning(install, output)?;
    }

    Ok(())
}

fn write_revoked_latest_warning(
    install: &resources::ManagedResourceInstall,
    output: &mut Output<'_, impl Write>,
) -> Result<(), ExecuteError> {
    let Some(revoked_latest) = install.revoked_latest() else {
        return Ok(());
    };

    output.line(&format!(
        "warning: newest {} artifact {} for track {} was revoked ({}); installed fallback {}",
        install.resource_name(),
        revoked_latest.artifact_version(),
        install.track(),
        revoked_latest.reason(),
        install.artifact_version(),
    ))?;

    Ok(())
}
