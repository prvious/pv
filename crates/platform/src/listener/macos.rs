use std::collections::BTreeSet;

use crate::PlatformError;

#[path = "macos/kernel_table.rs"]
mod kernel_table;

pub(super) fn loopback_tcp_listener_ports() -> Result<BTreeSet<u16>, PlatformError> {
    kernel_table::loopback_tcp_listener_ports().map_err(|source| {
        PlatformError::ListenerInspection {
            source: Box::new(source),
        }
    })
}
