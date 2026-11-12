//! Where the daemon gets the Gateway's two listening sockets: launchd opens `127.0.0.1:80` and
//! `:443` for PV's LaunchAgent on macOS, and on Linux the daemon binds them itself once the
//! kernel's unprivileged-port setting allows it.

use std::io;
#[cfg(unix)]
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
#[cfg(unix)]
use std::os::fd::OwnedFd;

/// The LaunchAgent's socket names for the Gateway's HTTP and HTTPS ports.
pub const GATEWAY_HTTP_SOCKET: &str = "GatewayHttp";
pub const GATEWAY_HTTPS_SOCKET: &str = "GatewayHttps";
pub const GATEWAY_HTTP_PORT: u16 = 80;
pub const GATEWAY_HTTPS_PORT: u16 = 443;

/// Where the daemon gets the Gateway's two listening sockets, and which ports it probes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GatewayListenerSource {
    /// macOS production: the LaunchAgent's `GatewayHttp`/`GatewayHttps` sockets.
    Launchd,
    /// Linux production uses ports 80 and 443; tests on any OS use two `pv_fake::port()`s.
    Bind { http_port: u16, https_port: u16 },
}

impl GatewayListenerSource {
    /// `Launchd` on macOS, `Bind { 80, 443 }` everywhere else.
    pub const fn production() -> Self {
        if cfg!(target_os = "macos") {
            Self::Launchd
        } else {
            Self::Bind {
                http_port: GATEWAY_HTTP_PORT,
                https_port: GATEWAY_HTTPS_PORT,
            }
        }
    }

    /// The ports readiness probes use: 80/443 for `Launchd`, the given ports for `Bind`.
    pub const fn ports(self) -> (u16, u16) {
        match self {
            Self::Launchd => (GATEWAY_HTTP_PORT, GATEWAY_HTTPS_PORT),
            Self::Bind {
                http_port,
                https_port,
            } => (http_port, https_port),
        }
    }

    /// Obtains the listeners, by launchd activation or by binding, and checks that each is bound
    /// to exactly `127.0.0.1` and its port. Launchd activation works once per process.
    #[cfg(not(unix))]
    pub fn open(self) -> Result<GatewayListeners, GatewayListenerError> {
        Err(GatewayListenerError::Io(io::Error::new(
            io::ErrorKind::Unsupported,
            "the Gateway's listeners need a Unix system",
        )))
    }

    /// Obtains the listeners, by launchd activation or by binding, and checks that each is bound
    /// to exactly `127.0.0.1` and its port. Launchd activation works once per process.
    #[cfg(unix)]
    pub fn open(self) -> Result<GatewayListeners, GatewayListenerError> {
        let listeners = match self {
            Self::Launchd => activate_launchd_listeners()?,
            Self::Bind {
                http_port,
                https_port,
            } => match (bind(http_port), bind(https_port)) {
                (Ok(http), Ok(https)) => GatewayListeners { http, https },
                (http, https) => {
                    return Err(bind_failure([
                        (http_port, http.err()),
                        (https_port, https.err()),
                    ]));
                }
            },
        };
        let (http_port, https_port) = self.ports();
        let unavailable = [(&listeners.http, http_port), (&listeners.https, https_port)]
            .into_iter()
            .filter(|(listener, port)| !bound_to_loopback_port(listener, *port))
            .map(|(_listener, port)| port)
            .collect::<Vec<_>>();
        if !unavailable.is_empty() {
            return Err(GatewayListenerError::Unavailable { ports: unavailable });
        }

        Ok(listeners)
    }
}

/// The two listening sockets the Gateway serves on. Both are close-on-exec.
#[derive(Debug)]
pub struct GatewayListeners {
    #[cfg(unix)]
    pub http: OwnedFd,
    #[cfg(unix)]
    pub https: OwnedFd,
}

impl GatewayListeners {
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            #[cfg(unix)]
            http: self.http.try_clone()?,
            #[cfg(unix)]
            https: self.https.try_clone()?,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GatewayListenerError {
    /// Another program holds these ports, or launchd handed over a socket that isn't bound to
    /// them.
    #[error("ports {ports:?} are in use by another program")]
    Unavailable { ports: Vec<u16> },
    /// macOS: the daemon wasn't started by launchd.
    #[error("the daemon was not started by launchd")]
    NotLaunchdManaged,
    /// macOS: the running LaunchAgent has no `GatewayHttp`/`GatewayHttps` sockets: it was
    /// registered by an older PV.
    #[error("the LaunchAgent has no Gateway sockets")]
    LaunchAgentWithoutSockets,
    /// Linux: the kernel refused the bind, because the unprivileged-port setting isn't applied.
    #[error("the kernel does not let PV open ports 80 and 443")]
    NotPermitted,
    #[error(transparent)]
    Io(#[from] io::Error),
}

#[cfg(unix)]
fn bind(port: u16) -> io::Result<OwnedFd> {
    use socket2::{Domain, Socket, Type};

    // Close-on-exec from the start; the daemon passes the Gateway's copies on explicitly.
    let socket = Socket::new(Domain::IPV4, Type::STREAM, None)?;
    socket.set_reuse_address(true)?;
    socket.bind(&SocketAddrV4::new(Ipv4Addr::LOCALHOST, port).into())?;
    socket.listen(1024)?;

    Ok(socket.into())
}

#[cfg(unix)]
fn bind_failure(failures: [(u16, Option<io::Error>); 2]) -> GatewayListenerError {
    let mut unavailable = Vec::new();
    for (port, error) in failures {
        match error {
            None => {}
            Some(error) if error.kind() == io::ErrorKind::AddrInUse => unavailable.push(port),
            Some(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                return GatewayListenerError::NotPermitted;
            }
            Some(error) => return GatewayListenerError::Io(error),
        }
    }

    GatewayListenerError::Unavailable { ports: unavailable }
}

/// A socket launchd couldn't bind reports `0.0.0.0:0`, and calling `listen` on it would bind a
/// random port on every address. Only an exact `127.0.0.1:<port>` reaches Caddy.
#[cfg(unix)]
fn bound_to_loopback_port(listener: &OwnedFd, port: u16) -> bool {
    socket2::SockRef::from(listener)
        .local_addr()
        .ok()
        .and_then(|address| address.as_socket())
        == Some(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)))
}

#[cfg(target_os = "macos")]
fn activate_launchd_listeners() -> Result<GatewayListeners, GatewayListenerError> {
    Ok(GatewayListeners {
        http: activate_launchd_socket(GATEWAY_HTTP_SOCKET)?,
        https: activate_launchd_socket(GATEWAY_HTTPS_SOCKET)?,
    })
}

#[cfg(target_os = "macos")]
fn activate_launchd_socket(name: &str) -> Result<OwnedFd, GatewayListenerError> {
    use std::os::fd::FromRawFd;

    let fds = raunch::activate_socket(name)
        .map_err(|error| match error {
            raunch::Error::NotManaged => GatewayListenerError::NotLaunchdManaged,
            raunch::Error::NotInPlist => GatewayListenerError::LaunchAgentWithoutSockets,
            error => GatewayListenerError::Io(io::Error::other(format!(
                "could not activate launchd socket {name}: {error}"
            ))),
        })?
        .into_iter()
        // SAFETY: launchd just transferred ownership of these descriptors to this process,
        // `raunch` freed only the array, and nothing else in the process holds these numbers.
        .map(|fd| unsafe { OwnedFd::from_raw_fd(fd) })
        .collect::<Vec<_>>();
    let [fd]: [OwnedFd; 1] = fds.try_into().map_err(|fds: Vec<OwnedFd>| {
        io::Error::other(format!(
            "launchd provided {} sockets for {name}, expected exactly one",
            fds.len()
        ))
    })?;
    // Children the daemon starts must not inherit ports 80 and 443.
    rustix::io::fcntl_setfd(&fd, rustix::io::FdFlags::CLOEXEC).map_err(io::Error::from)?;
    // Activation leaves the socket blocking, and every copy shares that mode, including a Gateway
    // that kept running through a daemon restart. Caddy can't stop a blocking `accept`, so after
    // its next reload the old config would answer one more connection on each port.
    socket2::SockRef::from(&fd).set_nonblocking(true)?;

    Ok(fd)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn activate_launchd_listeners() -> Result<GatewayListeners, GatewayListenerError> {
    Err(GatewayListenerError::Io(io::Error::new(
        io::ErrorKind::Unsupported,
        "launchd sockets exist only on macOS",
    )))
}

#[cfg(all(test, unix))]
mod tests {
    use std::net::{Ipv4Addr, SocketAddrV4};

    use socket2::{Domain, Socket, Type};

    use super::bound_to_loopback_port;

    #[test]
    fn only_a_socket_bound_to_the_exact_loopback_port_is_accepted() -> anyhow::Result<()> {
        let [port, wildcard_port] = pv_fake::ports()?;
        let unbound = Socket::new(Domain::IPV4, Type::STREAM, None)?;
        let wildcard = Socket::new(Domain::IPV4, Type::STREAM, None)?;
        wildcard.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, wildcard_port).into())?;
        let loopback = Socket::new(Domain::IPV4, Type::STREAM, None)?;
        loopback.bind(&SocketAddrV4::new(Ipv4Addr::LOCALHOST, port).into())?;

        assert!(!bound_to_loopback_port(&unbound.into(), port));
        assert!(!bound_to_loopback_port(&wildcard.into(), wildcard_port));
        assert!(bound_to_loopback_port(&loopback.try_clone()?.into(), port));
        assert!(!bound_to_loopback_port(&loopback.into(), port + 1));

        Ok(())
    }
}
