#![cfg(unix)]

use std::net::{Ipv4Addr, SocketAddr, TcpListener};

use anyhow::Result;
use platform::{GatewayListenerError, GatewayListenerSource};
use rustix::io::{FdFlags, fcntl_getfd};

#[test]
fn bound_listeners_are_close_on_exec_on_their_exact_loopback_ports() -> Result<()> {
    let [http_port, https_port] = pv_fake::ports()?;

    let listeners = GatewayListenerSource::Bind {
        http_port,
        https_port,
    }
    .open()?;

    for (listener, port) in [(&listeners.http, http_port), (&listeners.https, https_port)] {
        assert!(fcntl_getfd(listener)?.contains(FdFlags::CLOEXEC));
        assert_eq!(
            socket2::SockRef::from(listener).local_addr()?.as_socket(),
            Some(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
        );
    }

    Ok(())
}

#[test]
fn held_ports_are_named_as_unavailable() -> Result<()> {
    let [http_port, https_port] = pv_fake::ports()?;
    let held = TcpListener::bind((Ipv4Addr::LOCALHOST, https_port))?;

    let error = GatewayListenerSource::Bind {
        http_port,
        https_port,
    }
    .open()
    .err();
    drop(held);

    assert!(
        matches!(&error, Some(GatewayListenerError::Unavailable { ports }) if *ports == [https_port]),
        "{error:?}"
    );

    Ok(())
}
