use std::collections::BTreeSet;

use crate::PlatformError;

#[cfg(target_os = "linux")]
#[path = "listener/linux.rs"]
mod implementation;
#[cfg(target_os = "macos")]
#[path = "listener/macos.rs"]
mod implementation;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
#[path = "listener/unsupported.rs"]
mod implementation;
#[cfg(target_os = "windows")]
#[path = "listener/windows.rs"]
mod implementation;

pub fn loopback_tcp_listener_ports() -> Result<BTreeSet<u16>, PlatformError> {
    implementation::loopback_tcp_listener_ports()
}

pub fn loopback_tcp_port_has_listener(port: u16) -> Result<bool, PlatformError> {
    Ok(loopback_tcp_listener_ports()?.contains(&port))
}
