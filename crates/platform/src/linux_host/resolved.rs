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
const STUB_ADDRESS: &str = "127.0.0.53";

/// Checks that systemd-resolved is running.
pub(super) fn check_active() -> Result<(), PlatformError> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(resolved_error)?;
    runtime.block_on(async {
        clock::timeout(&INSPECTION, async {
            let connection = Connection::system().await.map_err(resolved_error)?;
            let manager = native_proxy(
                &connection,
                "/org/freedesktop/systemd1"
                    .try_into()
                    .map_err(resolved_error)?,
                "org.freedesktop.systemd1.Manager",
            )
            .await
            .map_err(resolved_error)?;
            let path: OwnedObjectPath = manager
                .call("GetUnit", &("systemd-resolved.service",))
                .await
                .map_err(resolved_error)?;
            let unit = native_proxy(&connection, path, "org.freedesktop.systemd1.Unit")
                .await
                .map_err(resolved_error)?;
            let active: String = unit
                .get_property("ActiveState")
                .await
                .map_err(resolved_error)?;
            if active == "active" {
                Ok(())
            } else {
                Err(resolved_error(format!("service is {active}")))
            }
        })
        .await
        .map_err(resolved_error)?
    })
}

/// Checks systemd-resolved is running and `/etc/resolv.conf` asks its stub first, so `.test`
/// lookups reach it.
pub(super) fn check_stub() -> Result<(), PlatformError> {
    check_active()?;
    let content = fs::read_to_string(Utf8Path::new("/etc/resolv.conf")).map_err(resolved_error)?;
    let first_nameserver = content.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        (words.next() == Some("nameserver"))
            .then(|| words.next())
            .flatten()
    });
    if first_nameserver == Some(STUB_ADDRESS) {
        Ok(())
    } else {
        Err(resolved_error(format!(
            "/etc/resolv.conf must list nameserver {STUB_ADDRESS} first; \
             link it to /run/systemd/resolve/stub-resolv.conf"
        )))
    }
}

fn resolved_error(error: impl std::fmt::Display) -> PlatformError {
    PlatformError::SystemIntegration(format!("systemd-resolved stub is unavailable: {error}"))
}
