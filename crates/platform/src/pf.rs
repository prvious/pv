use std::io;
use std::path::PathBuf;

use camino::{Utf8Path, Utf8PathBuf};

use crate::PlatformError;
use crate::command::run_system_command;

pub const SYSTEM_PF_ANCHOR_PATH: &str = "/etc/pf.anchors/com.prvious.pv";
pub const SYSTEM_PF_CONF_PATH: &str = "/etc/pf.conf";
const PV_MARKER: &str = "# Managed by PV";
const PF_ANCHOR_SOURCE_MARKER: &str =
    "# Source: PV prepared pf anchor for /etc/pf.anchors/com.prvious.pv";
const PF_CONF_SOURCE_MARKER: &str = "# Source: PV prepared pf.conf reference for /etc/pf.conf";
const PF_ANCHOR_DIRECTIVE: &str = "anchor \"com.prvious.pv\"";
const PF_LOAD_ANCHOR_DIRECTIVE: &str =
    "load anchor \"com.prvious.pv\" from \"/etc/pf.anchors/com.prvious.pv\"";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PfRedirectConfig {
    pub http_port: u16,
    pub https_port: u16,
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct PfConfReference;

#[derive(Clone, Debug, Eq, PartialEq)]
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
}

impl PfConfReference {
    pub fn render(self) -> String {
        format!(
            "{PV_MARKER}\n{PF_CONF_SOURCE_MARKER}\n{PF_ANCHOR_DIRECTIVE}\n{PF_LOAD_ANCHOR_DIRECTIVE}\n"
        )
    }

    pub fn parse_block(content: &str) -> Option<Self> {
        let mut has_anchor = false;
        let mut has_load = false;
        let mut active_line_count = 0;

        for line in content.lines().filter_map(active_pf_line) {
            active_line_count += 1;

            if line == PF_ANCHOR_DIRECTIVE {
                if has_anchor {
                    return None;
                }
                has_anchor = true;
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

        if active_line_count == 2 && has_anchor && has_load {
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

pub fn install_pf_redirects(
    prepared_anchor_path: &Utf8Path,
    prepared_reference_path: &Utf8Path,
    system_anchor_path: &Utf8Path,
    system_pf_conf_path: &Utf8Path,
) -> Result<(), PlatformError> {
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

    state::fs::write_sensitive_file(&candidate_path, &candidate)
        .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?;
    install_pf_anchor(prepared_anchor_path, system_anchor_path)?;
    run_system_command(
        "/usr/bin/sudo",
        &["/sbin/pfctl", "-nf", candidate_path.as_str()],
    )?;
    run_system_command(
        "/usr/bin/sudo",
        &[
            "/usr/bin/install",
            "-m",
            "0644",
            candidate_path.as_str(),
            system_pf_conf_path.as_str(),
        ],
    )?;
    reload_pf(system_pf_conf_path)?;
    run_system_command("/usr/bin/sudo", &["/sbin/pfctl", "-E"])
}

pub fn remove_pf_redirects(
    system_anchor_path: &Utf8Path,
    system_pf_conf_path: &Utf8Path,
) -> Result<(), PlatformError> {
    let system_reference_state = inspect_pf_conf_reference(system_pf_conf_path, None);

    match system_reference_state {
        PfFileState::Missing { .. } => {}
        PfFileState::Current { .. } | PfFileState::Stale { .. } => {
            let current = read_platform_file(system_pf_conf_path)?;
            let candidate = remove_pf_reference_lines(&current);
            let candidate_path = temporary_pf_conf_candidate_path()?;

            write_temporary_file(&candidate_path, &candidate)?;
            run_system_command(
                "/usr/bin/sudo",
                &["/sbin/pfctl", "-nf", candidate_path.as_str()],
            )?;
            run_system_command(
                "/usr/bin/sudo",
                &[
                    "/usr/bin/install",
                    "-m",
                    "0644",
                    candidate_path.as_str(),
                    system_pf_conf_path.as_str(),
                ],
            )?;
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

    run_system_command(
        "/usr/bin/sudo",
        &["/bin/rm", "-f", system_anchor_path.as_str()],
    )?;
    reload_pf(system_pf_conf_path)
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
    line.starts_with("anchor \"com.prvious.pv\"")
        || line.starts_with("load anchor \"com.prvious.pv\"")
}

fn parse_embedded_pf_conf_reference(content: &str) -> Option<PfConfReference> {
    let mut has_anchor = false;
    let mut has_load = false;

    for line in content.lines().filter_map(active_pf_line) {
        if line == PF_ANCHOR_DIRECTIVE {
            if has_anchor {
                return None;
            }
            has_anchor = true;
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

    if has_anchor && has_load {
        Some(PfConfReference)
    } else {
        None
    }
}

fn install_pf_anchor(
    prepared_anchor_path: &Utf8Path,
    system_anchor_path: &Utf8Path,
) -> Result<(), PlatformError> {
    let parent = system_anchor_path.parent().ok_or_else(|| {
        PlatformError::SystemIntegration(format!(
            "pf anchor path has no parent directory: {system_anchor_path}"
        ))
    })?;

    run_system_command(
        "/usr/bin/sudo",
        &["/sbin/pfctl", "-nf", prepared_anchor_path.as_str()],
    )?;
    run_system_command("/usr/bin/sudo", &["/bin/mkdir", "-p", parent.as_str()])?;
    run_system_command(
        "/usr/bin/sudo",
        &[
            "/usr/bin/install",
            "-m",
            "0644",
            prepared_anchor_path.as_str(),
            system_anchor_path.as_str(),
        ],
    )
}

fn reload_pf(system_pf_conf_path: &Utf8Path) -> Result<(), PlatformError> {
    run_system_command(
        "/usr/bin/sudo",
        &["/sbin/pfctl", "-f", system_pf_conf_path.as_str()],
    )
}

fn append_pf_reference(content: &str, reference: &str) -> String {
    if content.trim().is_empty() {
        return reference.to_string();
    }

    let mut candidate = content.to_string();
    if !candidate.ends_with('\n') {
        candidate.push('\n');
    }
    candidate.push_str(reference);
    candidate
}

fn remove_pf_reference_lines(content: &str) -> String {
    let mut candidate = String::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if matches!(
            trimmed,
            PV_MARKER | PF_CONF_SOURCE_MARKER | PF_ANCHOR_DIRECTIVE | PF_LOAD_ANCHOR_DIRECTIVE
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

fn temporary_pf_conf_candidate_path() -> Result<Utf8PathBuf, PlatformError> {
    let temp_dir = process_temp_dir();
    let temp_dir = Utf8PathBuf::from_path_buf(temp_dir).map_err(|path| {
        PlatformError::SystemIntegration(format!(
            "temporary directory path is not valid UTF-8: {path:?}"
        ))
    })?;

    Ok(temp_dir.join(format!("pv-pf-conf-{}-uninstall", std::process::id())))
}

#[expect(
    clippy::disallowed_methods,
    reason = "platform pf helper owns process temporary directory lookup"
)]
fn process_temp_dir() -> PathBuf {
    std::env::temp_dir()
}

#[expect(
    clippy::disallowed_methods,
    reason = "platform pf helper owns temporary pf.conf candidate writes"
)]
fn write_temporary_file(path: &Utf8Path, content: &str) -> Result<(), PlatformError> {
    std::fs::write(path, content).map_err(|source| {
        PlatformError::SystemIntegration(format!(
            "could not write pf.conf candidate {path}: {source}"
        ))
    })
}
