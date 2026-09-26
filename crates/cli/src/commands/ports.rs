use std::io;
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use platform::{PfConfReference, PfFileState, PfRedirectConfig};
use state::{
    Database, GatewayPort, GatewayPortAssignments, PortOwner, PvPaths, RuntimeObservedStatus,
    RuntimeSubject, StateError,
};

use crate::args::PortsStatusArgs;
use crate::environment::Environment;
use crate::error::{CliError, ExecuteError};
use crate::output::{Line, Mark, Output, Streams};

use super::pf_diagnostics::PfRoutingDiagnostic;

const LOW_PORTS: [u16; 2] = [80, 443];

pub(crate) fn status(
    args: PortsStatusArgs,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let database = Database::open_read_only(&paths)?;
    let diagnostic = PfRoutingDiagnostic::read(environment, &paths, database.as_ref())?;
    let (exit_code, mark) = if diagnostic.is_active() {
        (ExitCode::SUCCESS, Mark::Success)
    } else {
        (ExitCode::FAILURE, Mark::Failure)
    };

    if args.json {
        streams.out.json(&diagnostic)?;

        return Ok(exit_code);
    }

    let output = &mut streams.out;

    output.heading("ports:status", Some("Port redirect status"))?;
    output.status(mark, format!("State: {}", diagnostic.state.as_str()))?;
    output.detail(Line::field("Evidence: ", diagnostic.evidence.as_str()))?;
    output.detail(format!(
        "Expected redirects: HTTP {}, HTTPS {}",
        display_port(diagnostic.expected_http_port),
        display_port(diagnostic.expected_https_port),
    ))?;
    output.detail(format!(
        "Active redirects: HTTP {}, HTTPS {}",
        display_port(diagnostic.active_http_port),
        display_port(diagnostic.active_https_port),
    ))?;
    output.detail(Line::field("Observed: ", &diagnostic.observed_at))?;
    if !diagnostic.is_active() {
        output.hint("repair", "pv ports:install")?;
    }

    Ok(exit_code)
}

fn display_port(port: Option<u16>) -> String {
    port.map_or_else(|| "-".to_owned(), |port| port.to_string())
}

pub(crate) fn install(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let listening_ports = environment.loopback_tcp_listener_ports()?;
    let low_port_conflicts = low_port_conflicts(&listening_ports);
    let output = &mut streams.out;

    if !low_port_conflicts.is_empty() {
        output.failure("Port redirect preparation failed")?;
        for port in low_port_conflicts {
            output.detail(format!("Loopback TCP port {port} already has a listener."))?;
        }
        output.detail("Stop the conflicting service, then run `pv ports:install` again.")?;

        return Ok(ExitCode::FAILURE);
    }

    let mut database = Database::open(&paths)?;
    let existing_assignments = database.assigned_ports()?;
    let had_http_assignment = existing_assignments
        .iter()
        .any(|assignment| assignment.owner == PortOwner::Gateway(GatewayPort::Http));
    let had_https_assignment = existing_assignments
        .iter()
        .any(|assignment| assignment.owner == PortOwner::Gateway(GatewayPort::Https));
    let assignments = database.assign_gateway_ports(|port| !listening_ports.contains(&port))?;
    let config = pf_config_from_assignments(&assignments);
    let reference = PfConfReference;
    let prepared_anchor_path = paths.pf_anchor_config();
    let prepared_reference_path = paths.pf_conf_reference_config();
    let system_anchor_path = pf_anchor_path(environment)?;
    let system_pf_conf_path = pf_conf_path(environment)?;

    if let Err(error) =
        state::fs::write_sensitive_file(&prepared_anchor_path, &config.render_anchor())
    {
        release_new_gateway_ports(&mut database, had_http_assignment, had_https_assignment)?;

        return Err(error.into());
    }
    if let Err(error) =
        state::fs::write_sensitive_file(&prepared_reference_path, &reference.render())
    {
        release_new_gateway_ports(&mut database, had_http_assignment, had_https_assignment)?;

        return Err(error.into());
    }

    let system_anchor_state = platform::inspect_pf_anchor_file(&system_anchor_path, Some(&config));
    let system_reference_state =
        platform::inspect_pf_conf_reference(&system_pf_conf_path, Some(&reference));

    output.success("Prepared PV port redirect config")?;
    output.detail(Line::field("anchor path: ", &prepared_anchor_path))?;
    output.detail(Line::field(
        "pf.conf reference path: ",
        &prepared_reference_path,
    ))?;
    output.detail(Line::field(
        "HTTP redirect: 127.0.0.1:80 -> ",
        format!("127.0.0.1:{}", config.http_port),
    ))?;
    output.detail(Line::field(
        "HTTPS redirect: 127.0.0.1:443 -> ",
        format!("127.0.0.1:{}", config.https_port),
    ))?;

    if let Some(exit_code) =
        write_pf_blocker(output, &system_anchor_state, &system_reference_state)?
    {
        release_new_gateway_ports(&mut database, had_http_assignment, had_https_assignment)?;

        return Ok(exit_code);
    }

    let system_files_current = matches!(system_anchor_state, PfFileState::Current { .. })
        && matches!(system_reference_state, PfFileState::Current { .. });
    if system_files_current {
        let active_config = match environment.active_pf_redirect_config() {
            Ok(active_config) => active_config,
            Err(error) => {
                release_new_gateway_ports(
                    &mut database,
                    had_http_assignment,
                    had_https_assignment,
                )?;

                return Err(error.into());
            }
        };

        if active_config.as_ref() == Some(&config) {
            output.note("System pf redirect config already matches PV")?;
            refresh_gateway_observation_after_pf_repair(
                environment,
                &paths,
                &config,
                &mut database,
            )?;

            return Ok(ExitCode::SUCCESS);
        }
        output.status(
            Mark::Warning,
            "System pf redirect config matches PV, but active redirects are not loaded.",
        )?;
    }

    if let Err(error) = environment.install_pf_redirects(
        &prepared_anchor_path,
        &prepared_reference_path,
        &system_anchor_path,
        &system_pf_conf_path,
    ) {
        release_new_gateway_ports(&mut database, had_http_assignment, had_https_assignment)?;

        return Err(error.into());
    }
    ensure_active_gateway_ports(
        environment,
        &config,
        &mut database,
        had_http_assignment,
        had_https_assignment,
    )?;
    refresh_gateway_observation_after_pf_repair(environment, &paths, &config, &mut database)?;
    output.success("Installed system pf redirect config")?;

    Ok(ExitCode::SUCCESS)
}

fn refresh_gateway_observation_after_pf_repair(
    environment: &impl Environment,
    paths: &PvPaths,
    config: &PfRedirectConfig,
    database: &mut Database,
) -> Result<(), ExecuteError> {
    if environment
        .probe_gateway_redirects(config, &paths.ca_certificate())
        .is_ok()
    {
        database.record_runtime_observed_snapshot(
            RuntimeSubject::Gateway,
            RuntimeObservedStatus::Running,
            Some("Gateway identity verified through ports 80 and 443 after PF repair"),
        )?;

        return Ok(());
    }

    let pf_derived_observation = database
        .runtime_observed_states()?
        .into_iter()
        .any(|state| {
            state.subject == RuntimeSubject::Gateway
                && state.status == RuntimeObservedStatus::Degraded
                && state
                    .message
                    .as_deref()
                    .is_some_and(|message| message.starts_with("Low-port routing is "))
        });
    if pf_derived_observation {
        database.record_runtime_observed_snapshot(
            RuntimeSubject::Gateway,
            RuntimeObservedStatus::Pending,
            Some("Low-port routing repaired; Gateway readiness is pending reconciliation"),
        )?;
    }
    let _request_result = environment.request_system_reconciliation(paths);

    Ok(())
}

fn ensure_active_gateway_ports(
    environment: &impl Environment,
    config: &PfRedirectConfig,
    database: &mut Database,
    had_http_assignment: bool,
    had_https_assignment: bool,
) -> Result<(), ExecuteError> {
    let active_config = match environment.active_pf_redirect_config() {
        Ok(active_config) => active_config,
        Err(error) => {
            release_new_gateway_ports(database, had_http_assignment, had_https_assignment)?;

            return Err(error.into());
        }
    };

    if active_config.as_ref() == Some(config) {
        return Ok(());
    }

    release_new_gateway_ports(database, had_http_assignment, had_https_assignment)?;

    Err(CliError::PfRedirectsInactive.into())
}

fn release_new_gateway_ports(
    database: &mut Database,
    had_http_assignment: bool,
    had_https_assignment: bool,
) -> Result<(), ExecuteError> {
    if !had_http_assignment {
        database.release_port(PortOwner::Gateway(GatewayPort::Http))?;
    }
    if !had_https_assignment {
        database.release_port(PortOwner::Gateway(GatewayPort::Https))?;
    }
    Ok(())
}

pub(crate) fn uninstall(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let prepared_anchor_path = paths.pf_anchor_config();
    let prepared_reference_path = paths.pf_conf_reference_config();
    let candidate_dir = paths.config().join("pf");
    let system_anchor_path = pf_anchor_path(environment)?;
    let system_pf_conf_path = pf_conf_path(environment)?;
    let deleted_anchor = delete_optional_file(&prepared_anchor_path)?;
    let deleted_reference = delete_optional_file(&prepared_reference_path)?;
    let system_anchor_state = platform::inspect_pf_anchor_file(&system_anchor_path, None);
    let system_reference_state = platform::inspect_pf_conf_reference(&system_pf_conf_path, None);
    let output = &mut streams.out;

    write_delete_result(
        output,
        "prepared pf anchor",
        &prepared_anchor_path,
        deleted_anchor,
    )?;
    write_delete_result(
        output,
        "prepared pf.conf reference",
        &prepared_reference_path,
        deleted_reference,
    )?;

    if let Some(exit_code) =
        write_pf_blocker(output, &system_anchor_state, &system_reference_state)?
    {
        return Ok(exit_code);
    }

    if matches!(system_anchor_state, PfFileState::Missing { .. })
        && matches!(system_reference_state, PfFileState::Missing { .. })
    {
        output.note("System pf redirect config already absent")?;

        return Ok(ExitCode::SUCCESS);
    }

    environment.remove_pf_redirects(&system_anchor_path, &system_pf_conf_path, &candidate_dir)?;
    output.success("Removed PV-owned system pf redirect config")?;

    Ok(ExitCode::SUCCESS)
}

fn low_port_conflicts(listening_ports: &std::collections::BTreeSet<u16>) -> Vec<u16> {
    let mut conflicts = Vec::new();

    for port in LOW_PORTS {
        if listening_ports.contains(&port) {
            conflicts.push(port);
        }
    }

    conflicts
}

fn pf_config_from_assignments(assignments: &GatewayPortAssignments) -> PfRedirectConfig {
    PfRedirectConfig::new(assignments.http.port, assignments.https.port)
}

/// Reports a system pf file PV does not own or cannot inspect, which blocks
/// both install and uninstall.
fn write_pf_blocker(
    output: &mut Output<'_>,
    anchor_state: &PfFileState<PfRedirectConfig>,
    reference_state: &PfFileState<PfConfReference>,
) -> io::Result<Option<ExitCode>> {
    let blocker = match (anchor_state, reference_state) {
        (PfFileState::Conflict { path }, _) => {
            Some(("System pf anchor is not PV-owned: ", path, None))
        }
        (PfFileState::Unreadable { path, message }, _) => Some((
            "System pf anchor could not be inspected: ",
            path,
            Some(message),
        )),
        (_, PfFileState::Conflict { path }) => {
            Some(("System pf.conf reference is not PV-owned: ", path, None))
        }
        (_, PfFileState::Unreadable { path, message }) => Some((
            "System pf.conf reference could not be inspected: ",
            path,
            Some(message),
        )),
        _ => None,
    };
    let Some((summary, path, message)) = blocker else {
        return Ok(None);
    };
    super::write_left_in_place(output, summary, path, message.map(String::as_str))?;

    Ok(Some(ExitCode::FAILURE))
}

fn write_delete_result(
    output: &mut Output<'_>,
    label: &str,
    path: &Utf8Path,
    deleted: bool,
) -> io::Result<()> {
    if deleted {
        output.success(Line::from(format!("Deleted {label}: ")).value(path.as_str()))
    } else {
        output.note(Line::from(format!("{label} already absent: ")).value(path.as_str()))
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

fn pf_anchor_path(environment: &impl Environment) -> Result<Utf8PathBuf, ExecuteError> {
    Utf8PathBuf::from_path_buf(environment.pf_anchor_path())
        .map_err(|path| CliError::NonUtf8Path { path }.into())
}

fn pf_conf_path(environment: &impl Environment) -> Result<Utf8PathBuf, ExecuteError> {
    Utf8PathBuf::from_path_buf(environment.pf_conf_path())
        .map_err(|path| CliError::NonUtf8Path { path }.into())
}
