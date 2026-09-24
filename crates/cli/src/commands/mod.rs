use std::io;
use std::process::ExitCode;
use std::time::Duration;

use camino::Utf8PathBuf;
use platform::PlatformCapability;
use state::{PvPaths, RuntimeObservedStatus, StateError};

use crate::args::{Cli, Command};
use crate::environment::Environment;
use crate::error::{CliError, ExecuteError};
use crate::output::{Line, Mark, Output, Streams};

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
mod pf_diagnostics;
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
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    execute_with_capability_check(cli, environment, streams, require_command_capability)
}

fn execute_with_capability_check<CapabilityCheck>(
    cli: Cli,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
    capability_check: CapabilityCheck,
) -> Result<ExitCode, ExecuteError>
where
    CapabilityCheck: FnOnce(&Command) -> Result<(), ExecuteError>,
{
    capability_check(&cli.command)?;
    require_no_update_in_progress(&cli.command, environment)?;

    match cli.command {
        Command::Env(args) => env::run(args, environment, streams),
        Command::Completions(args) => Ok(completions::run(args, streams)),
        Command::Setup(args) => setup::setup(args, environment, streams),
        Command::Uninstall(args) => setup::uninstall(args, environment, streams),
        Command::DaemonEnable => daemon::enable(environment, streams),
        Command::DaemonDisable => daemon::disable(environment, streams),
        Command::DaemonRestart => daemon::restart(environment, streams),
        Command::DaemonRun => daemon::run(),
        Command::ShimPhp(args) => php::shim(args, environment),
        Command::ShimComposer(args) => composer::shim(args, environment),
        Command::DnsStatus => dns::status(environment, streams),
        Command::DnsInstall => dns::install(environment, streams),
        Command::DnsUninstall => dns::uninstall(environment, streams),
        Command::PortsStatus(args) => ports::status(args, environment, streams),
        Command::PortsInstall => ports::install(environment, streams),
        Command::PortsUninstall => ports::uninstall(environment, streams),
        Command::CaStatus => ca::status(environment, streams),
        Command::CaTrust => ca::trust(environment, streams),
        Command::CaUntrust => ca::untrust(environment, streams),
        Command::Link(args) => project::link(args, environment, streams),
        Command::Init(args) => init::run(args, environment, streams),
        Command::Unlink(args) => project::unlink(args, environment, streams),
        Command::Open(args) => project::open(args, environment, streams),
        Command::ProjectEnv(args) => project::env(args, environment, streams),
        Command::Status(args) => status::run(args, environment, streams),
        Command::Logs(args) => logs::run(args, environment, streams),
        Command::Doctor(args) => doctor::run(args, environment, streams),
        Command::Jobs(args) => jobs::run(args, environment, streams),
        Command::Update(args) => update::run(args, environment, streams),
        Command::InternalUpdateManagedResources => {
            update::run_managed_resource_continuation(environment, streams)
        }
        Command::List(args) => project::list(args, environment, streams),
        Command::PhpUse(args) => php::use_track(args, environment, streams),
        Command::PhpInstall(args) => php::install(args, environment, streams),
        Command::PhpUpdate => php::update(environment, streams),
        Command::PhpUninstall(args) => php::uninstall(args, environment, streams),
        Command::PhpList(args) => php::list(args, environment, streams),
        Command::ComposerInstall => composer::install(environment, streams),
        Command::ComposerUpdate => composer::update(environment, streams),
        Command::ComposerUninstall(args) => composer::uninstall(args, environment, streams),
        Command::MailpitInstall(args) | Command::MailInstall(args) => {
            mailpit::install(args, environment, streams)
        }
        Command::MailpitUpdate | Command::MailUpdate => mailpit::update(environment, streams),
        Command::MailpitUninstall(args) | Command::MailUninstall(args) => {
            mailpit::uninstall(args, environment, streams)
        }
        Command::MailpitList(args) | Command::MailList(args) => {
            mailpit::list(args, environment, streams)
        }
        Command::MailpitOpen | Command::MailOpen => mailpit::open(environment, streams),
        Command::RedisInstall(args) => redis::install(args, environment, streams),
        Command::RedisUpdate => redis::update(environment, streams),
        Command::RedisUninstall(args) => redis::uninstall(args, environment, streams),
        Command::RedisList(args) => redis::list(args, environment, streams),
        Command::RustfsInstall(args) | Command::S3Install(args) => {
            rustfs::install(args, environment, streams)
        }
        Command::RustfsUpdate | Command::S3Update => rustfs::update(environment, streams),
        Command::RustfsUninstall(args) | Command::S3Uninstall(args) => {
            rustfs::uninstall(args, environment, streams)
        }
        Command::RustfsList(args) | Command::S3List(args) => {
            rustfs::list(args, environment, streams)
        }
        Command::RustfsOpen | Command::S3Open => rustfs::open(environment, streams),
        Command::MysqlInstall(args) => mysql::install(args, environment, streams),
        Command::MysqlUpdate => mysql::update(environment, streams),
        Command::MysqlUninstall(args) => mysql::uninstall(args, environment, streams),
        Command::MysqlList(args) => mysql::list(args, environment, streams),
        Command::PostgresInstall(args) | Command::PgInstall(args) => {
            postgres::install(args, environment, streams)
        }
        Command::PostgresUpdate | Command::PgUpdate => postgres::update(environment, streams),
        Command::PostgresUninstall(args) | Command::PgUninstall(args) => {
            postgres::uninstall(args, environment, streams)
        }
        Command::PostgresList(args) | Command::PgList(args) => {
            postgres::list(args, environment, streams)
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
        Command::PortsStatus(_) | Command::PortsInstall | Command::PortsUninstall => {
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
    state::UpdateLock::require_no_update_in_progress(&paths).map_err(coordination_lock_error)
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

const RECONCILE_KIND: &str = "reconcile";
const SYSTEM_SCOPE: &str = "system";
const JOBS_LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(100);
const DEFERRED_RECONCILIATION_WARNING: &str =
    "reconciliation deferred while another PV mutation holds the jobs lock";
const DAEMON_UNAVAILABLE_WARNING: &str =
    "PV daemon is not running; reconciliation will run after `pv setup` starts it";

fn acquire_jobs_lock(paths: &PvPaths) -> Result<state::JobsLock, ExecuteError> {
    state::JobsLock::acquire(paths).map_err(coordination_lock_error)
}

/// Submits a reconciliation request for state a command has already committed.
///
/// The command releases `jobs.lock` before notifying the daemon, so a competing
/// mutation can win the handoff and make the daemon reject this request. Retry
/// that specific rejection until the request is admitted instead of losing the
/// reconciliation for committed state. Warnings go to `stderr`.
fn submit_reconciliation(
    paths: &PvPaths,
    scope: &str,
    stderr: &mut Output<'_>,
) -> Result<Option<::daemon::SubmittedJob>, ExecuteError> {
    let mut deferred = false;
    loop {
        match ::daemon::submit_job_blocking(paths.clone(), RECONCILE_KIND, scope) {
            Ok(job) => return Ok(Some(job)),
            Err(::daemon::DaemonError::Io(error)) if daemon_is_unavailable(&error) => {
                stderr.warning(DAEMON_UNAVAILABLE_WARNING)?;

                return Ok(None);
            }
            Err(::daemon::DaemonError::DaemonRejected { message })
                if message.contains(paths.jobs_lock().as_str()) =>
            {
                if !deferred {
                    deferred = true;
                    stderr.warning(DEFERRED_RECONCILIATION_WARNING)?;
                }
                std::thread::sleep(JOBS_LOCK_RETRY_INTERVAL);
            }
            Err(error) => return Err(error.into()),
        }
    }
}

/// Requests system reconciliation for committed state. The job line is part
/// of the command's result; warnings go to stderr.
fn request_system_reconciliation(
    paths: &PvPaths,
    streams: &mut Streams<'_>,
) -> Result<(), ExecuteError> {
    if let Some(job) = submit_reconciliation(paths, SYSTEM_SCOPE, &mut streams.err)? {
        write_reconciliation_requested(&mut streams.out, &job.id)?;
    }

    Ok(())
}

/// Requests reconciliation of one Project for committed state.
fn request_project_reconciliation(
    paths: &PvPaths,
    project: &state::ProjectRecord,
    streams: &mut Streams<'_>,
) -> Result<(), ExecuteError> {
    let scope = format!("project:{}", project.id);
    if let Some(job) = submit_reconciliation(paths, &scope, &mut streams.err)? {
        streams.out.follow_up(
            Line::from("Queued reconciliation ")
                .value(job.id)
                .text(format!(" for {}", project_display_name(project))),
        )?;
    }

    Ok(())
}

/// The name users know a Project by: its slug when resource-only, otherwise
/// its primary hostname.
fn project_display_name(project: &state::ProjectRecord) -> &str {
    if project.mode == state::ProjectMode::ResourceOnly {
        return project.slug.as_str();
    }

    project
        .primary_hostname
        .as_deref()
        .unwrap_or(project.slug.as_str())
}

/// Reports a system file PV must not touch because it is not PV-owned or
/// cannot be inspected.
fn write_left_in_place(
    output: &mut Output<'_>,
    summary: &str,
    path: &camino::Utf8Path,
    message: Option<&str>,
) -> io::Result<()> {
    output.failure(Line::field(summary, path))?;
    if let Some(message) = message {
        output.detail(message)?;
    }
    output.detail("Leaving it in place.")
}

/// The follow-up line for an accepted system reconciliation request.
fn write_reconciliation_requested(output: &mut Output<'_>, job_id: &str) -> io::Result<()> {
    output.follow_up(Line::field("System reconciliation requested: ", job_id))
}

fn write_php_pair_install_lines(
    installed: &resources::PhpPairInstall,
    streams: &mut Streams<'_>,
) -> Result<(), ExecuteError> {
    write_revoked_latest_warning(installed.php(), &mut streams.err)?;
    write_revoked_latest_warning(installed.frankenphp(), &mut streams.err)?;
    streams
        .out
        .success(Line::field("Installed PHP track ", installed.php().track()))?;
    streams.out.success(Line::field(
        "Installed FrankenPHP track ",
        installed.frankenphp().track(),
    ))?;

    Ok(())
}

/// A runtime's mark. A degraded runtime is a failure, as `pv status` and
/// `pv doctor` count it.
fn runtime_mark(status: Option<RuntimeObservedStatus>) -> Mark {
    match status {
        Some(RuntimeObservedStatus::Running) => Mark::Running,
        Some(RuntimeObservedStatus::Degraded | RuntimeObservedStatus::Failed) => Mark::Failure,
        Some(RuntimeObservedStatus::Pending | RuntimeObservedStatus::Stopped) | None => Mark::Idle,
    }
}

/// Announces the helper installation that macOS will ask an administrator
/// password for. It is a stderr row, like the prompt it introduces.
fn write_installing_helper(
    stderr: &mut Output<'_>,
    version: impl std::fmt::Display,
    protocol_version: u32,
) -> io::Result<()> {
    stderr.flow_label(
        Mark::Active,
        format!("Installing privileged helper {version} (protocol {protocol_version})"),
    )
}

/// Reports how many tracks an update changed; nothing changed is a no-op.
fn write_updated(output: &mut Output<'_>, count: usize, what: &str) -> io::Result<()> {
    let mark = if count == 0 {
        Mark::Idle
    } else {
        Mark::Success
    };
    output.status(mark, format!("Updated {count} {what}"))
}

fn daemon_is_unavailable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
}

fn coordination_lock_error(error: StateError) -> ExecuteError {
    match error {
        StateError::CoordinationLockHeld { path } => CliError::CoordinationLockHeld {
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
    stderr: &mut Output<'_>,
) -> Result<(), ExecuteError> {
    for install in installs {
        write_revoked_latest_warning(install, stderr)?;
    }

    Ok(())
}

fn write_revoked_latest_warning(
    install: &resources::ManagedResourceInstall,
    stderr: &mut Output<'_>,
) -> Result<(), ExecuteError> {
    let Some(revoked_latest) = install.revoked_latest() else {
        return Ok(());
    };

    stderr.warning(&format!(
        "newest {} artifact {} for track {} was revoked ({}); installed fallback {}",
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
    use crate::output::{Presentation, Streams};
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
                Command::Doctor(DoctorArgs { json: false }),
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
                Command::PortsStatus(crate::args::PortsStatusArgs { json: false }),
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
        let mut streams = Streams::new(&mut stdout, &mut stderr, Presentation::plain());
        let result = execute_with_capability_check(
            Cli {
                no_color: false,
                command,
            },
            &environment,
            &mut streams,
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
