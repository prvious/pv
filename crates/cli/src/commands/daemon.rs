use std::process::ExitCode;

use camino::Utf8PathBuf;
use platform::{LaunchAgentConfig, LaunchAgentFileState};
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

    let config = launch_agent_config(&paths);
    let path = launch_agent_path(environment)?;
    let state = platform::inspect_launch_agent_file(&path, Some(&config));
    let output = &mut streams.out;

    match state {
        LaunchAgentFileState::Current { .. } => {
            environment.kickstart_launch_agent()?;
            output.note("LaunchAgent already installed")?;
            output.success("Daemon started")?;
            wait_for_daemon(paths.clone(), output)?;
            if request_reconciliation {
                submit_system_reconciliation(paths, output)?;
            }

            Ok(ExitCode::SUCCESS)
        }
        LaunchAgentFileState::Missing { .. } => install_and_start_launch_agent(
            environment,
            &path,
            &config,
            paths,
            output,
            "Daemon started",
            request_reconciliation,
        ),
        LaunchAgentFileState::Stale { .. } => {
            bootout_launch_agent_if_loaded(environment)?;
            install_and_start_launch_agent(
                environment,
                &path,
                &config,
                paths,
                output,
                "Daemon started",
                request_reconciliation,
            )
        }
        LaunchAgentFileState::Conflict { .. } | LaunchAgentFileState::Unreadable { .. } => {
            refuse_launch_agent(&mut streams.err, &state)
        }
    }
}

pub(crate) fn disable(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let path = launch_agent_path(environment)?;
    let state = platform::inspect_launch_agent_file(&path, None);
    let output = &mut streams.out;

    match state {
        LaunchAgentFileState::Missing { .. } => {
            bootout_launch_agent_if_loaded(environment)?;
            output.note("LaunchAgent already absent")?;

            Ok(ExitCode::SUCCESS)
        }
        LaunchAgentFileState::Current { .. } | LaunchAgentFileState::Stale { .. } => {
            bootout_launch_agent_if_loaded(environment)?;
            platform::remove_launch_agent_file(&path)?;
            output.success("Daemon disabled")?;
            output.success(Line::field("LaunchAgent removed: ", &path))?;

            Ok(ExitCode::SUCCESS)
        }
        LaunchAgentFileState::Conflict { .. } | LaunchAgentFileState::Unreadable { .. } => {
            refuse_launch_agent(&mut streams.err, &state)
        }
    }
}

pub(crate) fn restart(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    state::fs::ensure_layout(&paths)?;

    let config = launch_agent_config(&paths);
    let path = launch_agent_path(environment)?;
    let state = platform::inspect_launch_agent_file(&path, Some(&config));
    let output = &mut streams.out;

    match state {
        LaunchAgentFileState::Current { .. } => {
            environment.kickstart_launch_agent()?;
            output.success("Daemon restarted")?;
            wait_for_daemon_and_submit_reconciliation(paths, output)?;

            Ok(ExitCode::SUCCESS)
        }
        LaunchAgentFileState::Missing { .. } => install_and_start_launch_agent(
            environment,
            &path,
            &config,
            paths,
            output,
            "Daemon restarted",
            true,
        ),
        LaunchAgentFileState::Stale { .. } => {
            bootout_launch_agent_if_loaded(environment)?;
            install_and_start_launch_agent(
                environment,
                &path,
                &config,
                paths,
                output,
                "Daemon restarted",
                true,
            )
        }
        LaunchAgentFileState::Conflict { .. } | LaunchAgentFileState::Unreadable { .. } => {
            refuse_launch_agent(&mut streams.err, &state)
        }
    }
}

pub(crate) fn run() -> Result<ExitCode, ExecuteError> {
    let paths = PvPaths::default_home()?;

    ::daemon::run_blocking(paths)?;

    Ok(ExitCode::SUCCESS)
}

/// Returns success when PV may touch the LaunchAgent file; otherwise writes
/// the refusal to stderr and returns failure, for a file PV does not own or
/// cannot read.
fn refuse_launch_agent(
    stderr: &mut Output<'_>,
    state: &LaunchAgentFileState,
) -> Result<ExitCode, ExecuteError> {
    let message = match state {
        LaunchAgentFileState::Conflict { path } => {
            format!("LaunchAgent file is not PV-owned; leaving it unchanged\npath: {path}")
        }
        LaunchAgentFileState::Unreadable { message, .. } => {
            format!("LaunchAgent file is unreadable; leaving it unchanged\n{message}")
        }
        LaunchAgentFileState::Missing { .. }
        | LaunchAgentFileState::Current { .. }
        | LaunchAgentFileState::Stale { .. } => return Ok(ExitCode::SUCCESS),
    };
    stderr.error(&message)?;

    Ok(ExitCode::FAILURE)
}

fn launch_agent_config(paths: &PvPaths) -> LaunchAgentConfig {
    LaunchAgentConfig::new(
        paths.active_pv_binary(),
        paths.logs().join("launchd.out.log"),
        paths.logs().join("launchd.err.log"),
    )
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

fn install_and_start_launch_agent(
    environment: &impl Environment,
    path: &Utf8PathBuf,
    config: &LaunchAgentConfig,
    paths: PvPaths,
    output: &mut Output<'_>,
    started_message: &str,
    request_reconciliation: bool,
) -> Result<ExitCode, ExecuteError> {
    platform::write_launch_agent_file(path, config)?;
    environment.bootstrap_launch_agent(path)?;
    environment.kickstart_launch_agent()?;
    output.success(Line::field("LaunchAgent installed: ", path))?;
    output.success(started_message)?;
    wait_for_daemon(paths.clone(), output)?;
    if request_reconciliation {
        submit_system_reconciliation(paths, output)?;
    }

    Ok(ExitCode::SUCCESS)
}

fn bootout_launch_agent_if_loaded(environment: &impl Environment) -> Result<(), ExecuteError> {
    match environment.bootout_launch_agent() {
        Ok(()) => Ok(()),
        Err(error) if launch_agent_is_already_unloaded(&error) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn launch_agent_is_already_unloaded(error: &platform::PlatformError) -> bool {
    match error {
        platform::PlatformError::LaunchAgent(message) => {
            let message = message.to_ascii_lowercase();
            message.contains("already unloaded")
                || message.contains("not loaded")
                || message.contains("not running")
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

fn launch_agent_path(environment: &impl Environment) -> Result<Utf8PathBuf, ExecuteError> {
    utf8_path(environment.launch_agent_path())
}

fn utf8_path(path: impl Into<std::path::PathBuf>) -> Result<Utf8PathBuf, ExecuteError> {
    Utf8PathBuf::from_path_buf(path.into()).map_err(|path| CliError::NonUtf8Path { path }.into())
}
