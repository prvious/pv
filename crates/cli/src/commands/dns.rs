use std::io;
use std::io::Write;
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use platform::{ResolverConfig, ResolverFileState};
use state::{Database, PortOwner, PortRequest, PvPaths, StateError};

use crate::environment::Environment;
use crate::error::{CliError, ExecuteError};
use crate::output::{Output, OutputMode};

pub(crate) fn status(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let prepared_path = paths.resolver_config();
    let system_path = resolver_test_path(environment)?;
    let prepared_state = platform::inspect_resolver_file(&prepared_path, None);
    let expected_config = resolver_config_from_state(&prepared_state);
    let system_state = platform::inspect_resolver_file(&system_path, expected_config.as_ref());
    let mut output = Output::new(stdout, OutputMode::plain());

    output.line("DNS resolver status")?;
    write_resolver_state(&mut output, "Prepared resolver config", &prepared_state)?;
    write_resolver_state(&mut output, "System resolver config", &system_state)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn install(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let system_path = resolver_test_path(environment)?;
    let mut database = Database::open(&paths)?;
    let dns_port = prepared_dns_port(&mut database)?;
    let config = ResolverConfig::new(dns_port);
    let prepared_path = paths.resolver_config();

    state::fs::write_sensitive_file(&prepared_path, &config.render())?;

    let system_state = platform::inspect_resolver_file(&system_path, Some(&config));
    let mut output = Output::new(stdout, OutputMode::plain());

    output.line("Prepared PV DNS resolver config")?;
    output.line(&format!("  path: {prepared_path}"))?;
    output.line(&format!("  DNS resolver port: {dns_port}"))?;

    match system_state {
        ResolverFileState::Current { path, port } => {
            output.line(&format!(
                "System resolver config already matches PV on port {port}: {path}"
            ))?;

            Ok(ExitCode::SUCCESS)
        }
        ResolverFileState::Missing { path } | ResolverFileState::Stale { path, .. } => {
            environment.install_resolver_config(&prepared_path, &system_path)?;
            output.line(&format!("Installed system resolver config: {path}"))?;

            Ok(ExitCode::SUCCESS)
        }
        ResolverFileState::Conflict { path } => {
            output.line(&format!("System resolver config is not PV-owned: {path}"))?;
            output.line("Leaving it in place.")?;

            Ok(ExitCode::FAILURE)
        }
        ResolverFileState::Unreadable { path, message } => {
            output.line(&format!(
                "System resolver config could not be inspected: {path}"
            ))?;
            output.line(&format!("  {message}"))?;
            output.line("Leaving it in place.")?;

            Ok(ExitCode::FAILURE)
        }
    }
}

pub(crate) fn uninstall(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let prepared_path = paths.resolver_config();
    let system_path = resolver_test_path(environment)?;
    let deleted_prepared = delete_optional_file(&prepared_path)?;
    let system_state = platform::inspect_resolver_file(&system_path, None);
    let mut output = Output::new(stdout, OutputMode::plain());

    if deleted_prepared {
        output.line(&format!(
            "Deleted prepared DNS resolver config: {prepared_path}"
        ))?;
    } else {
        output.line(&format!(
            "Prepared DNS resolver config already absent: {prepared_path}"
        ))?;
    }

    match system_state {
        ResolverFileState::Missing { path } => {
            output.line(&format!("System resolver config already absent: {path}"))?;

            Ok(ExitCode::SUCCESS)
        }
        ResolverFileState::Unreadable { path, message } => {
            output.line(&format!(
                "System resolver config could not be inspected: {path}"
            ))?;
            output.line(&format!("  {message}"))?;
            output.line("Leaving it in place.")?;

            Ok(ExitCode::FAILURE)
        }
        ResolverFileState::Current { path, .. } | ResolverFileState::Stale { path, .. } => {
            environment.remove_resolver_config(&system_path)?;
            output.line(&format!("Removed PV-owned system resolver config: {path}"))?;

            Ok(ExitCode::SUCCESS)
        }
        ResolverFileState::Conflict { path } => {
            output.line(&format!("System resolver config is not PV-owned: {path}"))?;
            output.line("Leaving it in place.")?;

            Ok(ExitCode::FAILURE)
        }
    }
}

fn write_resolver_state(
    output: &mut Output<'_, impl Write>,
    label: &str,
    state: &ResolverFileState,
) -> io::Result<()> {
    match state {
        ResolverFileState::Missing { path } => {
            output.line(&format!("{label}: missing"))?;
            output.line(&format!("  path: {path}"))
        }
        ResolverFileState::Current { path, port } => {
            output.line(&format!("{label}: current"))?;
            output.line(&format!("  path: {path}"))?;
            output.line(&format!("  port: {port}"))
        }
        ResolverFileState::Stale {
            path,
            expected_port,
            actual_port,
        } => {
            output.line(&format!("{label}: stale"))?;
            output.line(&format!("  path: {path}"))?;
            match expected_port {
                Some(expected_port) => output.line(&format!("  expected port: {expected_port}"))?,
                None => output.line("  expected port: unknown")?,
            }
            match actual_port {
                Some(actual_port) => output.line(&format!("  actual port: {actual_port}")),
                None => output.line("  actual port: unparseable"),
            }
        }
        ResolverFileState::Conflict { path } => {
            output.line(&format!("{label}: not PV-owned"))?;
            output.line(&format!("  path: {path}"))
        }
        ResolverFileState::Unreadable { path, message } => {
            output.line(&format!("{label}: unreadable"))?;
            output.line(&format!("  path: {path}"))?;
            output.line(&format!("  {message}"))
        }
    }
}

fn prepared_dns_port(database: &mut Database) -> Result<u16, StateError> {
    if let Some(assignment) = database
        .assigned_ports()?
        .into_iter()
        .find(|assignment| assignment.owner == PortOwner::Dns)
    {
        return Ok(assignment.port);
    }

    let assignment = database.assign_port(PortRequest::pv_dns(), daemon::dns_port_available)?;

    Ok(assignment.port)
}

fn resolver_config_from_state(state: &ResolverFileState) -> Option<ResolverConfig> {
    match state {
        ResolverFileState::Current { port, .. }
        | ResolverFileState::Stale {
            actual_port: Some(port),
            ..
        } => Some(ResolverConfig::new(*port)),
        ResolverFileState::Missing { .. }
        | ResolverFileState::Stale {
            actual_port: None, ..
        }
        | ResolverFileState::Conflict { .. }
        | ResolverFileState::Unreadable { .. } => None,
    }
}

fn delete_optional_file(path: &Utf8Path) -> Result<bool, ExecuteError> {
    match state::fs::delete_file(path) {
        Ok(()) => Ok(true),
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(false)
        }
        Err(error) => Err(error.into()),
    }
}

fn pv_paths(environment: &impl Environment) -> Result<PvPaths, ExecuteError> {
    let home = environment.home_dir().ok_or(StateError::MissingHome)?;
    let home = Utf8PathBuf::from_path_buf(home).map_err(|path| StateError::NonUtf8Home { path })?;

    Ok(PvPaths::for_home(home))
}

fn resolver_test_path(environment: &impl Environment) -> Result<Utf8PathBuf, ExecuteError> {
    Utf8PathBuf::from_path_buf(environment.resolver_test_path())
        .map_err(|path| CliError::NonUtf8Path { path }.into())
}
