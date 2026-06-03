use std::collections::BTreeSet;
use std::io;
use std::net::IpAddr;

use camino::{Utf8Path, Utf8PathBuf};
use thiserror::Error;

pub const SYSTEM_RESOLVER_TEST_PATH: &str = "/etc/resolver/test";
pub const SYSTEM_PF_ANCHOR_PATH: &str = "/etc/pf.anchors/com.prvious.pv";
pub const SYSTEM_PF_CONF_PATH: &str = "/etc/pf.conf";
const PV_MARKER: &str = "# Managed by PV";
const PREPARED_MARKER: &str = "# Source: PV prepared resolver config for /etc/resolver/test";
const PF_ANCHOR_SOURCE_MARKER: &str =
    "# Source: PV prepared pf anchor for /etc/pf.anchors/com.prvious.pv";
const PF_CONF_SOURCE_MARKER: &str = "# Source: PV prepared pf.conf reference for /etc/pf.conf";
const PF_ANCHOR_DIRECTIVE: &str = "anchor \"com.prvious.pv\"";
const PF_LOAD_ANCHOR_DIRECTIVE: &str =
    "load anchor \"com.prvious.pv\" from \"/etc/pf.anchors/com.prvious.pv\"";
const LOOPBACK_NAMESERVER: &str = "nameserver 127.0.0.1";

#[expect(
    clippy::disallowed_types,
    reason = "macOS socket inspection owns read-only netstat invocation"
)]
type StdCommand = std::process::Command;

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

#[derive(Debug, Error)]
pub enum MacosError {
    #[error("could not inspect socket table: {0}")]
    SocketTable(#[from] netstat::Error),
    #[error("could not run netstat for socket inspection: {0}")]
    SocketTableCommand(#[source] io::Error),
    #[error("netstat socket inspection exited with {status}")]
    SocketTableCommandStatus { status: String },
    #[error("could not decode netstat socket table: {0}")]
    SocketTableCommandUtf8(#[from] std::string::FromUtf8Error),
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
        let has_anchor = content
            .lines()
            .map(str::trim)
            .any(|line| line == PF_ANCHOR_DIRECTIVE);
        let has_load = content
            .lines()
            .map(str::trim)
            .any(|line| line == PF_LOAD_ANCHOR_DIRECTIVE);

        if has_anchor && has_load {
            Some(Self)
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

    let actual = PfConfReference::parse_block(&content);
    classify_pv_file_state(path, expected, actual)
}

pub fn loopback_tcp_listener_ports() -> Result<BTreeSet<u16>, MacosError> {
    let mut ports = loopback_tcp_listener_ports_from_socket_table()?;
    ports.extend(parse_netstat_tcp_listener_ports(
        &netstat_tcp_socket_table()?
    ));

    Ok(ports)
}

pub fn loopback_tcp_port_has_listener(port: u16) -> Result<bool, MacosError> {
    Ok(loopback_tcp_listener_ports()?.contains(&port))
}

fn loopback_tcp_listener_ports_from_socket_table() -> Result<BTreeSet<u16>, MacosError> {
    let sockets = netstat::get_sockets_info(
        netstat::AddressFamilyFlags::IPV4 | netstat::AddressFamilyFlags::IPV6,
        netstat::ProtocolFlags::TCP,
    )?;
    let mut ports = BTreeSet::new();

    for socket in sockets {
        let netstat::ProtocolSocketInfo::Tcp(tcp) = socket.protocol_socket_info else {
            continue;
        };

        if tcp.state == netstat::TcpState::Listen
            && tcp_listener_address_occupies_loopback(tcp.local_addr)
        {
            ports.insert(tcp.local_port);
        }
    }

    Ok(ports)
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
    line == PF_ANCHOR_DIRECTIVE || line == PF_LOAD_ANCHOR_DIRECTIVE
}

fn tcp_listener_address_occupies_loopback(address: IpAddr) -> bool {
    address.is_loopback() || address.is_unspecified()
}

fn netstat_tcp_socket_table() -> Result<String, MacosError> {
    let output = StdCommand::new("/usr/sbin/netstat")
        .args(["-anv", "-p", "tcp"])
        .output()
        .map_err(MacosError::SocketTableCommand)?;

    if !output.status.success() {
        return Err(MacosError::SocketTableCommandStatus {
            status: output.status.to_string(),
        });
    }

    Ok(String::from_utf8(output.stdout)?)
}

fn parse_netstat_tcp_listener_ports(output: &str) -> BTreeSet<u16> {
    let mut ports = BTreeSet::new();

    for line in output.lines() {
        let columns = line.split_whitespace().collect::<Vec<_>>();
        let [
            protocol,
            _recv_queue,
            _send_queue,
            local_address,
            _foreign_address,
            state,
            ..,
        ] = columns.as_slice()
        else {
            continue;
        };

        if !protocol.starts_with("tcp") || *state != "LISTEN" {
            continue;
        }

        if let Some(port) = loopback_port_from_netstat_local_address(local_address) {
            ports.insert(port);
        }
    }

    ports
}

fn loopback_port_from_netstat_local_address(local_address: &str) -> Option<u16> {
    let (address, port) = local_address.rsplit_once('.')?;
    let port = port.parse::<u16>().ok()?;

    if address == "*" {
        return Some(port);
    }

    let address = address.parse::<IpAddr>().ok()?;

    if tcp_listener_address_occupies_loopback(address) {
        Some(port)
    } else {
        None
    }
}
