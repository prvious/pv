use std::process::ExitCode;
use std::time::Duration;

use ::daemon::MonitorHooks;
use camino::Utf8PathBuf;
use platform::{DAEMON_REGISTRATION_NAME, DaemonRegistrationConfig, DaemonRegistrationFileState};
use state::{PvPaths, StateError};

use crate::environment::Environment;
use crate::error::{CliError, ExecuteError};
use crate::output::{Line, Output, Streams};

const RECONCILE_KIND: &str = "reconcile";

pub(crate) fn enable(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    enable_inner(environment, streams, true)
}

pub(crate) fn enable_without_reconciliation(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    enable_inner(environment, streams, false)
}

fn enable_inner(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
    request_reconciliation: bool,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    state::fs::ensure_layout(&paths)?;

    let config = DaemonRegistrationConfig::for_paths(&paths);
    let path = daemon_registration_path(environment)?;
    let state = platform::inspect_daemon_registration_file(&path, Some(&config));
    let output = &mut streams.out;
    let started_message = if output.surface().decorated() {
        "PV started · starts at login"
    } else {
        "Daemon started"
    };

    match state {
        DaemonRegistrationFileState::Current { .. } => {
            #[cfg(target_os = "linux")]
            environment.enable_daemon_registration(&path)?;
            environment.restart_daemon_registration()?;
            output.note(format!("{DAEMON_REGISTRATION_NAME} already installed"))?;
            output.success(started_message)?;
            wait_for_daemon(paths.clone(), output)?;
            if request_reconciliation {
                submit_system_reconciliation(paths, output)?;
            }

            Ok(ExitCode::SUCCESS)
        }
        DaemonRegistrationFileState::Missing { .. } => install_and_start_daemon_registration(
            environment,
            &path,
            &config,
            paths,
            output,
            started_message,
            request_reconciliation,
        ),
        DaemonRegistrationFileState::Stale { .. } => {
            unload_and_wait_for_daemon(environment, &paths)?;
            install_and_start_daemon_registration(
                environment,
                &path,
                &config,
                paths,
                output,
                started_message,
                request_reconciliation,
            )
        }
        DaemonRegistrationFileState::Conflict { .. }
        | DaemonRegistrationFileState::Unreadable { .. } => {
            refuse_daemon_registration(&mut streams.err, &state)
        }
    }
}

pub(crate) fn disable(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let path = daemon_registration_path(environment)?;
    let state = platform::inspect_daemon_registration_file(&path, None);
    let output = &mut streams.out;

    match state {
        DaemonRegistrationFileState::Missing { .. } => {
            stop_daemon_and_runtimes(environment)?;
            output.note(if output.surface().decorated() {
                "PV is already disabled".to_string()
            } else {
                format!("{DAEMON_REGISTRATION_NAME} already absent")
            })?;

            Ok(ExitCode::SUCCESS)
        }
        DaemonRegistrationFileState::Current { .. } | DaemonRegistrationFileState::Stale { .. } => {
            stop_daemon_and_runtimes(environment)?;
            environment.remove_daemon_registration(&path)?;
            output.success(if output.surface().decorated() {
                "PV stopped · automatic startup disabled"
            } else {
                "Daemon disabled"
            })?;
            output.success(Line::field(
                &format!("{DAEMON_REGISTRATION_NAME} removed: "),
                &path,
            ))?;

            Ok(ExitCode::SUCCESS)
        }
        DaemonRegistrationFileState::Conflict { .. }
        | DaemonRegistrationFileState::Unreadable { .. } => {
            refuse_daemon_registration(&mut streams.err, &state)
        }
    }
}

fn stop_daemon_and_runtimes(environment: &impl Environment) -> Result<(), ExecuteError> {
    let paths = pv_paths(environment)?;
    unload_and_wait_for_daemon(environment, &paths)?;
    ::daemon::stop_recorded_runtimes_blocking(paths)?;
    Ok(())
}

pub(crate) fn unload_and_wait_for_daemon(
    environment: &impl Environment,
    paths: &PvPaths,
) -> Result<(), ExecuteError> {
    let target = environment.platform_target()?;
    let daemon = environment.daemon_process_for_stop(paths)?;
    stop_daemon_registration_if_loaded(environment)?;
    // Startup can publish the daemon identity while the registration is stopping.
    let daemon = match daemon {
        Some(daemon) => Some(daemon),
        None => environment.daemon_process_for_stop(paths)?,
    };
    if let Some(daemon) = daemon {
        daemon.wait_for_exit(Duration::from_secs(10), target)?;
    }
    Ok(())
}

pub(crate) fn restart(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    state::fs::ensure_layout(&paths)?;

    let config = DaemonRegistrationConfig::for_paths(&paths);
    let path = daemon_registration_path(environment)?;
    let state = platform::inspect_daemon_registration_file(&path, Some(&config));
    let output = &mut streams.out;
    let restarted_message = if output.surface().decorated() {
        "PV restarted"
    } else {
        "Daemon restarted"
    };

    match state {
        DaemonRegistrationFileState::Current { .. } => {
            #[cfg(target_os = "linux")]
            environment.enable_daemon_registration(&path)?;
            environment.restart_daemon_registration()?;
            output.success(restarted_message)?;
            wait_for_daemon_and_submit_reconciliation(paths, output)?;

            Ok(ExitCode::SUCCESS)
        }
        DaemonRegistrationFileState::Missing { .. } => install_and_start_daemon_registration(
            environment,
            &path,
            &config,
            paths,
            output,
            restarted_message,
            true,
        ),
        DaemonRegistrationFileState::Stale { .. } => {
            unload_and_wait_for_daemon(environment, &paths)?;
            install_and_start_daemon_registration(
                environment,
                &path,
                &config,
                paths,
                output,
                restarted_message,
                true,
            )
        }
        DaemonRegistrationFileState::Conflict { .. }
        | DaemonRegistrationFileState::Unreadable { .. } => {
            refuse_daemon_registration(&mut streams.err, &state)
        }
    }
}

pub(crate) fn run() -> Result<ExitCode, ExecuteError> {
    let paths = PvPaths::default_home()?;

    ::daemon::run_blocking(paths)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn run_monitor() -> Result<ExitCode, ExecuteError> {
    let paths = PvPaths::default_home()?;

    ::daemon::run_monitor_blocking(paths, MonitorHooks::default())?;

    Ok(ExitCode::SUCCESS)
}

/// Returns only when the runtime could not start: on success, the runtime replaces this process.
pub(crate) fn run_monitor_gate() -> Result<ExitCode, ExecuteError> {
    ::daemon::run_monitor_gate(&MonitorHooks::default())?;

    Ok(ExitCode::SUCCESS)
}

/// Returns success when PV may touch the registration file; otherwise writes
/// the refusal to stderr and returns failure, for a file PV does not own or
/// cannot read.
fn refuse_daemon_registration(
    stderr: &mut Output<'_>,
    state: &DaemonRegistrationFileState,
) -> Result<ExitCode, ExecuteError> {
    let message = match state {
        DaemonRegistrationFileState::Conflict { path } => {
            format!(
                "{DAEMON_REGISTRATION_NAME} file is not PV-owned; leaving it unchanged\npath: {path}"
            )
        }
        DaemonRegistrationFileState::Unreadable { message, .. } => {
            format!(
                "{DAEMON_REGISTRATION_NAME} file is unreadable; leaving it unchanged\n{message}"
            )
        }
        DaemonRegistrationFileState::Missing { .. }
        | DaemonRegistrationFileState::Current { .. }
        | DaemonRegistrationFileState::Stale { .. } => return Ok(ExitCode::SUCCESS),
    };
    stderr.error(&message)?;

    Ok(ExitCode::FAILURE)
}

fn wait_for_daemon_and_submit_reconciliation(
    paths: PvPaths,
    output: &mut Output<'_>,
) -> Result<(), ExecuteError> {
    wait_for_daemon(paths.clone(), output)?;
    submit_system_reconciliation(paths, output)
}

fn wait_for_daemon(paths: PvPaths, output: &mut Output<'_>) -> Result<(), ExecuteError> {
    ::daemon::wait_until_healthy_blocking(paths)?;
    output.success("Daemon healthy")?;

    Ok(())
}

fn submit_system_reconciliation(
    paths: PvPaths,
    output: &mut Output<'_>,
) -> Result<(), ExecuteError> {
    let submitted = ::daemon::submit_job_blocking(paths, RECONCILE_KIND, super::SYSTEM_SCOPE)?;
    super::write_reconciliation_requested(output, &submitted.id)?;

    Ok(())
}

fn install_and_start_daemon_registration(
    environment: &impl Environment,
    path: &Utf8PathBuf,
    config: &DaemonRegistrationConfig,
    paths: PvPaths,
    output: &mut Output<'_>,
    started_message: &str,
    request_reconciliation: bool,
) -> Result<ExitCode, ExecuteError> {
    platform::write_daemon_registration_file(path, config)?;
    environment.enable_daemon_registration(path)?;
    environment.restart_daemon_registration()?;
    output.success(Line::field(
        &format!("{DAEMON_REGISTRATION_NAME} installed: "),
        path,
    ))?;
    output.success(started_message)?;
    wait_for_daemon(paths.clone(), output)?;
    if request_reconciliation {
        submit_system_reconciliation(paths, output)?;
    }

    Ok(ExitCode::SUCCESS)
}

fn stop_daemon_registration_if_loaded(environment: &impl Environment) -> Result<(), ExecuteError> {
    match environment.stop_daemon_registration() {
        Ok(()) => Ok(()),
        Err(error) if daemon_registration_is_already_stopped(&error) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn daemon_registration_is_already_stopped(error: &platform::PlatformError) -> bool {
    match error {
        platform::PlatformError::LaunchAgent(message) => {
            let message = message.to_ascii_lowercase();
            message.contains("already unloaded")
                || message.contains("not loaded")
                || message.contains("not running")
                || message.contains("no such process")
        }
        platform::PlatformError::LaunchAgentCommandStatus { .. } => false,
        _ => false,
    }
}

fn pv_paths(environment: &impl Environment) -> Result<PvPaths, ExecuteError> {
    let home = environment.home_dir().ok_or(StateError::MissingHome)?;
    let home = Utf8PathBuf::from_path_buf(home).map_err(|path| StateError::NonUtf8Home { path })?;

    Ok(PvPaths::for_home(home))
}

fn daemon_registration_path(environment: &impl Environment) -> Result<Utf8PathBuf, ExecuteError> {
    utf8_path(environment.daemon_registration_path())
}

fn utf8_path(path: impl Into<std::path::PathBuf>) -> Result<Utf8PathBuf, ExecuteError> {
    Utf8PathBuf::from_path_buf(path.into()).map_err(|path| CliError::NonUtf8Path { path }.into())
}
