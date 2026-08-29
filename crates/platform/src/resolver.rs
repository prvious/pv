use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::PlatformError;
use crate::command::run_system_command;

pub const SYSTEM_RESOLVER_TEST_PATH: &str = "/etc/resolver/test";
const PV_MARKER: &str = "# Managed by PV";
const PREPARED_MARKER: &str = "# Source: PV prepared resolver config for /etc/resolver/test";
const LOOPBACK_NAMESERVER: &str = "nameserver 127.0.0.1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResolverConfig {
    pub port: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ResolverFileState {
    Missing {
        path: Utf8PathBuf,
    },
    Current {
        path: Utf8PathBuf,
        port: u16,
    },
    Stale {
        path: Utf8PathBuf,
        expected_port: Option<u16>,
        actual_port: Option<u16>,
    },
    Conflict {
        path: Utf8PathBuf,
    },
    Unreadable {
        path: Utf8PathBuf,
        message: String,
    },
}

impl ResolverConfig {
    pub const fn new(port: u16) -> Self {
        Self { port }
    }

    pub fn render(&self) -> String {
        format!(
            "{PV_MARKER}\n{PREPARED_MARKER}\n{LOOPBACK_NAMESERVER}\nport {}\n",
            self.port
        )
    }

    pub fn parse(content: &str) -> Option<Self> {
        let mut port = None;
        let mut has_nameserver = false;
        let mut active_line_count = 0;

        for line in content.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            active_line_count += 1;

            if line.starts_with("nameserver ") {
                if line != LOOPBACK_NAMESERVER || has_nameserver {
                    return None;
                }
                has_nameserver = true;
                continue;
            }

            let value = line.strip_prefix("port ")?;

            if port.replace(value.parse::<u16>().ok()?).is_some() {
                return None;
            }
        }

        if active_line_count == 2 && has_nameserver {
            port.map(Self::new)
        } else {
            None
        }
    }
}

pub fn inspect_resolver_file(
    path: &Utf8Path,
    expected: Option<&ResolverConfig>,
) -> ResolverFileState {
    let content = match state::fs::read_to_string(path) {
        Ok(content) => content,
        Err(state::StateError::Filesystem { source, .. })
            if source.kind() == io::ErrorKind::NotFound =>
        {
            return ResolverFileState::Missing {
                path: path.to_path_buf(),
            };
        }
        Err(error) => {
            return ResolverFileState::Unreadable {
                path: path.to_path_buf(),
                message: error.to_string(),
            };
        }
    };

    if !content.lines().any(|line| line.trim() == PV_MARKER) {
        return ResolverFileState::Conflict {
            path: path.to_path_buf(),
        };
    }

    let actual = ResolverConfig::parse(&content);

    match (expected, actual) {
        (Some(expected), Some(actual)) if expected == &actual => ResolverFileState::Current {
            path: path.to_path_buf(),
            port: actual.port,
        },
        (Some(expected), actual) => ResolverFileState::Stale {
            path: path.to_path_buf(),
            expected_port: Some(expected.port),
            actual_port: actual.map(|config| config.port),
        },
        (None, Some(actual)) => ResolverFileState::Current {
            path: path.to_path_buf(),
            port: actual.port,
        },
        (None, None) => ResolverFileState::Stale {
            path: path.to_path_buf(),
            expected_port: None,
            actual_port: None,
        },
    }
}

pub fn install_resolver_config(
    prepared_path: &Utf8Path,
    system_path: &Utf8Path,
) -> Result<(), PlatformError> {
    require_fixed_system_path(system_path)?;
    let prepared = state::fs::read_to_string(prepared_path)
        .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?;
    let config = ResolverConfig::parse(&prepared).ok_or_else(|| {
        PlatformError::SystemIntegration(format!(
            "prepared resolver config is not valid PV configuration: {prepared_path}"
        ))
    })?;

    crate::PrivilegedHelperClient.apply_dns(&config)
}

pub fn remove_resolver_config(system_path: &Utf8Path) -> Result<(), PlatformError> {
    require_fixed_system_path(system_path)?;
    crate::PrivilegedHelperClient.remove_dns()
}

fn require_fixed_system_path(system_path: &Utf8Path) -> Result<(), PlatformError> {
    if system_path == Utf8Path::new(SYSTEM_RESOLVER_TEST_PATH) {
        return Ok(());
    }

    Err(PlatformError::SystemIntegration(format!(
        "resolver mutation requires fixed system path {SYSTEM_RESOLVER_TEST_PATH}"
    )))
}

#[cfg(target_os = "macos")]
pub(crate) fn apply_resolver_config_privileged(
    config: &ResolverConfig,
) -> Result<(), PlatformError> {
    let system_path = Utf8Path::new(SYSTEM_RESOLVER_TEST_PATH);
    match inspect_resolver_file(system_path, Some(config)) {
        ResolverFileState::Conflict { path } => {
            return Err(PlatformError::SystemIntegration(format!(
                "system resolver config is not PV-owned: {path}"
            )));
        }
        ResolverFileState::Unreadable { path, message } => {
            return Err(PlatformError::SystemIntegration(format!(
                "system resolver config could not be inspected: {path}: {message}"
            )));
        }
        ResolverFileState::Missing { .. }
        | ResolverFileState::Current { .. }
        | ResolverFileState::Stale { .. } => {}
    }
    crate::helper::validate_root_owned_file_if_present(system_path)?;

    let prepared_path = Utf8Path::new("/Library/Application Support/PV/resolver.test");
    crate::helper::write_root_work_file(prepared_path, &config.render(), "0755")?;
    run_system_command("/bin/mkdir", &["-p", "/etc/resolver"])?;
    run_system_command(
        "/usr/bin/install",
        &[
            "-o",
            "root",
            "-g",
            "wheel",
            "-m",
            "0644",
            prepared_path.as_str(),
            SYSTEM_RESOLVER_TEST_PATH,
        ],
    )?;

    match inspect_resolver_file(system_path, Some(config)) {
        ResolverFileState::Current { .. } => Ok(()),
        state => Err(PlatformError::SystemIntegration(format!(
            "resolver config did not match after helper apply: {state:?}"
        ))),
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn remove_resolver_config_privileged() -> Result<(), PlatformError> {
    let system_path = Utf8Path::new(SYSTEM_RESOLVER_TEST_PATH);
    match inspect_resolver_file(system_path, None) {
        ResolverFileState::Missing { .. } => return Ok(()),
        ResolverFileState::Conflict { path } => {
            return Err(PlatformError::SystemIntegration(format!(
                "system resolver config is not PV-owned: {path}"
            )));
        }
        ResolverFileState::Unreadable { path, message } => {
            return Err(PlatformError::SystemIntegration(format!(
                "system resolver config could not be inspected: {path}: {message}"
            )));
        }
        ResolverFileState::Current { .. } | ResolverFileState::Stale { .. } => {}
    }
    crate::helper::validate_root_owned_file_if_present(system_path)?;
    run_system_command("/bin/rm", &["-f", SYSTEM_RESOLVER_TEST_PATH])
}
