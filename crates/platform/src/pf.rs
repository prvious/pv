use std::collections::BTreeSet;
use std::io;
use std::net::IpAddr;

use camino::{Utf8Path, Utf8PathBuf};
use data_encoding::HEXLOWER;
use serde::{Deserialize, Serialize};

use crate::PlatformError;
use crate::command::{run_system_command, run_system_command_output};

pub const SYSTEM_PF_ANCHOR_PATH: &str = "/etc/pf.anchors/com.prvious.pv";
pub const SYSTEM_PF_CONF_PATH: &str = "/etc/pf.conf";
const PV_MARKER: &str = "# Managed by PV";
const PF_ANCHOR_SOURCE_MARKER: &str =
    "# Source: PV prepared pf anchor for /etc/pf.anchors/com.prvious.pv";
const PF_CONF_SOURCE_MARKER: &str = "# Source: PV prepared pf.conf reference for /etc/pf.conf";
const PF_RDR_ANCHOR_DIRECTIVE: &str = "rdr-anchor \"com.prvious.pv\"";
const LEGACY_PF_ANCHOR_DIRECTIVE: &str = "anchor \"com.prvious.pv\"";
const PF_LOAD_ANCHOR_DIRECTIVE: &str =
    "load anchor \"com.prvious.pv\" from \"/etc/pf.anchors/com.prvious.pv\"";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PfRedirectConfig {
    pub http_port: u16,
    pub https_port: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActivePfRedirectInspection {
    pub pf_enabled: bool,
    pub pv_config: Option<PfRedirectConfig>,
    pub pv_anchor_has_unparsed_rules: bool,
    pub resolved_target_ports: BTreeSet<u16>,
    pub has_unresolved_redirect_targets: bool,
}

#[derive(Copy, Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PfConfReference;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum PfFileState<T> {
    Missing {
        path: Utf8PathBuf,
    },
    Current {
        path: Utf8PathBuf,
        value: T,
    },
    Stale {
        path: Utf8PathBuf,
        expected: Option<T>,
        actual: Option<T>,
    },
    Conflict {
        path: Utf8PathBuf,
    },
    Unreadable {
        path: Utf8PathBuf,
        message: String,
    },
}

impl PfRedirectConfig {
    pub const fn new(http_port: u16, https_port: u16) -> Self {
        Self {
            http_port,
            https_port,
        }
    }

    pub fn render_anchor(&self) -> String {
        format!(
            "{PV_MARKER}\n{PF_ANCHOR_SOURCE_MARKER}\nrdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 80 -> 127.0.0.1 port {}\nrdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 443 -> 127.0.0.1 port {}\n",
            self.http_port, self.https_port
        )
    }

    pub fn parse_anchor(content: &str) -> Option<Self> {
        let mut http_port = None;
        let mut https_port = None;
        let mut active_line_count = 0;

        for line in content.lines().filter_map(active_pf_line) {
            active_line_count += 1;

            if let Some(port) = line.strip_prefix(
                "rdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 80 -> 127.0.0.1 port ",
            ) {
                if http_port.replace(port.parse::<u16>().ok()?).is_some() {
                    return None;
                }
                continue;
            }

            if let Some(port) = line.strip_prefix(
                "rdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 443 -> 127.0.0.1 port ",
            ) {
                if https_port.replace(port.parse::<u16>().ok()?).is_some() {
                    return None;
                }
                continue;
            }

            return None;
        }

        if active_line_count == 2 {
            Some(Self::new(http_port?, https_port?))
        } else {
            None
        }
    }

    pub fn parse_active_rules(content: &str) -> Option<Self> {
        let mut http_port = None;
        let mut https_port = None;

        for line in content.lines().filter_map(active_pf_line) {
            if let Some(port) = parse_active_redirect_port(line, 80) {
                if http_port.replace(port).is_some() {
                    return None;
                }
                continue;
            }

            if let Some(port) = parse_active_redirect_port(line, 443) {
                if https_port.replace(port).is_some() {
                    return None;
                }
                continue;
            }

            return None;
        }

        Some(Self::new(http_port?, https_port?))
    }
}

impl PfConfReference {
    pub fn render(self) -> String {
        let (rdr_directive, load_directive) = pf_conf_reference_directives();
        format!("{PV_MARKER}\n{PF_CONF_SOURCE_MARKER}\n{rdr_directive}\n{load_directive}\n")
    }

    pub fn parse_block(content: &str) -> Option<Self> {
        let mut has_rdr_anchor = false;
        let mut has_load = false;
        let mut active_line_count = 0;

        for line in content.lines().filter_map(active_pf_line) {
            active_line_count += 1;

            if line == PF_RDR_ANCHOR_DIRECTIVE {
                if has_rdr_anchor {
                    return None;
                }
                has_rdr_anchor = true;
                continue;
            }

            if line == PF_LOAD_ANCHOR_DIRECTIVE {
                if has_load {
                    return None;
                }
                has_load = true;
                continue;
            }

            if is_pv_pf_conf_reference_directive(line) {
                return None;
            }

            return None;
        }

        if active_line_count == 2 && has_rdr_anchor && has_load {
            Some(Self)
        } else {
            None
        }
    }
}

pub fn inspect_pf_anchor_file(
    path: &Utf8Path,
    expected: Option<&PfRedirectConfig>,
) -> PfFileState<PfRedirectConfig> {
    inspect_pv_file(path, expected, PfRedirectConfig::parse_anchor, true)
}

pub fn inspect_pf_conf_reference(
    path: &Utf8Path,
    expected: Option<&PfConfReference>,
) -> PfFileState<PfConfReference> {
    let content = match state::fs::read_to_string(path) {
        Ok(content) => content,
        Err(state::StateError::Filesystem { source, .. })
            if source.kind() == io::ErrorKind::NotFound =>
        {
            return PfFileState::Missing {
                path: path.to_path_buf(),
            };
        }
        Err(error) => {
            return PfFileState::Unreadable {
                path: path.to_path_buf(),
                message: error.to_string(),
            };
        }
    };

    let has_pv_marker = content.lines().any(|line| line.trim() == PV_MARKER);
    let has_anchor_directive = content
        .lines()
        .filter_map(active_pf_line)
        .any(is_pv_pf_conf_reference_directive);

    if !has_pv_marker {
        return if has_anchor_directive {
            PfFileState::Conflict {
                path: path.to_path_buf(),
            }
        } else {
            PfFileState::Missing {
                path: path.to_path_buf(),
            }
        };
    }

    let actual = parse_embedded_pf_conf_reference(&content);
    classify_pv_file_state(path, expected, actual)
}

pub fn active_pf_redirect_config() -> Result<Option<PfRedirectConfig>, PlatformError> {
    match active_pf_redirect_config_unprivileged_with_runner(&mut run_system_command_output) {
        Ok(config) => Ok(config),
        Err(_unprivileged_error) => crate::PrivilegedHelperClient
            .inspect_pf()
            .map(|inspection| inspection.pv_config),
    }
}

pub fn inspect_active_pf_redirects_unprivileged()
-> Result<ActivePfRedirectInspection, PlatformError> {
    inspect_active_pf_redirects_unprivileged_with_runner(&mut run_system_command_output)
}

fn active_pf_redirect_config_unprivileged_with_runner(
    run_system_output: &mut impl FnMut(&str, &[&str]) -> Result<String, PlatformError>,
) -> Result<Option<PfRedirectConfig>, PlatformError> {
    let main_nat_rules = run_system_output("/sbin/pfctl", &["-s", "nat"])?;

    if !main_nat_rules_load_pv_rdr_anchor(&main_nat_rules) {
        return Ok(None);
    }

    let anchor_nat_rules =
        run_system_output("/sbin/pfctl", &["-a", "com.prvious.pv", "-s", "nat"])?;

    Ok(PfRedirectConfig::parse_active_rules(&anchor_nat_rules))
}

fn inspect_active_pf_redirects_unprivileged_with_runner(
    run_system_output: &mut impl FnMut(&str, &[&str]) -> Result<String, PlatformError>,
) -> Result<ActivePfRedirectInspection, PlatformError> {
    let pf_info = run_system_output("/sbin/pfctl", &["-s", "info"])?;
    let pf_enabled = parse_pf_enabled(&pf_info)?;
    let main_nat_rules = run_system_output("/sbin/pfctl", &["-s", "nat"])?;
    let (pv_config, pv_anchor_has_unparsed_rules) =
        if main_nat_rules_load_pv_rdr_anchor(&main_nat_rules) {
            let anchor_nat_rules =
                run_system_output("/sbin/pfctl", &["-a", "com.prvious.pv", "-s", "nat"])?;
            let pv_config = PfRedirectConfig::parse_active_rules(&anchor_nat_rules);
            let has_rules = anchor_nat_rules
                .lines()
                .any(|line| active_pf_line(line).is_some());
            let has_unparsed_rules = pv_config.is_none() && has_rules;

            (pv_config, has_unparsed_rules)
        } else {
            (None, false)
        };
    let recursive_nat_rules = run_system_output("/sbin/pfctl", &["-a", "*", "-s", "nat"])?;

    let (resolved_target_ports, has_unresolved_redirect_targets) =
        inspect_redirect_targets(&recursive_nat_rules);

    Ok(ActivePfRedirectInspection {
        pf_enabled,
        pv_config,
        pv_anchor_has_unparsed_rules,
        resolved_target_ports,
        has_unresolved_redirect_targets,
    })
}

fn parse_pf_enabled(info: &str) -> Result<bool, PlatformError> {
    for line in info.lines().filter_map(active_pf_line) {
        let Some(status) = line.strip_prefix("Status:") else {
            continue;
        };
        match status.split_whitespace().next() {
            Some("Enabled") => return Ok(true),
            Some("Disabled") => return Ok(false),
            _ => break,
        }
    }

    Err(PlatformError::SystemIntegration(
        "pfctl status did not report whether PF is enabled".to_string(),
    ))
}

fn inspect_redirect_targets(rules: &str) -> (BTreeSet<u16>, bool) {
    let mut resolved_target_ports = BTreeSet::new();
    let mut has_unresolved_redirect_targets = false;

    for line in rules
        .lines()
        .filter_map(active_pf_line)
        .filter(|line| line.starts_with("rdr "))
    {
        match redirect_target(line) {
            RedirectTarget::ResolvedPort(port) => {
                resolved_target_ports.insert(port);
            }
            RedirectTarget::Unresolved => has_unresolved_redirect_targets = true,
        }
    }

    (resolved_target_ports, has_unresolved_redirect_targets)
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum RedirectTarget {
    ResolvedPort(u16),
    Unresolved,
}

fn redirect_target(rule: &str) -> RedirectTarget {
    let Some((_source, target)) = rule.rsplit_once(" -> ") else {
        return RedirectTarget::Unresolved;
    };
    let mut components = target.split_whitespace();
    let Some(address) = components.next() else {
        return RedirectTarget::Unresolved;
    };
    if address.parse::<IpAddr>().is_err() {
        return RedirectTarget::Unresolved;
    }
    if components.next() != Some("port") {
        return RedirectTarget::Unresolved;
    }
    let Some(port) = components.next() else {
        return RedirectTarget::Unresolved;
    };
    let port = if port == "=" {
        let Some(port) = components.next() else {
            return RedirectTarget::Unresolved;
        };
        port
    } else {
        port
    };
    let Ok(port) = port.parse::<u16>() else {
        return RedirectTarget::Unresolved;
    };

    RedirectTarget::ResolvedPort(port)
}

pub fn install_pf_redirects(
    prepared_anchor_path: &Utf8Path,
    prepared_reference_path: &Utf8Path,
    system_anchor_path: &Utf8Path,
    system_pf_conf_path: &Utf8Path,
) -> Result<(), PlatformError> {
    require_fixed_pf_paths(system_anchor_path, system_pf_conf_path)?;
    let prepared_anchor = read_platform_file(prepared_anchor_path)?;
    let prepared_reference = read_platform_file(prepared_reference_path)?;
    let config = PfRedirectConfig::parse_anchor(&prepared_anchor).ok_or_else(|| {
        PlatformError::SystemIntegration(format!(
            "prepared pf anchor is not a valid PV anchor: {prepared_anchor_path}"
        ))
    })?;
    if PfConfReference::parse_block(&prepared_reference).is_none() {
        return Err(PlatformError::SystemIntegration(format!(
            "prepared pf.conf reference is not valid: {prepared_reference_path}"
        )));
    }

    crate::PrivilegedHelperClient.apply_pf(&config)
}

#[cfg(test)]
fn install_pf_redirects_with_runner(
    prepared_anchor_path: &Utf8Path,
    prepared_reference_path: &Utf8Path,
    system_anchor_path: &Utf8Path,
    system_pf_conf_path: &Utf8Path,
    run_system: &mut impl FnMut(&str, &[&str]) -> Result<(), PlatformError>,
) -> Result<(), PlatformError> {
    let runner = std::cell::RefCell::new(run_system);
    install_pf_redirects_and_verify_with_runner(
        PfInstallPaths {
            prepared_anchor: prepared_anchor_path,
            prepared_reference: prepared_reference_path,
            system_anchor: system_anchor_path,
            system_pf_conf: system_pf_conf_path,
        },
        &mut |program, args| runner.borrow_mut()(program, args),
        &mut || {
            runner.borrow_mut()("/sbin/pfctl", &["-E"])?;
            Ok("Token : 1\n".to_string())
        },
        true,
        &mut || Ok(()),
    )
}

#[derive(Clone, Copy)]
struct PfInstallPaths<'path> {
    prepared_anchor: &'path Utf8Path,
    prepared_reference: &'path Utf8Path,
    system_anchor: &'path Utf8Path,
    system_pf_conf: &'path Utf8Path,
}

fn install_pf_redirects_and_verify_with_runner(
    paths: PfInstallPaths<'_>,
    run_system: &mut impl FnMut(&str, &[&str]) -> Result<(), PlatformError>,
    enable_pf: &mut impl FnMut() -> Result<String, PlatformError>,
    pf_was_enabled: bool,
    verify_active: &mut impl FnMut() -> Result<(), PlatformError>,
) -> Result<(), PlatformError> {
    let PfInstallPaths {
        prepared_anchor: prepared_anchor_path,
        prepared_reference: prepared_reference_path,
        system_anchor: system_anchor_path,
        system_pf_conf: system_pf_conf_path,
    } = paths;
    let prepared_anchor = read_platform_file(prepared_anchor_path)?;
    let prepared_reference = read_platform_file(prepared_reference_path)?;
    if PfRedirectConfig::parse_anchor(&prepared_anchor).is_none() {
        return Err(PlatformError::SystemIntegration(format!(
            "prepared pf anchor is not a valid PV anchor: {prepared_anchor_path}"
        )));
    }
    if PfConfReference::parse_block(&prepared_reference).is_none() {
        return Err(PlatformError::SystemIntegration(format!(
            "prepared pf.conf reference is not valid: {prepared_reference_path}"
        )));
    }

    let system_reference_state =
        inspect_pf_conf_reference(system_pf_conf_path, Some(&PfConfReference));
    let candidate = match system_reference_state {
        PfFileState::Missing { .. } => {
            let current = read_optional_platform_file(system_pf_conf_path)?;
            append_pf_reference(current.as_deref().unwrap_or_default(), &prepared_reference)
        }
        PfFileState::Current { .. } => read_platform_file(system_pf_conf_path)?,
        PfFileState::Stale { .. } => {
            let current = read_platform_file(system_pf_conf_path)?;
            append_pf_reference(&remove_pf_reference_lines(&current), &prepared_reference)
        }
        PfFileState::Conflict { path } => {
            return Err(PlatformError::SystemIntegration(format!(
                "system pf.conf reference is not PV-owned: {path}"
            )));
        }
        PfFileState::Unreadable { path, message } => {
            return Err(PlatformError::SystemIntegration(format!(
                "system pf.conf reference could not be inspected: {path}: {message}"
            )));
        }
    };
    let candidate_path = prepared_reference_path.with_file_name("pf.conf.candidate");
    let anchor_backup = read_optional_platform_file(system_anchor_path)?;
    let anchor_backup_path = prepared_anchor_path.with_file_name("pf.anchor.rollback");
    let pf_conf_backup = read_optional_platform_file(system_pf_conf_path)?;
    let pf_conf_backup_path = prepared_reference_path.with_file_name("pf.conf.rollback");

    state::fs::write_sensitive_file(&candidate_path, &candidate)
        .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?;
    let mut reload_attempted = false;
    let post_anchor_result = (|| {
        install_pf_anchor_with_runner(prepared_anchor_path, system_anchor_path, run_system)?;
        run_system("/sbin/pfctl", &["-nf", candidate_path.as_str()])?;
        run_system(
            "/usr/bin/install",
            &[
                "-m",
                "0644",
                candidate_path.as_str(),
                system_pf_conf_path.as_str(),
            ],
        )?;
        reload_attempted = true;
        reload_pf_with_runner(system_pf_conf_path, run_system)?;
        verify_active()?;
        if !pf_was_enabled {
            enable_pf()?;
        }
        Ok(())
    })();

    if let Err(error) = post_anchor_result {
        let mut rollback_errors = Vec::new();

        if let Err(rollback_error) = rollback_pf_conf_with_runner(
            system_pf_conf_path,
            pf_conf_backup.as_deref(),
            &pf_conf_backup_path,
            run_system,
        ) {
            rollback_errors.push(format!("pf.conf: {rollback_error}"));
        }
        if let Err(rollback_error) = rollback_pf_anchor_with_runner(
            system_anchor_path,
            anchor_backup.as_deref(),
            &anchor_backup_path,
            run_system,
        ) {
            rollback_errors.push(format!("pf anchor: {rollback_error}"));
        }
        if reload_attempted {
            let active_rollback = if pf_conf_backup.is_some() {
                reload_pf_with_runner(system_pf_conf_path, run_system)
            } else {
                run_system("/sbin/pfctl", &["-a", "com.prvious.pv", "-F", "all"])
            };
            if let Err(rollback_error) = active_rollback {
                rollback_errors.push(format!("active PF state: {rollback_error}"));
            }
        }

        if !rollback_errors.is_empty() {
            return Err(PlatformError::SystemIntegration(format!(
                "{error}; additionally failed to rollback {}",
                rollback_errors.join("; ")
            )));
        }

        return Err(error);
    }

    Ok(())
}

fn rollback_pf_anchor_with_runner(
    system_anchor_path: &Utf8Path,
    anchor_backup: Option<&str>,
    anchor_backup_path: &Utf8Path,
    run_system: &mut impl FnMut(&str, &[&str]) -> Result<(), PlatformError>,
) -> Result<(), PlatformError> {
    if let Some(anchor_backup) = anchor_backup {
        state::fs::write_sensitive_file(anchor_backup_path, anchor_backup)
            .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?;
        run_system(
            "/usr/bin/install",
            &[
                "-m",
                "0644",
                anchor_backup_path.as_str(),
                system_anchor_path.as_str(),
            ],
        )?;
        state::fs::delete_file(anchor_backup_path)
            .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?;

        return Ok(());
    }

    run_system("/bin/rm", &["-f", system_anchor_path.as_str()])
}

fn rollback_pf_conf_with_runner(
    system_pf_conf_path: &Utf8Path,
    pf_conf_backup: Option<&str>,
    pf_conf_backup_path: &Utf8Path,
    run_system: &mut impl FnMut(&str, &[&str]) -> Result<(), PlatformError>,
) -> Result<(), PlatformError> {
    if let Some(pf_conf_backup) = pf_conf_backup {
        state::fs::write_sensitive_file(pf_conf_backup_path, pf_conf_backup)
            .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?;
        run_system(
            "/usr/bin/install",
            &[
                "-m",
                "0644",
                pf_conf_backup_path.as_str(),
                system_pf_conf_path.as_str(),
            ],
        )?;
        state::fs::delete_file(pf_conf_backup_path)
            .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?;

        return Ok(());
    }

    run_system("/bin/rm", &["-f", system_pf_conf_path.as_str()])
}

pub fn remove_pf_redirects(
    system_anchor_path: &Utf8Path,
    system_pf_conf_path: &Utf8Path,
    candidate_dir: &Utf8Path,
) -> Result<(), PlatformError> {
    let _candidate_dir = candidate_dir;
    require_fixed_pf_paths(system_anchor_path, system_pf_conf_path)?;
    crate::PrivilegedHelperClient.remove_pf()
}

fn require_fixed_pf_paths(
    system_anchor_path: &Utf8Path,
    system_pf_conf_path: &Utf8Path,
) -> Result<(), PlatformError> {
    if system_anchor_path == Utf8Path::new(SYSTEM_PF_ANCHOR_PATH)
        && system_pf_conf_path == Utf8Path::new(SYSTEM_PF_CONF_PATH)
    {
        return Ok(());
    }

    Err(PlatformError::SystemIntegration(format!(
        "PF mutation requires fixed system paths {SYSTEM_PF_ANCHOR_PATH} and {SYSTEM_PF_CONF_PATH}"
    )))
}

#[cfg(target_os = "macos")]
pub(crate) fn inspect_active_pf_redirects_privileged()
-> Result<ActivePfRedirectInspection, PlatformError> {
    inspect_active_pf_redirects_unprivileged_with_runner(&mut run_system_command_output)
}

#[cfg(target_os = "macos")]
pub(crate) fn apply_pf_redirects_privileged(
    config: &PfRedirectConfig,
) -> Result<(), PlatformError> {
    let system_anchor_path = Utf8Path::new(SYSTEM_PF_ANCHOR_PATH);
    let system_pf_conf_path = Utf8Path::new(SYSTEM_PF_CONF_PATH);
    crate::helper::validate_root_owned_file_if_present(system_anchor_path)?;
    crate::helper::validate_root_owned_file_if_present(system_pf_conf_path)?;
    let pf_was_enabled = inspect_active_pf_redirects_privileged()?.pf_enabled;

    let work_dir = Utf8Path::new("/Library/Application Support/PV/pf");
    let prepared_anchor_path = work_dir.join("com.prvious.pv");
    let prepared_reference_path = work_dir.join("pf.conf.reference");
    write_temporary_file(&prepared_anchor_path, &config.render_anchor())?;
    write_temporary_file(&prepared_reference_path, &PfConfReference.render())?;
    install_pf_redirects_and_verify_with_runner(
        PfInstallPaths {
            prepared_anchor: &prepared_anchor_path,
            prepared_reference: &prepared_reference_path,
            system_anchor: system_anchor_path,
            system_pf_conf: system_pf_conf_path,
        },
        &mut run_system_command,
        &mut || run_system_command_output("/sbin/pfctl", &["-E"]),
        pf_was_enabled,
        &mut || {
            let active =
                active_pf_redirect_config_unprivileged_with_runner(&mut run_system_command_output)?;
            if active.as_ref() == Some(config) {
                Ok(())
            } else {
                Err(PlatformError::SystemIntegration(
                    "PF redirects did not match after helper apply".to_string(),
                ))
            }
        },
    )
}

#[cfg(target_os = "macos")]
pub(crate) fn reload_pf_redirects_privileged() -> Result<(), PlatformError> {
    let system_anchor_path = Utf8Path::new(SYSTEM_PF_ANCHOR_PATH);
    let system_pf_conf_path = Utf8Path::new(SYSTEM_PF_CONF_PATH);
    match inspect_pf_anchor_file(system_anchor_path, None) {
        PfFileState::Current { .. } => {}
        state => {
            return Err(PlatformError::SystemIntegration(format!(
                "PV PF anchor is not reloadable: {state:?}"
            )));
        }
    }
    match inspect_pf_conf_reference(system_pf_conf_path, None) {
        PfFileState::Current { .. } => {}
        state => {
            return Err(PlatformError::SystemIntegration(format!(
                "PV pf.conf reference is not reloadable: {state:?}"
            )));
        }
    }
    crate::helper::validate_root_owned_file_if_present(system_anchor_path)?;
    crate::helper::validate_root_owned_file_if_present(system_pf_conf_path)?;
    reload_pf_with_runner(system_pf_conf_path, &mut run_system_command)
}

#[cfg(target_os = "macos")]
pub(crate) fn remove_pf_redirects_privileged() -> Result<(), PlatformError> {
    let system_anchor_path = Utf8Path::new(SYSTEM_PF_ANCHOR_PATH);
    let system_pf_conf_path = Utf8Path::new(SYSTEM_PF_CONF_PATH);
    crate::helper::validate_root_owned_file_if_present(system_anchor_path)?;
    crate::helper::validate_root_owned_file_if_present(system_pf_conf_path)?;
    remove_pf_redirects_with_runner(
        system_anchor_path,
        system_pf_conf_path,
        Utf8Path::new("/Library/Application Support/PV/pf"),
        &mut run_system_command,
    )
}

fn remove_pf_redirects_with_runner(
    system_anchor_path: &Utf8Path,
    system_pf_conf_path: &Utf8Path,
    candidate_dir: &Utf8Path,
    run_system: &mut impl FnMut(&str, &[&str]) -> Result<(), PlatformError>,
) -> Result<(), PlatformError> {
    let system_reference_state = inspect_pf_conf_reference(system_pf_conf_path, None);

    match system_reference_state {
        PfFileState::Missing { .. } => {}
        PfFileState::Current { .. } | PfFileState::Stale { .. } => {
            let current = read_platform_file(system_pf_conf_path)?;
            let candidate = remove_pf_reference_lines(&current);
            let candidate_path = temporary_pf_conf_candidate_path(candidate_dir)?;

            write_temporary_file(&candidate_path, &candidate)?;
            let update_result = (|| {
                run_system("/sbin/pfctl", &["-nf", candidate_path.as_str()])?;
                run_system(
                    "/usr/bin/install",
                    &[
                        "-m",
                        "0644",
                        candidate_path.as_str(),
                        system_pf_conf_path.as_str(),
                    ],
                )
            })();
            let cleanup_result = state::fs::delete_file(&candidate_path)
                .map_err(|error| PlatformError::SystemIntegration(error.to_string()));
            match (update_result, cleanup_result) {
                (Ok(()), Ok(())) => {}
                (Ok(()), Err(error)) | (Err(error), Ok(())) => return Err(error),
                (Err(error), Err(cleanup_error)) => {
                    return Err(PlatformError::SystemIntegration(format!(
                        "{error}; additionally failed to remove PF candidate: {cleanup_error}"
                    )));
                }
            }
        }
        PfFileState::Conflict { path } => {
            return Err(PlatformError::SystemIntegration(format!(
                "system pf.conf reference is not PV-owned: {path}"
            )));
        }
        PfFileState::Unreadable { path, message } => {
            return Err(PlatformError::SystemIntegration(format!(
                "system pf.conf reference could not be inspected: {path}: {message}"
            )));
        }
    }

    run_system("/bin/rm", &["-f", system_anchor_path.as_str()])?;
    reload_pf_with_runner(system_pf_conf_path, run_system)
}

fn inspect_pv_file<T: Clone + Eq>(
    path: &Utf8Path,
    expected: Option<&T>,
    parse: impl FnOnce(&str) -> Option<T>,
    conflict_when_unmarked: bool,
) -> PfFileState<T> {
    let content = match state::fs::read_to_string(path) {
        Ok(content) => content,
        Err(state::StateError::Filesystem { source, .. })
            if source.kind() == io::ErrorKind::NotFound =>
        {
            return PfFileState::Missing {
                path: path.to_path_buf(),
            };
        }
        Err(error) => {
            return PfFileState::Unreadable {
                path: path.to_path_buf(),
                message: error.to_string(),
            };
        }
    };

    if !content.lines().any(|line| line.trim() == PV_MARKER) && conflict_when_unmarked {
        return PfFileState::Conflict {
            path: path.to_path_buf(),
        };
    }

    let actual = parse(&content);
    classify_pv_file_state(path, expected, actual)
}

fn classify_pv_file_state<T: Clone + Eq>(
    path: &Utf8Path,
    expected: Option<&T>,
    actual: Option<T>,
) -> PfFileState<T> {
    match (expected, actual) {
        (Some(expected), Some(actual)) if expected == &actual => PfFileState::Current {
            path: path.to_path_buf(),
            value: actual,
        },
        (Some(expected), actual) => PfFileState::Stale {
            path: path.to_path_buf(),
            expected: Some(expected.clone()),
            actual,
        },
        (None, Some(actual)) => PfFileState::Current {
            path: path.to_path_buf(),
            value: actual,
        },
        (None, None) => PfFileState::Stale {
            path: path.to_path_buf(),
            expected: None,
            actual: None,
        },
    }
}

fn active_pf_line(line: &str) -> Option<&str> {
    let line = line.trim();

    if line.is_empty() || line.starts_with('#') {
        None
    } else {
        Some(line)
    }
}

fn is_pv_pf_conf_reference_directive(line: &str) -> bool {
    line.starts_with("rdr-anchor \"com.prvious.pv\"")
        || line.starts_with("anchor \"com.prvious.pv\"")
        || line.starts_with("load anchor \"com.prvious.pv\"")
}

fn parse_active_redirect_port(line: &str, public_port: u16) -> Option<u16> {
    let prefix = "rdr pass on lo0 inet proto tcp from any to 127.0.0.1 port ";
    let tail = line.strip_prefix(prefix)?;
    let tail = tail
        .strip_prefix(&format!("{public_port} -> "))
        .or_else(|| tail.strip_prefix(&format!("= {public_port} -> ")))?;
    let redirect_port = tail.rsplit_once(" port ")?.1;
    let redirect_port = redirect_port.split_whitespace().next()?;

    redirect_port.parse::<u16>().ok()
}

fn main_nat_rules_load_pv_rdr_anchor(content: &str) -> bool {
    content
        .lines()
        .filter_map(active_pf_line)
        .any(is_loaded_pv_rdr_anchor_rule)
}

fn is_loaded_pv_rdr_anchor_rule(line: &str) -> bool {
    let Some(tail) = line.strip_prefix("rdr-anchor \"com.prvious.pv\"") else {
        return false;
    };

    tail.is_empty() || tail.starts_with(' ')
}

fn parse_embedded_pf_conf_reference(content: &str) -> Option<PfConfReference> {
    let mut has_rdr_anchor = false;
    let mut has_load = false;

    for line in content.lines().filter_map(active_pf_line) {
        if line == PF_RDR_ANCHOR_DIRECTIVE {
            if has_rdr_anchor {
                return None;
            }
            has_rdr_anchor = true;
            continue;
        }

        if line == PF_LOAD_ANCHOR_DIRECTIVE {
            if has_load {
                return None;
            }
            has_load = true;
            continue;
        }

        if is_pv_pf_conf_reference_directive(line) {
            return None;
        }
    }

    if has_rdr_anchor && has_load {
        Some(PfConfReference)
    } else {
        None
    }
}

fn install_pf_anchor_with_runner(
    prepared_anchor_path: &Utf8Path,
    system_anchor_path: &Utf8Path,
    run_system: &mut impl FnMut(&str, &[&str]) -> Result<(), PlatformError>,
) -> Result<(), PlatformError> {
    let parent = system_anchor_path.parent().ok_or_else(|| {
        PlatformError::SystemIntegration(format!(
            "pf anchor path has no parent directory: {system_anchor_path}"
        ))
    })?;

    run_system("/sbin/pfctl", &["-nf", prepared_anchor_path.as_str()])?;
    run_system("/bin/mkdir", &["-p", parent.as_str()])?;
    run_system(
        "/usr/bin/install",
        &[
            "-m",
            "0644",
            prepared_anchor_path.as_str(),
            system_anchor_path.as_str(),
        ],
    )
}

fn reload_pf_with_runner(
    system_pf_conf_path: &Utf8Path,
    run_system: &mut impl FnMut(&str, &[&str]) -> Result<(), PlatformError>,
) -> Result<(), PlatformError> {
    run_system("/sbin/pfctl", &["-f", system_pf_conf_path.as_str()])
}

fn append_pf_reference(content: &str, reference: &str) -> String {
    if content.trim().is_empty() {
        return reference.to_string();
    }

    let mut candidate = content.to_string();
    if !candidate.ends_with('\n') {
        candidate.push('\n');
    }
    let (rdr_directive, load_directive) = pf_conf_reference_directives();
    let rdr_reference = format!("{PV_MARKER}\n{PF_CONF_SOURCE_MARKER}\n{rdr_directive}\n");
    let load_reference = format!("{PV_MARKER}\n{PF_CONF_SOURCE_MARKER}\n{load_directive}\n");

    if let Some(index) = first_pf_filter_rule_index(&candidate) {
        candidate.insert_str(index, &rdr_reference);
    } else {
        candidate.push_str(&rdr_reference);
    }

    if !candidate.ends_with('\n') {
        candidate.push('\n');
    }
    candidate.push_str(&load_reference);
    candidate
}

fn pf_conf_reference_directives() -> (&'static str, &'static str) {
    (PF_RDR_ANCHOR_DIRECTIVE, PF_LOAD_ANCHOR_DIRECTIVE)
}

fn first_pf_filter_rule_index(content: &str) -> Option<usize> {
    let mut offset = 0;
    for line in content.split_inclusive('\n') {
        if let Some(active_line) = active_pf_line(line)
            && is_pf_filter_rule(active_line)
        {
            return Some(offset);
        }
        offset += line.len();
    }

    None
}

fn is_pf_filter_rule(line: &str) -> bool {
    line == "block"
        || line.starts_with("block ")
        || line == "pass"
        || line.starts_with("pass ")
        || line == "dummynet-anchor"
        || line.starts_with("dummynet-anchor ")
        || line.starts_with("anchor ")
        || line.starts_with("antispoof ")
}

fn remove_pf_reference_lines(content: &str) -> String {
    let mut candidate = String::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if matches!(
            trimmed,
            PV_MARKER
                | PF_CONF_SOURCE_MARKER
                | PF_RDR_ANCHOR_DIRECTIVE
                | LEGACY_PF_ANCHOR_DIRECTIVE
                | PF_LOAD_ANCHOR_DIRECTIVE
        ) {
            continue;
        }

        candidate.push_str(line);
        candidate.push('\n');
    }

    candidate
}

fn read_platform_file(path: &Utf8Path) -> Result<String, PlatformError> {
    state::fs::read_to_string(path)
        .map_err(|error| PlatformError::SystemIntegration(error.to_string()))
}

fn read_optional_platform_file(path: &Utf8Path) -> Result<Option<String>, PlatformError> {
    match state::fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(state::StateError::Filesystem { source, .. })
            if source.kind() == io::ErrorKind::NotFound =>
        {
            Ok(None)
        }
        Err(error) => Err(PlatformError::SystemIntegration(error.to_string())),
    }
}

fn temporary_pf_conf_candidate_path(
    candidate_dir: &Utf8Path,
) -> Result<Utf8PathBuf, PlatformError> {
    let mut suffix = [0_u8; 16];
    getrandom::fill(&mut suffix)
        .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?;

    Ok(candidate_dir.join(format!("pv-pf-conf-{}-uninstall", HEXLOWER.encode(&suffix))))
}

fn write_temporary_file(path: &Utf8Path, content: &str) -> Result<(), PlatformError> {
    state::fs::write_sensitive_file(path, content)
        .map_err(|error| PlatformError::SystemIntegration(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeSet;

    use camino::Utf8Path;
    use camino_tempfile::tempdir;

    use super::{
        ActivePfRedirectInspection, PfConfReference, PfInstallPaths, PfRedirectConfig,
        inspect_active_pf_redirects_unprivileged_with_runner,
        install_pf_redirects_and_verify_with_runner, install_pf_redirects_with_runner,
        read_platform_file, remove_pf_redirects_with_runner, temporary_pf_conf_candidate_path,
    };

    #[test]
    fn active_pf_redirect_inspection_never_invokes_sudo() {
        let mut commands = Vec::new();

        let result = inspect_active_pf_redirects_unprivileged_with_runner(&mut |program, args| {
            let command = format!("{program} {}", args.join(" "));
            commands.push(command.clone());

            Err(crate::PlatformError::SystemIntegrationCommandStatus {
                command,
                status: "exit status: 1".to_string(),
            })
        });

        assert!(matches!(
            result,
            Err(crate::PlatformError::SystemIntegrationCommandStatus { .. })
        ));
        assert_eq!(commands, ["/sbin/pfctl -s info"]);
    }

    #[test]
    fn active_pf_redirect_inspection_reports_recursive_resolved_targets() -> anyhow::Result<()> {
        let mut commands = Vec::new();

        let inspection = inspect_active_pf_redirects_unprivileged_with_runner(
            &mut |program, args| {
                let command = format!("{program} {}", args.join(" "));
                commands.push(command.clone());

                match command.as_str() {
                    "/sbin/pfctl -s info" => {
                        Ok("Status: Enabled for 0 days 00:00:01\n".to_string())
                    }
                    "/sbin/pfctl -s nat" => Ok(
                        "rdr-anchor \"com.prvious.pv\" all\nrdr-anchor \"other\" all\n".to_string(),
                    ),
                    "/sbin/pfctl -a com.prvious.pv -s nat" => Ok(
                        "rdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 80 -> 127.0.0.1 port 48080\nrdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 443 -> 127.0.0.1 port 48443\n"
                            .to_string(),
                    ),
                    _ => Ok(
                        "rdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 80 -> 127.0.0.1 port 48080\nrdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 8080 -> 127.0.0.1 port = 45080 round-robin\nrdr pass on lo0 inet6 proto tcp from any to ::1 port 8081 -> ::1 port 46080\n"
                            .to_string(),
                    ),
                }
            },
        )?;

        assert_eq!(
            inspection,
            ActivePfRedirectInspection {
                pf_enabled: true,
                pv_config: Some(PfRedirectConfig::new(48080, 48443)),
                pv_anchor_has_unparsed_rules: false,
                resolved_target_ports: [45080, 46080, 48080].into_iter().collect(),
                has_unresolved_redirect_targets: false,
            }
        );
        assert_eq!(
            commands,
            [
                "/sbin/pfctl -s info",
                "/sbin/pfctl -s nat",
                "/sbin/pfctl -a com.prvious.pv -s nat",
                "/sbin/pfctl -a * -s nat",
            ]
        );

        Ok(())
    }

    #[test]
    fn active_pf_redirect_inspection_preserves_unresolved_targets() -> anyhow::Result<()> {
        let inspection = inspect_active_pf_redirects_unprivileged_with_runner(
            &mut |_program, args| {
                if args == ["-s", "info"] {
                    return Ok("Status: Enabled for 0 days 00:00:01\n".to_string());
                }
                if args == ["-s", "nat"] {
                    return Ok(String::new());
                }

                Ok(
                    "rdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 80 -> <backends> port 45080\nrdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 443 -> 127.0.0.1 port 45443:45444\n"
                        .to_string(),
                )
            },
        )?;

        assert_eq!(
            inspection,
            ActivePfRedirectInspection {
                pf_enabled: true,
                pv_config: None,
                pv_anchor_has_unparsed_rules: false,
                resolved_target_ports: BTreeSet::new(),
                has_unresolved_redirect_targets: true,
            }
        );

        Ok(())
    }

    #[test]
    fn active_pf_redirect_inspection_preserves_disabled_partial_anchor() -> anyhow::Result<()> {
        let partial_rule = "rdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 80 -> 127.0.0.1 port 45080\n";
        let inspection = inspect_active_pf_redirects_unprivileged_with_runner(
            &mut |_program, args| match args {
                ["-s", "info"] => Ok("Status: Disabled\n".to_string()),
                ["-s", "nat"] => Ok("rdr-anchor \"com.prvious.pv\" all\n".to_string()),
                _ => Ok(partial_rule.to_string()),
            },
        )?;

        assert_eq!(
            inspection,
            ActivePfRedirectInspection {
                pf_enabled: false,
                pv_config: None,
                pv_anchor_has_unparsed_rules: true,
                resolved_target_ports: BTreeSet::from([45080]),
                has_unresolved_redirect_targets: false,
            }
        );

        Ok(())
    }

    #[test]
    fn install_pf_redirects_validates_candidate_after_installing_anchor() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let prepared_anchor_path = tempdir.path().join("prepared-anchor");
        let prepared_reference_path = tempdir.path().join("prepared-pf.conf");
        let system_anchor_path = tempdir.path().join("etc/pf.anchors/com.prvious.pv");
        let system_pf_conf_path = tempdir.path().join("etc/pf.conf");
        let mut commands = Vec::new();

        state::fs::write_sensitive_file(
            &prepared_anchor_path,
            &PfRedirectConfig::new(48080, 48443).render_anchor(),
        )?;
        state::fs::write_sensitive_file(&prepared_reference_path, &PfConfReference.render())?;

        install_pf_redirects_with_runner(
            &prepared_anchor_path,
            &prepared_reference_path,
            &system_anchor_path,
            &system_pf_conf_path,
            &mut |program, args| {
                commands.push(format!("{program} {}", args.join(" ")));

                Ok(())
            },
        )?;

        let candidate =
            read_platform_file(&prepared_reference_path.with_file_name("pf.conf.candidate"))?;
        let validate_candidate_index = commands
            .iter()
            .position(|command| {
                command.contains("/sbin/pfctl -nf") && command.contains("pf.conf.candidate")
            })
            .ok_or_else(|| anyhow::anyhow!("candidate validation command was not recorded"))?;
        let install_anchor_index = commands
            .iter()
            .position(|command| {
                command.contains("/usr/bin/install")
                    && command.contains(prepared_anchor_path.as_str())
                    && command.contains(system_anchor_path.as_str())
            })
            .ok_or_else(|| anyhow::anyhow!("anchor install command was not recorded"))?;

        assert!(candidate.contains("load anchor \"com.prvious.pv\""));
        assert!(install_anchor_index < validate_candidate_index);

        Ok(())
    }

    #[test]
    fn install_pf_redirects_inserts_rdr_anchor_before_filter_rules() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let prepared_anchor_path = tempdir.path().join("prepared-anchor");
        let prepared_reference_path = tempdir.path().join("prepared-pf.conf");
        let system_anchor_path = tempdir.path().join("etc/pf.anchors/com.prvious.pv");
        let system_pf_conf_path = tempdir.path().join("etc/pf.conf");

        state::fs::write_sensitive_file(
            &prepared_anchor_path,
            &PfRedirectConfig::new(48080, 48443).render_anchor(),
        )?;
        state::fs::write_sensitive_file(&prepared_reference_path, &PfConfReference.render())?;
        state::fs::write_sensitive_file(
            &system_pf_conf_path,
            "scrub-anchor \"com.apple/*\" all fragment reassemble\nnat-anchor \"com.apple/*\" all\nrdr-anchor \"com.apple/*\" all\ndummynet-anchor \"com.apple/*\" all\nanchor \"com.apple/*\" all\nload anchor \"com.apple\" from \"/etc/pf.anchors/com.apple\"\n",
        )?;

        install_pf_redirects_with_runner(
            &prepared_anchor_path,
            &prepared_reference_path,
            &system_anchor_path,
            &system_pf_conf_path,
            &mut |_program, _args| Ok(()),
        )?;

        let candidate =
            read_platform_file(&prepared_reference_path.with_file_name("pf.conf.candidate"))?;
        let rdr_index = candidate
            .find("rdr-anchor \"com.prvious.pv\"")
            .ok_or_else(|| anyhow::anyhow!("candidate did not contain PV rdr-anchor"))?;
        let filter_index = candidate
            .find("\nanchor \"com.apple/*\" all")
            .map(|index| index + 1)
            .ok_or_else(|| anyhow::anyhow!("candidate did not preserve filter anchor"))?;
        let dummynet_index = candidate
            .find("\ndummynet-anchor \"com.apple/*\" all")
            .map(|index| index + 1)
            .ok_or_else(|| anyhow::anyhow!("candidate did not preserve dummynet anchor"))?;
        let load_index = candidate
            .find("load anchor \"com.prvious.pv\"")
            .ok_or_else(|| anyhow::anyhow!("candidate did not contain PV load anchor"))?;

        assert!(rdr_index < dummynet_index);
        assert!(dummynet_index < filter_index);
        assert!(filter_index < load_index);

        Ok(())
    }

    #[test]
    fn install_pf_redirects_removes_new_anchor_when_candidate_validation_fails()
    -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let prepared_anchor_path = tempdir.path().join("prepared-anchor");
        let prepared_reference_path = tempdir.path().join("prepared-pf.conf");
        let system_anchor_path = tempdir.path().join("etc/pf.anchors/com.prvious.pv");
        let system_pf_conf_path = tempdir.path().join("etc/pf.conf");
        let mut commands = Vec::new();

        state::fs::write_sensitive_file(
            &prepared_anchor_path,
            &PfRedirectConfig::new(48080, 48443).render_anchor(),
        )?;
        state::fs::write_sensitive_file(&prepared_reference_path, &PfConfReference.render())?;

        let result = install_pf_redirects_with_runner(
            &prepared_anchor_path,
            &prepared_reference_path,
            &system_anchor_path,
            &system_pf_conf_path,
            &mut |program, args| {
                let command = format!("{program} {}", args.join(" "));
                commands.push(command.clone());

                if command.contains("/sbin/pfctl -nf") && command.contains("pf.conf.candidate") {
                    return Err(crate::PlatformError::SystemIntegration(
                        "validate pf.conf candidate failed".to_string(),
                    ));
                }

                Ok(())
            },
        );

        assert!(matches!(
            result,
            Err(crate::PlatformError::SystemIntegration(message))
                if message == "validate pf.conf candidate failed"
        ));
        assert!(commands.iter().any(|command| {
            command.contains("/usr/bin/install")
                && command.contains(prepared_anchor_path.as_str())
                && command.contains(system_anchor_path.as_str())
        }));
        assert!(
            commands
                .iter()
                .any(|command| command == &format!("/bin/rm -f {system_anchor_path}"))
        );

        Ok(())
    }

    #[test]
    fn install_pf_redirects_rolls_back_an_attempted_anchor_install() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let prepared_anchor_path = tempdir.path().join("prepared-anchor");
        let prepared_reference_path = tempdir.path().join("prepared-pf.conf");
        let system_anchor_path = tempdir.path().join("etc/pf.anchors/com.prvious.pv");
        let system_pf_conf_path = tempdir.path().join("etc/pf.conf");
        let mut commands = Vec::new();

        state::fs::write_sensitive_file(
            &prepared_anchor_path,
            &PfRedirectConfig::new(48080, 48443).render_anchor(),
        )?;
        state::fs::write_sensitive_file(&prepared_reference_path, &PfConfReference.render())?;

        let result = install_pf_redirects_with_runner(
            &prepared_anchor_path,
            &prepared_reference_path,
            &system_anchor_path,
            &system_pf_conf_path,
            &mut |program, args| {
                let command = format!("{program} {}", args.join(" "));
                commands.push(command.clone());

                if command.contains("/usr/bin/install")
                    && command.contains(prepared_anchor_path.as_str())
                    && command.contains(system_anchor_path.as_str())
                {
                    return Err(crate::PlatformError::SystemIntegration(
                        "install anchor failed after mutation".to_string(),
                    ));
                }

                Ok(())
            },
        );

        assert!(matches!(
            result,
            Err(crate::PlatformError::SystemIntegration(message))
                if message == "install anchor failed after mutation"
        ));
        assert!(
            commands
                .iter()
                .any(|command| command == &format!("/bin/rm -f {system_anchor_path}"))
        );

        Ok(())
    }

    #[test]
    fn install_pf_redirects_removes_new_anchor_when_later_install_fails() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let prepared_anchor_path = tempdir.path().join("prepared-anchor");
        let prepared_reference_path = tempdir.path().join("prepared-pf.conf");
        let system_anchor_path = tempdir.path().join("etc/pf.anchors/com.prvious.pv");
        let system_pf_conf_path = tempdir.path().join("etc/pf.conf");
        let mut commands = Vec::new();

        state::fs::write_sensitive_file(
            &prepared_anchor_path,
            &PfRedirectConfig::new(48080, 48443).render_anchor(),
        )?;
        state::fs::write_sensitive_file(&prepared_reference_path, &PfConfReference.render())?;

        let result = install_pf_redirects_with_runner(
            &prepared_anchor_path,
            &prepared_reference_path,
            &system_anchor_path,
            &system_pf_conf_path,
            &mut |program, args| {
                let command = format!("{program} {}", args.join(" "));
                commands.push(command.clone());

                if command.contains("/usr/bin/install")
                    && command.contains("pf.conf.candidate")
                    && command.contains(system_pf_conf_path.as_str())
                {
                    return Err(crate::PlatformError::SystemIntegration(
                        "install pf.conf failed".to_string(),
                    ));
                }

                Ok(())
            },
        );

        assert!(matches!(
            result,
            Err(crate::PlatformError::SystemIntegration(message))
                if message == "install pf.conf failed"
        ));
        assert!(commands.iter().any(|command| {
            command.contains("/usr/bin/install")
                && command.contains(prepared_anchor_path.as_str())
                && command.contains(system_anchor_path.as_str())
        }));
        assert!(
            commands
                .iter()
                .any(|command| command == &format!("/bin/rm -f {system_anchor_path}"))
        );

        Ok(())
    }

    #[test]
    fn install_pf_redirects_restores_pf_conf_when_reload_fails() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let prepared_anchor_path = tempdir.path().join("prepared-anchor");
        let prepared_reference_path = tempdir.path().join("prepared-pf.conf");
        let system_anchor_path = tempdir.path().join("etc/pf.anchors/com.prvious.pv");
        let system_pf_conf_path = tempdir.path().join("etc/pf.conf");
        let mut commands = Vec::new();

        state::fs::write_sensitive_file(
            &prepared_anchor_path,
            &PfRedirectConfig::new(48080, 48443).render_anchor(),
        )?;
        state::fs::write_sensitive_file(&prepared_reference_path, &PfConfReference.render())?;
        state::fs::write_sensitive_file(&system_pf_conf_path, "set skip on lo0\n")?;

        let result = install_pf_redirects_with_runner(
            &prepared_anchor_path,
            &prepared_reference_path,
            &system_anchor_path,
            &system_pf_conf_path,
            &mut |program, args| {
                let command = format!("{program} {}", args.join(" "));
                commands.push(command.clone());

                if command == format!("/sbin/pfctl -f {system_pf_conf_path}")
                    && commands
                        .iter()
                        .filter(|recorded| *recorded == &command)
                        .count()
                        == 1
                {
                    return Err(crate::PlatformError::SystemIntegration(
                        "reload pf failed".to_string(),
                    ));
                }

                Ok(())
            },
        );

        assert!(matches!(
            result,
            Err(crate::PlatformError::SystemIntegration(message))
                if message == "reload pf failed"
        ));
        assert!(commands.iter().any(|command| {
            command.contains("/usr/bin/install")
                && command.contains("pf.conf.rollback")
                && command.contains(system_pf_conf_path.as_str())
        }));
        assert!(
            commands
                .iter()
                .any(|command| command == &format!("/bin/rm -f {system_anchor_path}"))
        );
        assert_eq!(
            commands
                .iter()
                .filter(|command| command == &&format!("/sbin/pfctl -f {system_pf_conf_path}"))
                .count(),
            2
        );

        Ok(())
    }

    #[test]
    fn failed_pf_apply_does_not_enable_pf_and_flushes_new_anchor() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let prepared_anchor_path = tempdir.path().join("prepared-anchor");
        let prepared_reference_path = tempdir.path().join("prepared-pf.conf");
        let system_anchor_path = tempdir.path().join("etc/pf.anchors/com.prvious.pv");
        let system_pf_conf_path = tempdir.path().join("etc/pf.conf");
        let commands = RefCell::new(Vec::new());

        state::fs::write_sensitive_file(
            &prepared_anchor_path,
            &PfRedirectConfig::new(48080, 48443).render_anchor(),
        )?;
        state::fs::write_sensitive_file(&prepared_reference_path, &PfConfReference.render())?;

        let result = install_pf_redirects_and_verify_with_runner(
            PfInstallPaths {
                prepared_anchor: &prepared_anchor_path,
                prepared_reference: &prepared_reference_path,
                system_anchor: &system_anchor_path,
                system_pf_conf: &system_pf_conf_path,
            },
            &mut |program, args| {
                commands
                    .borrow_mut()
                    .push(format!("{program} {}", args.join(" ")));
                Ok(())
            },
            &mut || {
                commands.borrow_mut().push("/sbin/pfctl -E".to_string());
                Ok("pf enabled\nToken : 4242\n".to_string())
            },
            false,
            &mut || {
                Err(crate::PlatformError::SystemIntegration(
                    "active verification failed".to_string(),
                ))
            },
        );
        let commands = commands.into_inner();

        assert!(result.is_err());
        assert!(
            commands
                .iter()
                .any(|command| command == "/sbin/pfctl -a com.prvious.pv -F all")
        );
        assert!(!commands.iter().any(|command| command == "/sbin/pfctl -E"));

        Ok(())
    }

    #[test]
    fn temporary_pf_conf_candidate_path_uses_candidate_dir_and_random_suffix() -> anyhow::Result<()>
    {
        let tempdir = tempdir()?;
        let candidate_dir = tempdir.path().join("config/pf");
        let first = temporary_pf_conf_candidate_path(&candidate_dir)?;
        let second = temporary_pf_conf_candidate_path(&candidate_dir)?;
        let process_id = std::process::id().to_string();

        assert_eq!(first.parent(), Some(candidate_dir.as_path()));
        assert_eq!(second.parent(), Some(candidate_dir.as_path()));
        assert_ne!(first, second);
        assert!(first.file_name().is_some_and(|name| {
            name.starts_with("pv-pf-conf-") && name.ends_with("-uninstall")
        }));
        assert!(
            !first
                .file_name()
                .is_some_and(|name| name.contains(&process_id))
        );

        Ok(())
    }

    #[test]
    fn remove_pf_redirects_writes_candidate_in_pv_config_dir() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let candidate_dir = tempdir.path().join("home/.pv/config/pf");
        let system_anchor_path = tempdir.path().join("etc/pf.anchors/com.prvious.pv");
        let system_pf_conf_path = tempdir.path().join("etc/pf.conf");
        let mut commands = Vec::new();
        let mut validated_candidate = None;

        state::fs::write_sensitive_file(
            &system_anchor_path,
            &PfRedirectConfig::new(48080, 48443).render_anchor(),
        )?;
        state::fs::write_sensitive_file(
            &system_pf_conf_path,
            &format!("{}\n{}", "set skip on lo0", PfConfReference.render()),
        )?;

        remove_pf_redirects_with_runner(
            &system_anchor_path,
            &system_pf_conf_path,
            &candidate_dir,
            &mut |program, args| {
                commands.push(format!("{program} {}", args.join(" ")));
                if program == "/sbin/pfctl"
                    && args.first() == Some(&"-nf")
                    && let Some(path) = args.get(1)
                {
                    validated_candidate = Some(read_platform_file(Utf8Path::new(path))?);
                }

                Ok(())
            },
        )?;

        let candidate_command = commands
            .iter()
            .find(|command| command.contains("/sbin/pfctl -nf"))
            .ok_or_else(|| anyhow::anyhow!("candidate validation command was not recorded"))?;
        let candidate_path = candidate_command
            .split_whitespace()
            .last()
            .ok_or_else(|| anyhow::anyhow!("candidate validation command had no path"))?;
        let candidate_path = Utf8Path::new(candidate_path);

        assert!(candidate_path.starts_with(&candidate_dir));
        assert_eq!(validated_candidate.as_deref(), Some("set skip on lo0\n"));
        assert!(!state::fs::path_entry_exists(candidate_path)?);

        Ok(())
    }
}
