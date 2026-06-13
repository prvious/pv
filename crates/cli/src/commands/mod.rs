use std::io::Write;
use std::process::ExitCode;

use camino::Utf8PathBuf;
use state::{PvPaths, StateError};

use crate::args::{Cli, Command};
use crate::environment::Environment;
use crate::error::{CliError, ExecuteError};
use crate::output::Output;

mod artifact_resource;
mod ca;
mod completions;
mod composer;
mod daemon;
mod dns;
mod doctor;
mod env;
mod jobs;
mod logs;
mod mailpit;
mod mysql;
mod php;
mod ports;
mod postgres;
mod project;
mod redis;
mod rustfs;
mod setup;
mod status;
mod update;

pub(crate) fn execute(
    cli: Cli,
    environment: &impl Environment,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    require_no_update_in_progress(&cli.command, environment)?;

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
        Command::Status(args) => status::run(args, environment, stdout),
        Command::Logs(args) => logs::run(args, cli.no_color, environment, stdout),
        Command::Doctor(args) => doctor::run(args, environment, stdout),
        Command::Jobs(args) => jobs::run(args, environment, stdout),
        Command::Update(args) => update::run(args, environment, stdout, stderr),
        Command::InternalUpdateManagedResources => {
            update::run_managed_resource_continuation(environment, stdout)
        }
        Command::List(args) => project::list(args, environment, stdout),
        Command::PhpUse(args) => php::use_track(args, environment, stdout),
        Command::PhpInstall(args) => php::install(args, environment, stdout),
        Command::PhpUpdate => php::update(environment, stdout),
        Command::PhpUninstall(args) => php::uninstall(args, environment, stdout),
        Command::PhpList(args) => php::list(args, environment, stdout),
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
        Command::MailpitList(args) | Command::MailList(args) => {
            mailpit::list(args, environment, stdout)
        }
        Command::MailpitOpen | Command::MailOpen => mailpit::open(environment, stdout),
        Command::RedisInstall(args) => redis::install(args, environment, stdout),
        Command::RedisUpdate => redis::update(environment, stdout),
        Command::RedisUninstall(args) => redis::uninstall(args, environment, stdout),
        Command::RedisList(args) => redis::list(args, environment, stdout),
        Command::RustfsInstall(args) | Command::S3Install(args) => {
            rustfs::install(args, environment, stdout)
        }
        Command::RustfsUpdate | Command::S3Update => rustfs::update(environment, stdout),
        Command::RustfsUninstall(args) | Command::S3Uninstall(args) => {
            rustfs::uninstall(args, environment, stdout)
        }
        Command::RustfsList(args) | Command::S3List(args) => {
            rustfs::list(args, environment, stdout)
        }
        Command::RustfsOpen | Command::S3Open => rustfs::open(environment, stdout),
        Command::MysqlInstall(args) => mysql::install(args, environment, stdout),
        Command::MysqlUpdate => mysql::update(environment, stdout),
        Command::MysqlUninstall(args) => mysql::uninstall(args, environment, stdout),
        Command::MysqlList(args) => mysql::list(args, environment, stdout),
        Command::PostgresInstall(args) | Command::PgInstall(args) => {
            postgres::install(args, environment, stdout)
        }
        Command::PostgresUpdate | Command::PgUpdate => postgres::update(environment, stdout),
        Command::PostgresUninstall(args) | Command::PgUninstall(args) => {
            postgres::uninstall(args, environment, stdout)
        }
        Command::PostgresList(args) | Command::PgList(args) => {
            postgres::list(args, environment, stdout)
        }
    }
}

fn require_no_update_in_progress(
    command: &Command,
    environment: &impl Environment,
) -> Result<(), ExecuteError> {
    if !command_blocked_during_update(command) {
        return Ok(());
    }

    let paths = pv_paths(environment)?;
    state::UpdateLock::require_no_update_in_progress(&paths).map_err(update_lock_error)
}

fn command_blocked_during_update(command: &Command) -> bool {
    match command {
        Command::Update(args) => args.check,
        command => matches!(
            command,
            Command::Setup(_)
                | Command::Uninstall(_)
                | Command::InternalUpdateManagedResources
                | Command::DaemonEnable
                | Command::DaemonDisable
                | Command::DaemonRestart
                | Command::DnsInstall
                | Command::DnsUninstall
                | Command::PortsInstall
                | Command::PortsUninstall
                | Command::CaTrust
                | Command::CaUntrust
                | Command::Link(_)
                | Command::Unlink(_)
                | Command::PhpUse(_)
                | Command::PhpInstall(_)
                | Command::PhpUpdate
                | Command::PhpUninstall(_)
                | Command::ComposerInstall
                | Command::ComposerUpdate
                | Command::ComposerUninstall(_)
                | Command::MailpitInstall(_)
                | Command::MailInstall(_)
                | Command::MailpitUpdate
                | Command::MailUpdate
                | Command::MailpitUninstall(_)
                | Command::MailUninstall(_)
                | Command::RedisInstall(_)
                | Command::RedisUpdate
                | Command::RedisUninstall(_)
                | Command::RustfsInstall(_)
                | Command::S3Install(_)
                | Command::RustfsUpdate
                | Command::S3Update
                | Command::RustfsUninstall(_)
                | Command::S3Uninstall(_)
                | Command::MysqlInstall(_)
                | Command::MysqlUpdate
                | Command::MysqlUninstall(_)
                | Command::PostgresInstall(_)
                | Command::PgInstall(_)
                | Command::PostgresUpdate
                | Command::PgUpdate
                | Command::PostgresUninstall(_)
                | Command::PgUninstall(_)
        ),
    }
}

fn update_lock_error(error: StateError) -> ExecuteError {
    match error {
        StateError::UpdateInProgress { path } => CliError::UpdateInProgress {
            path: path.to_string(),
        }
        .into(),
        error => error.into(),
    }
}

fn pv_paths(environment: &impl Environment) -> Result<PvPaths, ExecuteError> {
    let home = environment.home_dir().ok_or(StateError::MissingHome)?;
    let home = Utf8PathBuf::from_path_buf(home).map_err(|path| StateError::NonUtf8Home { path })?;

    Ok(PvPaths::for_home(home))
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
