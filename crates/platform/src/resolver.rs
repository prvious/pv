use std::io;

use camino::{Utf8Path, Utf8PathBuf};

pub const SYSTEM_RESOLVER_TEST_PATH: &str = "/etc/resolver/test";
const PV_MARKER: &str = "# Managed by PV";
const PREPARED_MARKER: &str = "# Source: PV prepared resolver config for /etc/resolver/test";
const LOOPBACK_NAMESERVER: &str = "nameserver 127.0.0.1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolverConfig {
    pub port: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
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
        let mut nameserver_count = 0;
        let mut has_loopback_nameserver = false;

        for line in content.lines().map(str::trim) {
            if line.starts_with("nameserver ") {
                nameserver_count += 1;
                if line == LOOPBACK_NAMESERVER {
                    has_loopback_nameserver = true;
                }
                continue;
            }

            let Some(value) = line.strip_prefix("port ") else {
                continue;
            };

            port = value.parse::<u16>().ok();
        }

        if nameserver_count == 1 && has_loopback_nameserver {
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
