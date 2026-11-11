#[cfg(any(not(target_os = "linux"), test))]
use std::io;
use std::net::Ipv4Addr;

#[cfg(any(not(target_os = "linux"), test))]
use camino::Utf8Path;
use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

use crate::PlatformError;
#[cfg(target_os = "macos")]
use crate::command::run_system_command;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub(crate) use linux::{
    DNS_MUTATION_TIMEOUT, apply_resolver_config_privileged, remove_resolver_config_privileged,
};
#[cfg(target_os = "linux")]
pub use linux::{start_resolver_link, stop_resolver_link};

#[cfg(target_os = "linux")]
pub const SYSTEM_RESOLVER_TEST_PATH: &str = "/etc/systemd/system/pv-dns.service";
#[cfg(not(target_os = "linux"))]
pub const SYSTEM_RESOLVER_TEST_PATH: &str = "/etc/resolver/test";
const PV_MARKER: &str = "# Managed by PV";
/// Fixed DNS listener address; DNS A answers remain `127.0.0.1` on every platform.
#[cfg(target_os = "linux")]
pub const DNS_LISTEN_ADDRESS: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 2);
#[cfg(not(target_os = "linux"))]
pub const DNS_LISTEN_ADDRESS: Ipv4Addr = Ipv4Addr::LOCALHOST;

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
            "{PV_MARKER}\nnameserver {}\nport {}\n",
            Ipv4Addr::LOCALHOST,
            self.port
        )
    }

    pub fn parse(content: &str) -> Option<Self> {
        let mut port = None;
        let mut has_nameserver = false;
        let mut active_line_count = 0;
        let nameserver = format!("nameserver {}", Ipv4Addr::LOCALHOST);

        for line in content.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            active_line_count += 1;

            if line.starts_with("nameserver ") {
                if line != nameserver || has_nameserver {
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

/// The system's `.test` resolver configuration: `/etc/resolver/test` on macOS; `pv-dns.service` and
/// the `pv0` link on Linux. Reading it needs no privileges.
pub fn inspect_system_resolver(expected: Option<&ResolverConfig>) -> ResolverFileState {
    #[cfg(target_os = "linux")]
    return linux::inspect(expected);
    #[cfg(not(target_os = "linux"))]
    inspect_resolver_file(Utf8Path::new(SYSTEM_RESOLVER_TEST_PATH), expected)
}

/// Points the system's `.test` lookups at PV's resolver on `port`, through the helper.
pub fn apply_system_resolver(port: u16) -> Result<(), PlatformError> {
    crate::PrivilegedHelperClient.apply_dns(&ResolverConfig::new(port))
}

/// Removes PV's `.test` resolver configuration, through the helper.
pub fn remove_system_resolver() -> Result<(), PlatformError> {
    crate::PrivilegedHelperClient.remove_dns()
}

#[cfg(any(not(target_os = "linux"), test))]
fn inspect_resolver_file(path: &Utf8Path, expected: Option<&ResolverConfig>) -> ResolverFileState {
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

#[cfg(test)]
mod tests {
    use camino_tempfile::tempdir;

    use super::{ResolverConfig, inspect_resolver_file};

    #[test]
    fn resolver_file_inspection_reports_missing_current_stale_conflict_and_unreadable()
    -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let current_path = tempdir.path().join("current");
        let stale_path = tempdir.path().join("stale");
        let wrong_nameserver_path = tempdir.path().join("wrong-nameserver");
        let mixed_nameservers_path = tempdir.path().join("mixed-nameservers");
        let conflict_path = tempdir.path().join("conflict");
        let unreadable_path = tempdir.path().join("directory");
        let expected = ResolverConfig::new(35353);

        state::fs::write_sensitive_file(&current_path, &expected.render())?;
        state::fs::write_sensitive_file(&stale_path, &ResolverConfig::new(45000).render())?;
        state::fs::write_sensitive_file(
            &wrong_nameserver_path,
            "# Managed by PV\nnameserver 10.0.0.1\nport 35353\n",
        )?;
        state::fs::write_sensitive_file(
            &mixed_nameservers_path,
            "# Managed by PV\nnameserver 127.0.0.1\nnameserver 10.0.0.1\nport 35353\n",
        )?;
        state::fs::write_sensitive_file(&conflict_path, "nameserver 127.0.0.1\nport 35353\n")?;
        state::fs::write_sensitive_file(&unreadable_path.join("child"), "child\n")?;

        let states = [
            tempdir.path().join("missing"),
            current_path,
            stale_path,
            wrong_nameserver_path,
            mixed_nameservers_path,
            conflict_path,
            unreadable_path,
        ]
        .iter()
        .map(|path| {
            format!("{:?}", inspect_resolver_file(path, Some(&expected)))
                .replace(tempdir.path().as_str(), "<tempdir>")
        })
        .collect::<Vec<_>>();

        insta::assert_debug_snapshot!(states);

        Ok(())
    }
}
