use std::io;
use std::io::Write;
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use platform::{PfConfReference, PfFileState, PfRedirectConfig};
use state::{Database, GatewayPort, GatewayPortAssignments, PortOwner, PvPaths, StateError};

use crate::environment::Environment;
use crate::error::{CliError, ExecuteError};
use crate::output::{Output, OutputMode};

const LOW_PORTS: [u16; 2] = [80, 443];

pub(crate) fn status(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let prepared_anchor_path = paths.pf_anchor_config();
    let prepared_reference_path = paths.pf_conf_reference_config();
    let system_anchor_path = pf_anchor_path(environment)?;
    let system_pf_conf_path = pf_conf_path(environment)?;
    let prepared_anchor_state = platform::inspect_pf_anchor_file(&prepared_anchor_path, None);
    let prepared_reference_state =
        platform::inspect_pf_conf_reference(&prepared_reference_path, None);
    let expected_anchor = pf_config_from_anchor_state(&prepared_anchor_state);
    let expected_reference = pf_reference_from_state(&prepared_reference_state);
    let system_anchor_state =
        platform::inspect_pf_anchor_file(&system_anchor_path, expected_anchor.as_ref());
    let system_reference_state =
        platform::inspect_pf_conf_reference(&system_pf_conf_path, expected_reference.as_ref());
    let mut output = Output::new(stdout, OutputMode::plain());

    output.line("Port redirect status")?;
    write_pf_anchor_state(&mut output, "Prepared pf anchor", &prepared_anchor_state)?;
    write_pf_reference_state(
        &mut output,
        "Prepared pf.conf reference",
        &prepared_reference_state,
    )?;
    write_pf_anchor_state(&mut output, "System pf anchor", &system_anchor_state)?;
    write_pf_reference_state(
        &mut output,
        "System pf.conf reference",
        &system_reference_state,
    )?;

    Ok(ExitCode::SUCCESS)
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
        release_new_gateway_ports(&mut database, had_http_assignment, had_https_assignment)?;

        return Ok(exit_code);
    }

    let system_files_current = matches!(system_anchor_state, PfFileState::Current { .. })
        && matches!(system_reference_state, PfFileState::Current { .. });
    let active_config = match environment.active_pf_redirect_config() {
        Ok(active_config) => active_config,
        Err(error) => {
            release_new_gateway_ports(&mut database, had_http_assignment, had_https_assignment)?;

            return Err(error.into());
        }
    };

    if system_files_current && active_config.as_ref() == Some(&config) {
        output.line("System pf redirect config already matches PV")?;

        return Ok(ExitCode::SUCCESS);
    }
    if system_files_current {
        output
            .line("System pf redirect config matches PV, but active redirects are not loaded.")?;
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
    output.line("Installed system pf redirect config")?;

    Ok(ExitCode::SUCCESS)
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

fn pf_config_from_anchor_state(state: &PfFileState<PfRedirectConfig>) -> Option<PfRedirectConfig> {
    match state {
        PfFileState::Current { value, .. }
        | PfFileState::Stale {
            actual: Some(value),
            ..
        } => Some(value.clone()),
        PfFileState::Missing { .. }
        | PfFileState::Stale { actual: None, .. }
        | PfFileState::Conflict { .. }
        | PfFileState::Unreadable { .. } => None,
    }
}

fn pf_reference_from_state(state: &PfFileState<PfConfReference>) -> Option<PfConfReference> {
    match state {
        PfFileState::Current { value, .. }
        | PfFileState::Stale {
            actual: Some(value),
            ..
        } => Some(*value),
        PfFileState::Missing { .. }
        | PfFileState::Stale { actual: None, .. }
        | PfFileState::Conflict { .. }
        | PfFileState::Unreadable { .. } => None,
    }
}

fn write_pf_anchor_state(
    output: &mut Output<'_, impl Write>,
    label: &str,
    state: &PfFileState<PfRedirectConfig>,
) -> io::Result<()> {
    match state {
        PfFileState::Missing { path } => {
            output.line(&format!("{label}: missing"))?;
            output.line(&format!("  path: {path}"))
        }
        PfFileState::Current { path, value } => {
            output.line(&format!("{label}: current"))?;
            output.line(&format!("  path: {path}"))?;
            output.line(&format!(
                "  HTTP redirect: 127.0.0.1:80 -> 127.0.0.1:{}",
                value.http_port
            ))?;
            output.line(&format!(
                "  HTTPS redirect: 127.0.0.1:443 -> 127.0.0.1:{}",
                value.https_port
            ))
        }
        PfFileState::Stale {
            path,
            expected,
            actual,
        } => {
            output.line(&format!("{label}: stale"))?;
            output.line(&format!("  path: {path}"))?;
            write_optional_pf_config(output, "expected", expected.as_ref())?;
            write_optional_pf_config(output, "actual", actual.as_ref())
        }
        PfFileState::Conflict { path } => {
            output.line(&format!("{label}: not PV-owned"))?;
            output.line(&format!("  path: {path}"))
        }
        PfFileState::Unreadable { path, message } => {
            output.line(&format!("{label}: unreadable"))?;
            output.line(&format!("  path: {path}"))?;
            output.line(&format!("  {message}"))
        }
    }
}

fn write_pf_reference_state(
    output: &mut Output<'_, impl Write>,
    label: &str,
    state: &PfFileState<PfConfReference>,
) -> io::Result<()> {
    match state {
        PfFileState::Missing { path } => {
            output.line(&format!("{label}: missing"))?;
            output.line(&format!("  path: {path}"))
        }
        PfFileState::Current { path, .. } => {
            output.line(&format!("{label}: current"))?;
            output.line(&format!("  path: {path}"))?;
            output.line("  anchor: com.prvious.pv")
        }
        PfFileState::Stale { path, .. } => {
            output.line(&format!("{label}: stale"))?;
            output.line(&format!("  path: {path}"))?;
            output.line("  anchor: com.prvious.pv")
        }
        PfFileState::Conflict { path } => {
            output.line(&format!("{label}: not PV-owned"))?;
            output.line(&format!("  path: {path}"))
        }
        PfFileState::Unreadable { path, message } => {
            output.line(&format!("{label}: unreadable"))?;
            output.line(&format!("  path: {path}"))?;
            output.line(&format!("  {message}"))
        }
    }
}

fn write_optional_pf_config(
    output: &mut Output<'_, impl Write>,
    label: &str,
    config: Option<&PfRedirectConfig>,
) -> io::Result<()> {
    match config {
        Some(config) => {
            output.line(&format!("  {label} HTTP port: {}", config.http_port))?;
            output.line(&format!("  {label} HTTPS port: {}", config.https_port))
        }
        None => output.line(&format!("  {label}: unparseable")),
    }
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
