use std::collections::BTreeSet;
#[cfg(any(target_os = "macos", test))]
use std::net::IpAddr;

#[cfg(target_os = "macos")]
use netstat::{AddressFamilyFlags, ProtocolFlags, ProtocolSocketInfo, TcpState, get_sockets_info};

#[cfg(not(target_os = "macos"))]
use crate::PlatformCapability;
#[cfg(not(target_os = "macos"))]
use crate::capability::unsupported;
use crate::error::PlatformError;

#[cfg(target_os = "macos")]
#[expect(
    clippy::disallowed_types,
    reason = "macOS socket inspection owns read-only netstat invocation"
)]
type StdCommand = std::process::Command;

pub fn loopback_tcp_listener_ports() -> Result<BTreeSet<u16>, PlatformError> {
    #[cfg(target_os = "macos")]
    {
        let mut ports = loopback_tcp_listener_ports_from_socket_table()?;
        ports.extend(parse_netstat_tcp_listener_ports(
            &netstat_tcp_socket_table()?
        ));

        Ok(ports)
    }

    #[cfg(not(target_os = "macos"))]
    {
        Err(unsupported(PlatformCapability::ListenerInspection)?)
    }
}

pub fn loopback_tcp_port_has_listener(port: u16) -> Result<bool, PlatformError> {
    Ok(loopback_tcp_listener_ports()?.contains(&port))
}

#[cfg(target_os = "macos")]
fn loopback_tcp_listener_ports_from_socket_table() -> Result<BTreeSet<u16>, PlatformError> {
    let sockets = get_sockets_info(
        AddressFamilyFlags::IPV4 | AddressFamilyFlags::IPV6,
        ProtocolFlags::TCP,
    )?;
    let mut ports = BTreeSet::new();

    for socket in sockets {
        let ProtocolSocketInfo::Tcp(tcp) = socket.protocol_socket_info else {
            continue;
        };

        if tcp.state == TcpState::Listen && tcp_listener_address_occupies_loopback(tcp.local_addr) {
            ports.insert(tcp.local_port);
        }
    }

    Ok(ports)
}

#[cfg(target_os = "macos")]
fn netstat_tcp_socket_table() -> Result<String, PlatformError> {
    let output = StdCommand::new("/usr/sbin/netstat")
        .args(["-anv", "-p", "tcp"])
        .output()
        .map_err(PlatformError::SocketTableCommand)?;

    if !output.status.success() {
        return Err(PlatformError::SocketTableCommandStatus {
            status: output.status.to_string(),
        });
    }

    Ok(String::from_utf8(output.stdout)?)
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn parse_netstat_tcp_listener_ports(output: &str) -> BTreeSet<u16> {
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

#[cfg(any(target_os = "macos", test))]
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

#[cfg(any(target_os = "macos", test))]
fn tcp_listener_address_occupies_loopback(address: IpAddr) -> bool {
    address.is_loopback() || address.is_unspecified()
}
