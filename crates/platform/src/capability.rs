use std::fmt;

use crate::{PlatformError, PlatformTarget};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformCapability {
    BrowserHandoff,
    DaemonIpc,
    DaemonRegistration,
    ListenerInspection,
    LowPortFrontend,
    ProcessContainment,
    ResolverIntegration,
    TrustStore,
}

impl PlatformCapability {
    const fn as_str(self) -> &'static str {
        match self {
            Self::BrowserHandoff => "browser handoff",
            Self::DaemonIpc => "daemon IPC",
            Self::DaemonRegistration => "daemon registration",
            Self::ListenerInspection => "listener inspection",
            Self::LowPortFrontend => "low-port frontend",
            Self::ProcessContainment => "process containment",
            Self::ResolverIntegration => "resolver integration",
            Self::TrustStore => "trust store",
        }
    }
}

impl fmt::Display for PlatformCapability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

pub fn require_capability(capability: PlatformCapability) -> Result<(), PlatformError> {
    require_capability_for(PlatformTarget::current()?, capability)
}

pub(crate) fn require_capability_for(
    target: PlatformTarget,
    capability: PlatformCapability,
) -> Result<(), PlatformError> {
    match target {
        PlatformTarget::Macos => Ok(()),
        PlatformTarget::Linux | PlatformTarget::Windows => {
            Err(PlatformError::Unsupported { capability, target })
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn unsupported(capability: PlatformCapability) -> Result<PlatformError, PlatformError> {
    Ok(PlatformError::Unsupported {
        capability,
        target: PlatformTarget::current()?,
    })
}
