use std::io;
use std::io::Write;
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
use crate::output::{Output, OutputMode};

use super::pf_diagnostics::PfRoutingDiagnostic;

const LOW_PORTS: [u16; 2] = [80, 443];

pub(crate) fn status(
    args: PortsStatusArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let database = Database::open_read_only(&paths)?;
    let diagnostic = PfRoutingDiagnostic::read(environment, &paths, database.as_ref())?;
    let exit_code = if diagnostic.is_active() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    };

    if args.json {
        serde_json::to_writer(&mut *stdout, &diagnostic)?;
        writeln!(stdout)?;

        return Ok(exit_code);
    }

    let mut output = Output::new(stdout, OutputMode::plain());

    output.line("Port redirect status")?;
    output.line(&format!("State: {}", diagnostic.state.as_str()))?;
    output.line(&format!("Evidence: {}", diagnostic.evidence.as_str()))?;
    output.line(&format!(
        "Expected redirects: HTTP {}, HTTPS {}",
        display_port(diagnostic.expected_http_port),
        display_port(diagnostic.expected_https_port),
    ))?;
    output.line(&format!(
        "Active redirects: HTTP {}, HTTPS {}",
        display_port(diagnostic.active_http_port),
        display_port(diagnostic.active_https_port),
    ))?;
    output.line(&format!("Observed: {}", diagnostic.observed_at))?;
    if !diagnostic.is_active() {
        output.line("Repair: `pv ports:install`")?;
    }

    Ok(exit_code)
}

fn display_port(port: Option<u16>) -> String {
    port.map_or_else(|| "-".to_owned(), |port| port.to_string())
}

pub(crate) fn install(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let listening_ports = environment.loopback_tcp_listener_ports()?;
    let low_port_conflicts = low_port_conflicts(&listening_ports);
    let mut output = Output::new(stdout, OutputMode::plain());

    if !low_port_conflicts.is_empty() {
        output.line("Port redirect preparation failed")?;
        for port in low_port_conflicts {
            output.line(&format!("Loopback TCP port {port} already has a listener."))?;
        }
        output.line("Stop the conflicting service, then run `pv ports:install` again.")?;

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
    let had_admin_assignment = existing_assignments
        .iter()
        .any(|assignment| assignment.owner == PortOwner::Gateway(GatewayPort::Admin));
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
        release_new_gateway_ports(
            &mut database,
            had_http_assignment,
            had_https_assignment,
            had_admin_assignment,
        )?;

        return Err(error.into());
    }
    if let Err(error) =
        state::fs::write_sensitive_file(&prepared_reference_path, &reference.render())
    {
        release_new_gateway_ports(
            &mut database,
            had_http_assignment,
            had_https_assignment,
            had_admin_assignment,
        )?;

        return Err(error.into());
    }

    let system_anchor_state = platform::inspect_pf_anchor_file(&system_anchor_path, Some(&config));
    let system_reference_state =
        platform::inspect_pf_conf_reference(&system_pf_conf_path, Some(&reference));

    output.line("Prepared PV port redirect config")?;
    output.line(&format!("  anchor path: {prepared_anchor_path}"))?;
    output.line(&format!(
        "  pf.conf reference path: {prepared_reference_path}"
    ))?;
    output.line(&format!(
        "  HTTP redirect: 127.0.0.1:80 -> 127.0.0.1:{}",
        config.http_port
    ))?;
    output.line(&format!(
        "  HTTPS redirect: 127.0.0.1:443 -> 127.0.0.1:{}",
        config.https_port
    ))?;

    if let Some(exit_code) =
        write_pf_install_blocker(&mut output, &system_anchor_state, &system_reference_state)?
    {
        release_new_gateway_ports(
            &mut database,
            had_http_assignment,
            had_https_assignment,
            had_admin_assignment,
        )?;

        return Ok(exit_code);
    }

    let system_files_current = matches!(system_anchor_state, PfFileState::Current { .. })
        && matches!(system_reference_state, PfFileState::Current { .. });
    if system_files_current {
        let active_config = match environment
            .active_pf_redirect_config_with_privilege_mode(platform::PrivilegeMode::Interactive)
        {
            Ok(active_config) => active_config,
            Err(error) => {
                release_new_gateway_ports(
                    &mut database,
                    had_http_assignment,
                    had_https_assignment,
                    had_admin_assignment,
                )?;

                return Err(error.into());
            }
        };

        if active_config.as_ref() == Some(&config) {
            output.line("System pf redirect config already matches PV")?;
            refresh_gateway_observation_after_pf_repair(
                environment,
                &paths,
                &config,
                &mut database,
            )?;

            return Ok(ExitCode::SUCCESS);
        }
        output
            .line("System pf redirect config matches PV, but active redirects are not loaded.")?;
    }

    if let Err(error) = environment.install_pf_redirects(
        &prepared_anchor_path,
        &prepared_reference_path,
        &system_anchor_path,
        &system_pf_conf_path,
    ) {
        release_new_gateway_ports(
            &mut database,
            had_http_assignment,
            had_https_assignment,
            had_admin_assignment,
        )?;

        return Err(error.into());
    }
    ensure_active_gateway_ports(
        environment,
        &config,
        &mut database,
        had_http_assignment,
        had_https_assignment,
        had_admin_assignment,
    )?;
    refresh_gateway_observation_after_pf_repair(environment, &paths, &config, &mut database)?;
    output.line("Installed system pf redirect config")?;

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
    had_admin_assignment: bool,
) -> Result<(), ExecuteError> {
    let active_config = match environment
        .active_pf_redirect_config_with_privilege_mode(platform::PrivilegeMode::Interactive)
    {
        Ok(active_config) => active_config,
        Err(error) => {
            release_new_gateway_ports(
                database,
                had_http_assignment,
                had_https_assignment,
                had_admin_assignment,
            )?;

            return Err(error.into());
        }
    };

    if active_config.as_ref() == Some(config) {
        return Ok(());
    }

    release_new_gateway_ports(
        database,
        had_http_assignment,
        had_https_assignment,
        had_admin_assignment,
    )?;

    Err(CliError::PfRedirectsInactive.into())
}

fn release_new_gateway_ports(
    database: &mut Database,
    had_http_assignment: bool,
    had_https_assignment: bool,
    had_admin_assignment: bool,
) -> Result<(), ExecuteError> {
    if !had_http_assignment {
        database.release_port(PortOwner::Gateway(GatewayPort::Http))?;
    }
    if !had_https_assignment {
        database.release_port(PortOwner::Gateway(GatewayPort::Https))?;
    }
    if !had_admin_assignment {
        database.release_port(PortOwner::Gateway(GatewayPort::Admin))?;
    }

    Ok(())
}

pub(crate) fn uninstall(
    environment: &impl Environment,
    stdout: &mut impl Write,
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
    let mut output = Output::new(stdout, OutputMode::plain());

    write_delete_result(
        &mut output,
        "prepared pf anchor",
        &prepared_anchor_path,
        deleted_anchor,
    )?;
    write_delete_result(
        &mut output,
        "prepared pf.conf reference",
        &prepared_reference_path,
        deleted_reference,
    )?;

    if let Some(exit_code) =
        write_pf_uninstall_blocker(&mut output, &system_anchor_state, &system_reference_state)?
    {
        return Ok(exit_code);
    }

    if matches!(system_anchor_state, PfFileState::Missing { .. })
        && matches!(system_reference_state, PfFileState::Missing { .. })
    {
        output.line("System pf redirect config already absent")?;

        return Ok(ExitCode::SUCCESS);
    }

    environment.remove_pf_redirects(&system_anchor_path, &system_pf_conf_path, &candidate_dir)?;
    output.line("Removed PV-owned system pf redirect config")?;

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

fn write_pf_install_blocker(
    output: &mut Output<'_, impl Write>,
    anchor_state: &PfFileState<PfRedirectConfig>,
    reference_state: &PfFileState<PfConfReference>,
) -> io::Result<Option<ExitCode>> {
    match anchor_state {
        PfFileState::Conflict { path } => {
            output.line(&format!("System pf anchor is not PV-owned: {path}"))?;
            output.line("Leaving it in place.")?;
            return Ok(Some(ExitCode::FAILURE));
        }
        PfFileState::Unreadable { path, message } => {
            output.line(&format!("System pf anchor could not be inspected: {path}"))?;
            output.line(&format!("  {message}"))?;
            output.line("Leaving it in place.")?;
            return Ok(Some(ExitCode::FAILURE));
        }
        PfFileState::Missing { .. } | PfFileState::Current { .. } | PfFileState::Stale { .. } => {}
    }

    match reference_state {
        PfFileState::Conflict { path } => {
            output.line(&format!("System pf.conf reference is not PV-owned: {path}"))?;
            output.line("Leaving it in place.")?;
            Ok(Some(ExitCode::FAILURE))
        }
        PfFileState::Unreadable { path, message } => {
            output.line(&format!(
                "System pf.conf reference could not be inspected: {path}"
            ))?;
            output.line(&format!("  {message}"))?;
            output.line("Leaving it in place.")?;
            Ok(Some(ExitCode::FAILURE))
        }
        PfFileState::Missing { .. } | PfFileState::Current { .. } | PfFileState::Stale { .. } => {
            Ok(None)
        }
    }
}

fn write_pf_uninstall_blocker(
    output: &mut Output<'_, impl Write>,
    anchor_state: &PfFileState<PfRedirectConfig>,
    reference_state: &PfFileState<PfConfReference>,
) -> io::Result<Option<ExitCode>> {
    match anchor_state {
        PfFileState::Conflict { path } => {
            output.line(&format!("System pf anchor is not PV-owned: {path}"))?;
            output.line("Leaving it in place.")?;
            return Ok(Some(ExitCode::FAILURE));
        }
        PfFileState::Unreadable { path, message } => {
            output.line(&format!("System pf anchor could not be inspected: {path}"))?;
            output.line(&format!("  {message}"))?;
            output.line("Leaving it in place.")?;
            return Ok(Some(ExitCode::FAILURE));
        }
        PfFileState::Missing { .. } | PfFileState::Current { .. } | PfFileState::Stale { .. } => {}
    }

    match reference_state {
        PfFileState::Conflict { path } => {
            output.line(&format!("System pf.conf reference is not PV-owned: {path}"))?;
            output.line("Leaving it in place.")?;
            Ok(Some(ExitCode::FAILURE))
        }
        PfFileState::Unreadable { path, message } => {
            output.line(&format!(
                "System pf.conf reference could not be inspected: {path}"
            ))?;
            output.line(&format!("  {message}"))?;
            output.line("Leaving it in place.")?;
            Ok(Some(ExitCode::FAILURE))
        }
        PfFileState::Missing { .. } | PfFileState::Current { .. } | PfFileState::Stale { .. } => {
            Ok(None)
        }
    }
}

fn write_delete_result(
    output: &mut Output<'_, impl Write>,
    label: &str,
    path: &Utf8Path,
    deleted: bool,
) -> io::Result<()> {
    if deleted {
        output.line(&format!("Deleted {label}: {path}"))
    } else {
        output.line(&format!("{label} already absent: {path}"))
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
