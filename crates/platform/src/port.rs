use std::net::{Ipv4Addr, SocketAddrV4};

use socket2::{Domain, Protocol, Socket, Type};

/// Whether a TCP port can be used by PV's IPv4 loopback runtimes.
///
/// Both `127.0.0.1` and `0.0.0.0` must bind with the runtimes' `SO_REUSEADDR`
/// option. Closing connections therefore do not churn persisted ports, while the
/// wildcard probe catches listeners (such as Docker Desktop publications) that
/// a same-user loopback bind could otherwise shadow. `SO_REUSEPORT` stays off.
/// The probe only binds; it never listens.
/// Any socket, option, or bind error counts as unavailable: PV never takes a port
/// whose availability it cannot verify. This is a probe, not a reservation.
pub fn loopback_tcp_port_available(port: u16) -> bool {
    [Ipv4Addr::LOCALHOST, Ipv4Addr::UNSPECIFIED]
        .into_iter()
        .all(|address| {
            let Ok(socket) = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP)) else {
                return false;
            };
            socket.set_reuse_address(true).is_ok()
                && socket
                    .bind(&SocketAddrV4::new(address, port).into())
                    .is_ok()
        })
}
