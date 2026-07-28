use std::collections::BTreeSet;

use crate::capability::unsupported;
use crate::{PlatformCapability, PlatformError};

pub(super) fn loopback_tcp_listener_ports() -> Result<BTreeSet<u16>, PlatformError> {
    Err(unsupported(PlatformCapability::ListenerInspection)?)
}
