//! Bind-conflict acceptance tests observe a separate process, regardless of signing ancestry.
#![cfg(target_os = "macos")]

use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::process::{Child, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Result, bail, ensure};
use camino_tempfile::tempdir;
use insta::assert_debug_snapshot;
use platform::loopback_tcp_port_available;
use pv_fake::{EventKind, FakeSettings, InstalledFake, Persona, TcpHolderConfig};
use socket2::{Domain, Protocol, Socket, Type};

#[expect(
    clippy::disallowed_types,
    reason = "port-probe acceptance tests own and reap their separate TCP holder"
)]
type HolderCommand = std::process::Command;

struct Holder {
    child: Child,
    fake: InstalledFake,
}

impl Holder {
    fn stop(&mut self) -> Result<()> {
        if self.child.try_wait()?.is_none() {
            self.child.kill()?;
        }
        self.child.wait()?;
        Ok(())
    }

    fn wait_for(&mut self, ready: impl Fn(&EventKind) -> bool) -> Result<SocketAddr> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let events = self.fake.events()?;
            if events.iter().any(|event| ready(&event.kind))
                && let Some(address) = events.iter().find_map(|event| match event.kind {
                    EventKind::TcpReady { address } => Some(address),
                    _ => None,
                })
            {
                return Ok(address);
            }
            if let Some(status) = self.child.try_wait()? {
                bail!("TCP holder exited before readiness: {status}");
            }
            ensure!(
                Instant::now() < deadline,
                "TCP holder did not report readiness"
            );
            sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Holder {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            let _write_result = writeln!(std::io::stderr(), "TCP holder cleanup failed: {error:#}");
        }
    }
}

fn start_holder(fake: InstalledFake) -> Result<Holder> {
    let child = HolderCommand::new(fake.executable())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()?;
    Ok(Holder { child, fake })
}

#[test]
fn probes_separate_process_socket_classes() -> Result<()> {
    for (name, address, reuse_address, reuse_port, ipv6_only, listen, expected) in [
        (
            "ipv4_loopback",
            "127.0.0.1:0",
            false,
            false,
            false,
            true,
            false,
        ),
        (
            "ipv4_wildcard",
            "0.0.0.0:0",
            false,
            false,
            false,
            true,
            false,
        ),
        (
            "ipv4_wildcard_reuse_port",
            "0.0.0.0:0",
            false,
            true,
            false,
            true,
            false,
        ),
        (
            "ipv4_wildcard_reuse_address",
            "0.0.0.0:0",
            true,
            false,
            false,
            true,
            false,
        ),
        (
            "ipv6_dual_stack_wildcard",
            "[::]:0",
            false,
            false,
            false,
            true,
            false,
        ),
        (
            "ipv4_bound_without_listen",
            "127.0.0.1:0",
            false,
            false,
            false,
            false,
            false,
        ),
        (
            "ipv6_only_wildcard",
            "[::]:0",
            false,
            false,
            true,
            true,
            true,
        ),
        ("ipv6_loopback", "[::1]:0", false, false, false, true, true),
    ] {
        let tempdir = tempdir()?;
        // IPv6-only allocation does not reserve the same number in IPv4's port space.
        let reservation = if expected {
            Some(TcpListener::bind(("0.0.0.0", 0))?)
        } else {
            None
        };
        let mut address: SocketAddr = address.parse()?;
        if let Some(reservation) = &reservation {
            address.set_port(reservation.local_addr()?.port());
        }
        let fake = pv_fake::install_with_settings(
            &tempdir.path().join("tcp-holder"),
            Persona::TcpHolder,
            FakeSettings {
                tcp_holder: Some(TcpHolderConfig {
                    address,
                    reuse_address,
                    reuse_port,
                    ipv6_only,
                    listen,
                }),
                ..FakeSettings::default()
            },
        )?;
        let mut holder = start_holder(fake)?;
        let address = holder.wait_for(|event| matches!(event, EventKind::TcpReady { .. }))?;
        drop(reservation);
        let available = loopback_tcp_port_available(address.port());
        holder.stop()?;
        assert_eq!(available, expected, "unexpected probe verdict for {name}");
        assert_debug_snapshot!(name, available);
    }
    Ok(())
}

#[test]
fn server_time_wait_does_not_churn_the_port() -> Result<()> {
    let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
    socket.set_reuse_address(true)?;
    socket.bind(&"127.0.0.1:0".parse::<SocketAddr>()?.into())?;
    socket.listen(1)?;
    let listener = TcpListener::from(socket);
    let address = listener.local_addr()?;
    let mut client = TcpStream::connect(address)?;
    client.set_read_timeout(Some(Duration::from_secs(5)))?;
    let (mut server, _) = listener.accept()?;
    server.set_read_timeout(Some(Duration::from_secs(5)))?;
    server.shutdown(Shutdown::Write)?;
    client.read_to_end(&mut Vec::new())?;
    client.shutdown(Shutdown::Write)?;
    server.read_to_end(&mut Vec::new())?;
    drop(server);
    drop(client);
    drop(listener);
    // An explicit non-reuse bind proves the server-side TIME_WAIT still holds the port.
    let plain = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
    plain.set_reuse_address(false)?;
    assert!(plain.bind(&address.into()).is_err());
    let available = loopback_tcp_port_available(address.port());
    assert!(available);
    assert_debug_snapshot!(available);
    Ok(())
}

#[test]
fn killed_server_with_connected_client_does_not_churn_the_port() -> Result<()> {
    let tempdir = tempdir()?;
    let fake = pv_fake::install_with_settings(
        &tempdir.path().join("tcp-holder"),
        Persona::TcpHolder,
        FakeSettings {
            tcp_holder: Some(TcpHolderConfig {
                address: "127.0.0.1:0".parse()?,
                reuse_address: true,
                reuse_port: false,
                ipv6_only: false,
                listen: true,
            }),
            ..FakeSettings::default()
        },
    )?;
    let mut holder = start_holder(fake)?;
    let address = holder.wait_for(|event| matches!(event, EventKind::TcpReady { .. }))?;
    let client = TcpStream::connect(address)?;
    holder.wait_for(|event| matches!(event, EventKind::TcpAccepted))?;
    holder.stop()?;
    // Keep the client socket alive while probing after SIGKILL has reaped the server.
    let available = loopback_tcp_port_available(address.port());
    drop(client);
    assert!(available);
    assert_debug_snapshot!(available);
    Ok(())
}
