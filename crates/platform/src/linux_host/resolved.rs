use std::time::Duration;

use camino::Utf8Path;
use state::fs;
use tokio::runtime::Builder;
use zbus::Connection;
use zbus::zvariant::OwnedObjectPath;

use crate::PlatformError;
use crate::clock::{self, Limit};
use crate::systemd::native_proxy;

const INSPECTION: Limit = Limit::new("resolved stub inspection", Duration::from_secs(10));

pub(super) fn check_stub() -> Result<(), PlatformError> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(resolved_error)?;
    runtime.block_on(async {
        clock::timeout(&INSPECTION, async {
            let connection = Connection::system().await.map_err(resolved_error)?;
            let manager = native_proxy(&connection, "/org/freedesktop/systemd1".try_into().map_err(resolved_error)?, "org.freedesktop.systemd1.Manager").await.map_err(resolved_error)?;
            let path: OwnedObjectPath = manager.call("GetUnit", &("systemd-resolved.service",)).await.map_err(resolved_error)?;
            let unit = native_proxy(&connection, path, "org.freedesktop.systemd1.Unit").await.map_err(resolved_error)?;
            let active: String = unit.get_property("ActiveState").await.map_err(resolved_error)?;
            if active != "active" { return Err(resolved_error(format!("service is {active}"))); }
            let content = fs::read_to_string(Utf8Path::new("/etc/resolv.conf")).map_err(resolved_error)?;
            let nameservers: Vec<_> = content.lines().filter_map(|line| {
                let mut words = line.split_whitespace();
                (words.next() == Some("nameserver")).then(|| words.next()).flatten()
            }).collect();
            if nameservers != ["127.0.0.53"] {
                return Err(resolved_error("/etc/resolv.conf must list only nameserver 127.0.0.53; link it to /run/systemd/resolve/stub-resolv.conf"));
            }
            Ok(())
        }).await.map_err(resolved_error)?
    })
}

fn resolved_error(error: impl std::fmt::Display) -> PlatformError {
    PlatformError::SystemIntegration(format!("systemd-resolved stub is unavailable: {error}"))
}
