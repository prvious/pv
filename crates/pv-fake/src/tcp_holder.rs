use std::future::pending;
use std::net::TcpListener as StdTcpListener;

use anyhow::{Context, Result};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::TcpListener;

use crate::FakeSettings;
use crate::events::{EventKind, EventLog};

pub(crate) async fn start(settings: &FakeSettings, events: &EventLog) -> Result<()> {
    let config = settings
        .tcp_holder
        .as_ref()
        .context("TCP holder has no socket configuration")?;
    let socket = Socket::new(
        Domain::for_address(config.address),
        Type::STREAM,
        Some(Protocol::TCP),
    )?;
    socket.set_reuse_address(config.reuse_address)?;
    socket.set_reuse_port(config.reuse_port)?;
    if config.address.is_ipv6() {
        socket.set_only_v6(config.ipv6_only)?;
    }
    socket.bind(&config.address.into())?;
    if config.listen {
        socket.listen(128)?;
    }
    let address = socket
        .local_addr()?
        .as_socket()
        .context("TCP holder address is not IP")?;
    events.record(EventKind::TcpReady { address })?;
    let _client = if config.listen {
        socket.set_nonblocking(true)?;
        let listener = TcpListener::from_std(StdTcpListener::from(socket.try_clone()?))?;
        let client = crate::accept(&listener).await;
        events.record(EventKind::TcpAccepted)?;
        Some(client)
    } else {
        None
    };
    // The persona future owns both sockets until the fake's signal handler cancels it.
    pending::<Result<()>>().await
}
