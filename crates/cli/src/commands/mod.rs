use std::io::Write;
use std::process::ExitCode;

use camino::Utf8PathBuf;
use platform::PlatformCapability;
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
mod init;
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
    execute_with_capability_check(cli, environment, stdout, stderr, require_command_capability)
}

fn execute_with_capability_check<CapabilityCheck>(
    cli: Cli,
    environment: &impl Environment,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
    capability_check: CapabilityCheck,
) -> Result<ExitCode, ExecuteError>
where
    CapabilityCheck: FnOnce(&Command) -> Result<(), ExecuteError>,
{
    capability_check(&cli.command)?;
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
        Command::Init(args) => init::run(args, environment, stdout),
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

fn require_command_capability(command: &Command) -> Result<(), ExecuteError> {
    if let Some(capability) = required_capability(command) {
        platform::require_capability(capability)?;
    }

    Ok(())
}

fn required_capability(command: &Command) -> Option<PlatformCapability> {
    match command {
        Command::Setup(_)
        | Command::Uninstall(_)
        | Command::DnsStatus
        | Command::DnsInstall
        | Command::DnsUninstall => Some(PlatformCapability::ResolverIntegration),
        Command::DaemonEnable
        | Command::DaemonDisable
        | Command::DaemonRestart
        | Command::Status(_)
        | Command::Doctor(_)
        | Command::Update(_)
        | Command::InternalUpdateManagedResources => Some(PlatformCapability::DaemonRegistration),
        Command::DaemonRun | Command::Link(_) | Command::Unlink(_) => {
            Some(PlatformCapability::DaemonIpc)
        }
        Command::PortsStatus | Command::PortsInstall | Command::PortsUninstall => {
            Some(PlatformCapability::LowPortFrontend)
        }
        Command::CaStatus | Command::CaTrust | Command::CaUntrust => {
            Some(PlatformCapability::TrustStore)
        }
        Command::Open(_)
        | Command::MailpitOpen
        | Command::MailOpen
        | Command::RustfsOpen
        | Command::S3Open => Some(PlatformCapability::BrowserHandoff),
        // These commands use portable CLI/state/config behavior or a lower crate's typed
        // filesystem boundary. They do not require a host-integration capability here.
        Command::Env(_)
        | Command::Completions(_)
        | Command::Init(_)
        | Command::ProjectEnv(_)
        | Command::Logs(_)
        | Command::Jobs(_)
        | Command::List(_) => None,
        // Shims perform portable local process dispatch over already-installed state. The
        // cross-platform launcher lifecycle remains an approved follow-up boundary.
        Command::ShimPhp(_) | Command::ShimComposer(_) => None,
        // Managed Resource handlers select a fallible artifact target before installation,
        // update, or removal. Functional non-macOS artifacts remain deliberately deferred.
        Command::PhpUse(_)
        | Command::PhpInstall(_)
        | Command::PhpUpdate
        | Command::PhpUninstall(_)
        | Command::PhpList(_)
        | Command::ComposerInstall
        | Command::ComposerUpdate
        | Command::ComposerUninstall(_)
        | Command::MailpitInstall(_)
        | Command::MailInstall(_)
        | Command::MailpitUpdate
        | Command::MailUpdate
        | Command::MailpitUninstall(_)
        | Command::MailUninstall(_)
        | Command::MailpitList(_)
        | Command::MailList(_)
        | Command::RedisInstall(_)
        | Command::RedisUpdate
        | Command::RedisUninstall(_)
        | Command::RedisList(_)
        | Command::RustfsInstall(_)
        | Command::S3Install(_)
        | Command::RustfsUpdate
        | Command::S3Update
        | Command::RustfsUninstall(_)
        | Command::S3Uninstall(_)
        | Command::RustfsList(_)
        | Command::S3List(_)
        | Command::MysqlInstall(_)
        | Command::MysqlUpdate
        | Command::MysqlUninstall(_)
        | Command::MysqlList(_)
        | Command::PostgresInstall(_)
        | Command::PgInstall(_)
        | Command::PostgresUpdate
        | Command::PgUpdate
        | Command::PostgresUninstall(_)
        | Command::PgUninstall(_)
        | Command::PostgresList(_)
        | Command::PgList(_) => None,
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::ffi::OsString;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::process::ExitCode;

    use platform::{PlatformCapability, PlatformError, PlatformTarget};

    use super::{execute_with_capability_check, required_capability};
    use crate::args::{
        Cli, Command, CompletionsArgs, DoctorArgs, LinkArgs, OpenArgs, SetupArgs, StatusArgs,
        UninstallArgs, UnlinkArgs, UpdateArgs,
    };
    use crate::environment::Environment;
    use crate::error::ExecuteError;
    use crate::shell::Shell;

    #[test]
    fn required_capability_maps_resolver_integration_commands() {
        assert_required_capability(
            &[
                Command::Setup(SetupArgs {
                    yes: false,
                    non_interactive: false,
                    no_path: false,
                }),
                Command::Uninstall(UninstallArgs {
                    prune: false,
                    force: false,
                }),
                Command::DnsStatus,
                Command::DnsInstall,
                Command::DnsUninstall,
            ],
            Some(PlatformCapability::ResolverIntegration),
        );
    }

    #[test]
    fn required_capability_maps_daemon_registration_commands() {
        assert_required_capability(
            &[
                Command::DaemonEnable,
                Command::DaemonDisable,
                Command::DaemonRestart,
                Command::Status(StatusArgs { json: false }),
                Command::Doctor(DoctorArgs {}),
                Command::Update(UpdateArgs {
                    check: false,
                    json: false,
                }),
            ],
            Some(PlatformCapability::DaemonRegistration),
        );
    }

    #[test]
    fn required_capability_maps_daemon_ipc_command() {
        assert_required_capability(
            &[
                Command::DaemonRun,
                Command::Link(LinkArgs {
                    path: None,
                    hostname: None,
                }),
                Command::Unlink(UnlinkArgs { hostname: None }),
            ],
            Some(PlatformCapability::DaemonIpc),
        );
    }

    #[test]
    fn required_capability_maps_internal_update_like_update() {
        assert_required_capability(
            &[Command::InternalUpdateManagedResources],
            Some(PlatformCapability::DaemonRegistration),
        );
    }

    #[test]
    fn required_capability_maps_low_port_frontend_commands() {
        assert_required_capability(
            &[
                Command::PortsStatus,
                Command::PortsInstall,
                Command::PortsUninstall,
            ],
            Some(PlatformCapability::LowPortFrontend),
        );
    }

    #[test]
    fn required_capability_maps_trust_store_commands() {
        assert_required_capability(
            &[Command::CaStatus, Command::CaTrust, Command::CaUntrust],
            Some(PlatformCapability::TrustStore),
        );
    }

    #[test]
    fn required_capability_maps_browser_handoff_commands() {
        assert_required_capability(
            &[
                Command::Open(OpenArgs { hostname: None }),
                Command::MailpitOpen,
                Command::MailOpen,
                Command::RustfsOpen,
                Command::S3Open,
            ],
            Some(PlatformCapability::BrowserHandoff),
        );
    }

    #[test]
    fn required_capability_leaves_completions_portable() {
        assert_required_capability(
            &[Command::Completions(CompletionsArgs { shell: Shell::Bash })],
            None,
        );
    }

    #[test]
    fn daemon_dependent_mutations_preflight_before_update_lock_tls_and_release_state() {
        assert_preflight_before_environment_access(
            Command::Link(LinkArgs {
                path: None,
                hostname: None,
            }),
            PlatformCapability::DaemonIpc,
        );
        assert_preflight_before_environment_access(
            Command::Unlink(UnlinkArgs { hostname: None }),
            PlatformCapability::DaemonIpc,
        );
        assert_preflight_before_environment_access(
            Command::InternalUpdateManagedResources,
            PlatformCapability::DaemonRegistration,
        );
    }

    fn assert_required_capability(commands: &[Command], expected: Option<PlatformCapability>) {
        for command in commands {
            assert_eq!(required_capability(command), expected);
        }
    }

    fn assert_preflight_before_environment_access(
        command: Command,
        capability: PlatformCapability,
    ) {
        let environment = AccessTrackingEnvironment::default();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let result = execute_with_capability_check(
            Cli {
                no_color: false,
                command,
            },
            &environment,
            &mut stdout,
            &mut stderr,
            |command| {
                if required_capability(command) != Some(capability) {
                    return Ok(());
                }

                Err(PlatformError::Unsupported {
                    capability,
                    target: PlatformTarget::Linux,
                }
                .into())
            },
        );

        assert!(matches!(
            result,
            Err(ExecuteError::Platform(PlatformError::Unsupported {
                capability: actual_capability,
                target: PlatformTarget::Linux,
            })) if actual_capability == capability
        ));
        assert!(!environment.accessed.get());
        assert!(stdout.is_empty());
        assert!(stderr.is_empty());
    }

    #[derive(Default)]
    struct AccessTrackingEnvironment {
        accessed: Cell<bool>,
    }

    impl AccessTrackingEnvironment {
        fn record_access(&self) {
            self.accessed.set(true);
        }
    }

    impl Environment for AccessTrackingEnvironment {
        fn var_os(&self, _key: &str) -> Option<OsString> {
            self.record_access();
            None
        }

        fn home_dir(&self) -> Option<PathBuf> {
            self.record_access();
            None
        }

        fn current_dir(&self) -> io::Result<PathBuf> {
            self.record_access();
            Err(io::Error::other("unexpected current directory access"))
        }

        fn current_exe(&self) -> io::Result<PathBuf> {
            self.record_access();
            Err(io::Error::other("unexpected executable access"))
        }

        fn stdin_is_terminal(&self) -> bool {
            self.record_access();
            false
        }

        fn read_line(&self) -> io::Result<String> {
            self.record_access();
            Err(io::Error::other("unexpected input access"))
        }

        fn open_url(&self, _url: &str) -> io::Result<()> {
            self.record_access();
            Err(io::Error::other("unexpected browser access"))
        }

        fn exec(&self, _program: &Path, _args: &[String]) -> io::Result<ExitCode> {
            self.record_access();
            Err(io::Error::other("unexpected process access"))
        }
    }
}
