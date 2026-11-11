use std::io;
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use platform::{ResolverConfig, ResolverFileState};
use state::{Database, PortOwner, PortRequest, PvPaths, StateError};

use crate::environment::Environment;
use crate::error::ExecuteError;
use crate::output::{Line, Mark, Output, Streams};

pub(crate) fn status(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let dns_port = match Database::open_read_only(&paths)? {
        Some(database) => assigned_dns_port(&database)?,
        None => None,
    };
    let system_state =
        environment.inspect_system_resolver(dns_port.map(ResolverConfig::new).as_ref());
    let output = &mut streams.out;

    output.heading("dns:status", Some("DNS resolver status"))?;
    match dns_port {
        Some(port) => output.status(Mark::Success, format!("DNS port: {port}"))?,
        None => {
            output.status(Mark::Idle, "DNS port: not assigned")?;
            output.detail("Run `pv setup`.")?;
        }
    }
    write_resolver_state(output, "System resolver config", &system_state)?;
    Ok(ExitCode::SUCCESS)
}

/// Assigns PV's DNS port when `pv.db` has none, then points the system's `.test` lookups at it.
pub(crate) fn configure(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let mut database = Database::open(&paths)?;
    let assigned = assigned_dns_port(&database)?;
    let dns_port = match assigned {
        Some(port) => port,
        None => {
            database
                .assign_port(PortRequest::pv_dns(), daemon::dns_port_available)?
                .port
        }
    };
    let system_state = environment.inspect_system_resolver(Some(&ResolverConfig::new(dns_port)));
    let output = &mut streams.out;

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
            if let Err(error) = environment.apply_system_resolver(dns_port) {
                release_new_dns_port(&mut database, assigned)?;

                return Err(error.into());
            }
            if output.surface().decorated() {
                output.success(".test domain routing configured")?;
                output.detail(Line::default().value(path.as_str()))?;
            } else {
                output.success(Line::field("Installed system resolver config: ", &path))?;
            }
            output.detail(Line::field("DNS resolver port: ", dns_port))?;
        }
        ResolverFileState::Conflict { path } => {
            release_new_dns_port(&mut database, assigned)?;
            write_resolver_conflict(output, &path)?;

            return Ok(ExitCode::FAILURE);
        }
        ResolverFileState::Unreadable { path, message } => {
            release_new_dns_port(&mut database, assigned)?;
            super::write_left_in_place(
                output,
                "System resolver config could not be inspected: ",
                &path,
                Some(&message),
            )?;

            return Ok(ExitCode::FAILURE);
        }
    }

    Ok(ExitCode::SUCCESS)
}

/// Removes PV's `.test` resolver configuration. The DNS port stays in `pv.db` until uninstall
/// deletes PV's state.
pub(crate) fn remove(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let output = &mut streams.out;

    match environment.inspect_system_resolver(None) {
        ResolverFileState::Missing { path } => {
            output.note(Line::field(
                "System resolver config already absent: ",
                &path,
            ))?;

            Ok(ExitCode::SUCCESS)
        }
        ResolverFileState::Unreadable { path, message } if !cfg!(target_os = "linux") => {
            super::write_left_in_place(
                output,
                "System resolver config could not be inspected: ",
                &path,
                Some(&message),
            )?;

            Ok(ExitCode::FAILURE)
        }
        ResolverFileState::Conflict { path } if !cfg!(target_os = "linux") => {
            write_resolver_conflict(output, &path)?;

            Ok(ExitCode::FAILURE)
        }
        ResolverFileState::Current { path, .. }
        | ResolverFileState::Stale { path, .. }
        | ResolverFileState::Conflict { path }
        | ResolverFileState::Unreadable { path, .. } => {
            environment.remove_system_resolver()?;
            if output.surface().decorated() {
                output.success(".test domain routing removed")?;
                output.detail(Line::default().value(path.as_str()))?;
            } else {
                output.success(Line::field(
                    "Removed PV-owned system resolver config: ",
                    &path,
                ))?;
            }

            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Releases a DNS port this run assigned, so a failed setup leaves `pv.db` as it found it.
fn release_new_dns_port(
    database: &mut Database,
    assigned: Option<u16>,
) -> Result<(), ExecuteError> {
    if assigned.is_none() {
        database.release_port(PortOwner::Dns)?;
    }

    Ok(())
}

fn write_resolver_conflict(output: &mut Output<'_>, path: &Utf8Path) -> io::Result<()> {
    super::write_left_in_place(
        output,
        "System resolver config is not PV-owned: ",
        path,
        None,
    )
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
        ResolverFileState::Conflict { path } => (
            Mark::Failure,
            if cfg!(target_os = "linux") {
                "conflict"
            } else {
                "not PV-owned"
            },
            path,
        ),
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

/// The DNS port PV's resolver listens on, from `pv.db`.
pub(crate) fn assigned_dns_port(database: &Database) -> Result<Option<u16>, StateError> {
    Ok(database
        .assigned_ports()?
        .into_iter()
        .find(|assignment| assignment.owner == PortOwner::Dns)
        .map(|assignment| assignment.port))
}

fn pv_paths(environment: &impl Environment) -> Result<PvPaths, ExecuteError> {
    let home = environment.home_dir().ok_or(StateError::MissingHome)?;
    let home = Utf8PathBuf::from_path_buf(home).map_err(|path| StateError::NonUtf8Home { path })?;

    Ok(PvPaths::for_home(home))
}
