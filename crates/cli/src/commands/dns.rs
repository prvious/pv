use std::io;
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use platform::{ResolverConfig, ResolverFileState};
use state::{Database, PortOwner, PortRequest, PvPaths, StateError};

use crate::environment::Environment;
use crate::error::{CliError, ExecuteError};
use crate::output::{Line, Mark, Output, Streams};

pub(crate) fn status(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let prepared_path = paths.resolver_config();
    let system_path = resolver_test_path(environment)?;
    let prepared_state = platform::inspect_resolver_file(&prepared_path, None);
    let expected_config = resolver_config_from_state(&prepared_state);
    let system_state = environment.inspect_resolver_file(&system_path, expected_config.as_ref());
    let output = &mut streams.out;

    output.heading("dns:status", Some("DNS resolver status"))?;
    write_resolver_state(output, "Prepared resolver config", &prepared_state)?;
    write_resolver_state(output, "System resolver config", &system_state)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn install(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    install_inner(environment, streams, true)
}

pub(crate) fn install_config_only(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    install_inner(environment, streams, false)
}

fn install_inner(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
    ensure_daemon: bool,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let system_path = resolver_test_path(environment)?;
    let mut database = Database::open(&paths)?;
    let had_dns_assignment = database
        .assigned_ports()?
        .iter()
        .any(|assignment| assignment.owner == PortOwner::Dns);
    let dns_port = prepared_dns_port(&mut database)?;
    let config = ResolverConfig::new(dns_port);
    let prepared_path = paths.resolver_config();

    if let Err(error) = state::fs::write_sensitive_file(&prepared_path, &config.render()) {
        release_new_dns_port(&mut database, had_dns_assignment)?;

        return Err(error.into());
    }

    let system_state = environment.inspect_resolver_file(&system_path, Some(&config));
    let output = &mut streams.out;

    output.success("Prepared PV DNS resolver config")?;
    output.detail(Line::field("path: ", &prepared_path))?;
    output.detail(Line::field("DNS resolver port: ", dns_port))?;

    match &system_state {
        ResolverFileState::Current { .. }
        | ResolverFileState::Missing { .. }
        | ResolverFileState::Stale { .. } => {}
        ResolverFileState::Conflict { path } => {
            release_new_dns_port(&mut database, had_dns_assignment)?;
            super::write_left_in_place(
                output,
                "System resolver config is not PV-owned: ",
                path,
                None,
            )?;

            return Ok(ExitCode::FAILURE);
        }
        ResolverFileState::Unreadable { path, message } => {
            release_new_dns_port(&mut database, had_dns_assignment)?;
            super::write_left_in_place(
                output,
                "System resolver config could not be inspected: ",
                path,
                Some(message),
            )?;

            return Ok(ExitCode::FAILURE);
        }
    }

    if ensure_daemon {
        let exit_code = ensure_daemon_running(&paths, output)?;
        if exit_code != ExitCode::SUCCESS {
            release_new_dns_port(&mut database, had_dns_assignment)?;

            return Ok(exit_code);
        }
    }

    match system_state {
        ResolverFileState::Current { path, port } => {
            output.note(
                Line::from(format!(
                    "System resolver config already matches PV on port {port}: "
                ))
                .value(path.as_str()),
            )?;
        }
        ResolverFileState::Missing { path } | ResolverFileState::Stale { path, .. } => {
            if let Err(error) = environment.install_resolver_config(&prepared_path, &system_path) {
                release_new_dns_port(&mut database, had_dns_assignment)?;

                return Err(error.into());
            }
            output.success(Line::field("Installed system resolver config: ", &path))?;
        }
        ResolverFileState::Conflict { .. } | ResolverFileState::Unreadable { .. } => {
            return Ok(ExitCode::FAILURE);
        }
    }

    Ok(ExitCode::SUCCESS)
}

fn release_new_dns_port(
    database: &mut Database,
    had_dns_assignment: bool,
) -> Result<(), ExecuteError> {
    if !had_dns_assignment {
        database.release_port(PortOwner::Dns)?;
    }

    Ok(())
}

fn ensure_daemon_running(
    paths: &PvPaths,
    output: &mut Output<'_>,
) -> Result<ExitCode, ExecuteError> {
    let daemon_socket = paths.daemon_socket();

    let cause = if daemon_socket.exists() {
        match ::daemon::wait_until_healthy_blocking(paths.clone()) {
            Ok(()) => {
                output.status(Mark::Running, "PV daemon is running.")?;

                return Ok(ExitCode::SUCCESS);
            }
            Err(error) => Line::from(error.to_string()),
        }
    } else {
        Line::field("socket: ", &daemon_socket)
    };
    output.failure("PV daemon is not running; .test lookups will not resolve yet.")?;
    output.detail("Run `pv setup` or `pv daemon:enable`, then retry `pv dns:install`.")?;
    output.detail(cause)?;

    Ok(ExitCode::FAILURE)
}

pub(crate) fn uninstall(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let mut database = Database::open(&paths)?;
    let prepared_path = paths.resolver_config();
    let system_path = resolver_test_path(environment)?;
    let deleted_prepared = delete_optional_file(&prepared_path)?;
    let system_state = environment.inspect_resolver_file(&system_path, None);
    let output = &mut streams.out;

    if deleted_prepared {
        output.success(Line::field(
            "Deleted prepared DNS resolver config: ",
            &prepared_path,
        ))?;
    } else {
        output.note(Line::field(
            "Prepared DNS resolver config already absent: ",
            &prepared_path,
        ))?;
    }

    match system_state {
        ResolverFileState::Missing { path } => {
            output.note(Line::field(
                "System resolver config already absent: ",
                &path,
            ))?;
            database.release_port(PortOwner::Dns)?;

            Ok(ExitCode::SUCCESS)
        }
        ResolverFileState::Unreadable { path, message } => {
            super::write_left_in_place(
                output,
                "System resolver config could not be inspected: ",
                &path,
                Some(&message),
            )?;

            Ok(ExitCode::FAILURE)
        }
        ResolverFileState::Current { path, .. } | ResolverFileState::Stale { path, .. } => {
            environment.remove_resolver_config(&system_path)?;
            output.success(Line::field(
                "Removed PV-owned system resolver config: ",
                &path,
            ))?;
            database.release_port(PortOwner::Dns)?;

            Ok(ExitCode::SUCCESS)
        }
        ResolverFileState::Conflict { path } => {
            super::write_left_in_place(
                output,
                "System resolver config is not PV-owned: ",
                &path,
                None,
            )?;

            Ok(ExitCode::FAILURE)
        }
    }
}

fn write_resolver_state(
    output: &mut Output<'_>,
    label: &str,
    state: &ResolverFileState,
) -> io::Result<()> {
    let (mark, status, path) = match state {
        ResolverFileState::Missing { path } => (Mark::Idle, "missing", path),
        ResolverFileState::Current { path, .. } => (Mark::Success, "current", path),
        ResolverFileState::Stale { path, .. } => (Mark::Warning, "stale", path),
        ResolverFileState::Conflict { path } => (Mark::Failure, "not PV-owned", path),
        ResolverFileState::Unreadable { path, .. } => (Mark::Failure, "unreadable", path),
    };
    output.status(mark, format!("{label}: {status}"))?;
    output.detail(Line::field("path: ", path))?;
    match state {
        ResolverFileState::Current { port, .. } => output.detail(Line::field("port: ", port)),
        ResolverFileState::Stale {
            expected_port,
            actual_port,
            ..
        } => {
            let expected =
                expected_port.map_or_else(|| "unknown".to_string(), |port| port.to_string());
            let actual =
                actual_port.map_or_else(|| "unparseable".to_string(), |port| port.to_string());
            output.detail(Line::field("expected port: ", &expected))?;
            output.detail(Line::field("actual port: ", &actual))
        }
        ResolverFileState::Unreadable { message, .. } => output.detail(message),
        ResolverFileState::Missing { .. } | ResolverFileState::Conflict { .. } => Ok(()),
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
