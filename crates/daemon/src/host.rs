//! How the daemon reaches the host: where the Gateway's ports 80 and 443 come from, and whether it
//! keeps `.test` DNS configured.

use std::sync::{Mutex, PoisonError};

use platform::{GatewayListenerError, GatewayListenerSource, GatewayListeners};

use crate::DaemonError;

/// What the daemon takes from the host. `pv daemon:run` uses [`HostIntegration::production`];
/// tests bind two test ports instead and never repair DNS.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostIntegration {
    pub gateway_listeners: GatewayListenerSource,
    pub dns_repair: DnsRepair,
}

impl HostIntegration {
    pub const fn production() -> Self {
        Self {
            gateway_listeners: GatewayListenerSource::production(),
            dns_repair: DnsRepair::Helper,
        }
    }
}

/// Whether the daemon asks PV's privileged helper to keep the system's `.test` resolver
/// configuration pointed at PV's DNS port.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DnsRepair {
    Helper,
    Off,
}

/// The Gateway's two listening sockets, held for the daemon's lifetime. launchd hands its sockets
/// over once per process, so they are taken when the daemon starts. Bound sockets wait until a
/// Gateway first has to start, by when the previous Gateway has released the ports.
#[derive(Debug)]
pub struct GatewayListenerHolder {
    source: GatewayListenerSource,
    listeners: Mutex<Option<GatewayListeners>>,
    /// Why launchd's sockets are unusable, for every later start in this process.
    launchd_failure: Option<String>,
}

impl GatewayListenerHolder {
    pub fn new(source: GatewayListenerSource) -> Self {
        let (listeners, launchd_failure) = match source {
            GatewayListenerSource::Launchd => match source.open() {
                Ok(listeners) => (Some(listeners), None),
                Err(error) => (None, Some(failure_message(&error))),
            },
            GatewayListenerSource::Bind { .. } => (None, None),
        };

        Self {
            source,
            listeners: Mutex::new(listeners),
            launchd_failure,
        }
    }

    pub fn source(&self) -> GatewayListenerSource {
        self.source
    }

    /// Copies of the listeners for a Gateway about to start, binding them first if needed.
    pub(crate) fn for_start(&self) -> Result<GatewayListeners, DaemonError> {
        if let Some(message) = &self.launchd_failure {
            return Err(DaemonError::GatewayListeners {
                message: message.clone(),
            });
        }
        let mut held = self
            .listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let listeners = match held.take() {
            Some(listeners) => listeners,
            None => self
                .source
                .open()
                .map_err(|error| DaemonError::GatewayListeners {
                    message: failure_message(&error),
                })?,
        };
        let copies = listeners.try_clone();
        *held = Some(listeners);

        Ok(copies?)
    }
}

const IN_USE_BY_ANOTHER_PROGRAM: &str = "is in use by another program";
const IN_USE_BY_OTHER_PROGRAMS: &str = "are in use by other programs";

/// Whether the Gateway failed because other programs hold its ports, so `pv doctor` can name them.
/// Any other failure leaves the ports with PV.
pub fn gateway_failure_is_port_conflict(message: &str) -> bool {
    message.contains(IN_USE_BY_ANOTHER_PROGRAM) || message.contains(IN_USE_BY_OTHER_PROGRAMS)
}

/// What a user sees on the Gateway when it has no listeners: why, and the command that repairs it.
fn failure_message(error: &GatewayListenerError) -> String {
    match error {
        GatewayListenerError::Unavailable { ports } => match ports.as_slice() {
            [port] => format!(
                "Port {port} {IN_USE_BY_ANOTHER_PROGRAM}; free it, then run `pv daemon:restart`."
            ),
            ports => format!(
                "Ports {} {IN_USE_BY_OTHER_PROGRAMS}; free them, then run `pv daemon:restart`.",
                ports
                    .iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(" and ")
            ),
        },
        GatewayListenerError::NotLaunchdManaged => {
            "PV's daemon was not started by launchd; run `pv daemon:enable`.".to_owned()
        }
        GatewayListenerError::LaunchAgentWithoutSockets => {
            "PV's LaunchAgent predates port sockets; run `pv daemon:restart`.".to_owned()
        }
        GatewayListenerError::NotPermitted => {
            "Ports 80 and 443 are not available to PV; run `pv setup`.".to_owned()
        }
        GatewayListenerError::Io(error) => format!("PV could not open ports 80 and 443: {error}"),
    }
}
