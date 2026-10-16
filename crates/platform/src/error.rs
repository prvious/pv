use std::io;

use camino::Utf8PathBuf;
use thiserror::Error;

use crate::{CaRepairReason, PlatformCapability, PlatformTarget};

#[derive(Debug, Error)]
pub enum PlatformError {
    #[error("could not inspect kernel boot identity: {0}")]
    BootSessionInspection(#[source] io::Error),

    #[error("could not generate PV local CA: {0}")]
    CaGeneration(#[from] rcgen::Error),

    #[error("could not generate Project TLS certificate: {0}")]
    ProjectCertificateGeneration(#[source] rcgen::Error),

    #[error("could not parse PEM file: {0}")]
    Pem(#[from] io::Error),

    #[error("could not parse X.509 certificate")]
    X509,

    #[error("local CA certificate is not a usable PV root CA")]
    InvalidCaShape,

    #[error("could not parse local CA private key")]
    MalformedPrivateKey,

    #[error("local CA certificate and private key do not match")]
    KeyMismatch,

    #[error("{capability} is unsupported on {target}")]
    Unsupported {
        capability: PlatformCapability,
        target: PlatformTarget,
    },

    #[error("{target} is an unsupported platform target")]
    UnsupportedTarget { target: &'static str },

    #[error("could not open URL in the browser: {0}")]
    BrowserOpen(#[source] io::Error),

    #[error("generated PV local CA files are missing after writing")]
    LocalCaPostWriteMissing,

    #[error("generated PV local CA requires repair after writing: {reason:?}")]
    LocalCaPostWriteRepairRequired { reason: CaRepairReason },

    #[error("generated PV local CA is unreadable after writing at {path}: {message}")]
    LocalCaPostWriteUnreadable { path: Utf8PathBuf, message: String },

    #[error("macOS keychain inspection failed: {0}")]
    Keychain(String),

    #[error("LaunchAgent operation failed: {0}")]
    LaunchAgent(String),

    #[error("could not run LaunchAgent command `{command}`: {source}")]
    LaunchAgentCommand {
        command: String,
        #[source]
        source: io::Error,
    },

    #[error("LaunchAgent command `{command}` exited with {status}")]
    LaunchAgentCommandStatus { command: String, status: String },

    #[error("daemon registration file operation failed: {0}")]
    DaemonRegistrationFile(#[source] state::StateError),

    #[error("daemon registration file is not PV-owned: {path}")]
    DaemonRegistrationConflict { path: Utf8PathBuf },

    #[error("daemon registration file could not be read at {path}: {message}")]
    DaemonRegistrationUnreadable { path: Utf8PathBuf, message: String },

    #[cfg(target_os = "linux")]
    #[error(
        "systemd user manager unavailable: DBUS_SESSION_BUS_ADDRESS and a valid XDG_RUNTIME_DIR are missing"
    )]
    UserManagerSessionEnvironment,

    #[cfg(target_os = "linux")]
    #[error("systemd user manager unavailable: {0}")]
    UserManagerUnavailable(#[source] Box<zbus::Error>),

    #[cfg(target_os = "linux")]
    #[error("daemon registration D-Bus operation failed: {0}")]
    DaemonRegistrationBus(#[source] Box<zbus::Error>),

    #[cfg(target_os = "linux")]
    #[error("privileged helper systemd operation failed: {0}")]
    PrivilegedHelperBus(#[source] Box<zbus::Error>),

    #[cfg(target_os = "linux")]
    #[error("could not inspect Linux TCP listeners: {0}")]
    LowPortProcfs(#[source] Box<procfs::ProcError>),

    #[cfg(target_os = "linux")]
    #[error("socket proxy D-Bus operation failed: {0}")]
    PortProxyBus(#[source] Box<zbus::Error>),

    #[cfg(target_os = "linux")]
    #[error("socket proxy unit I/O failed: {0}")]
    PortProxyIo(#[source] io::Error),

    #[cfg(target_os = "linux")]
    #[error(
        "socket proxy {operation} failed: {source}; port routing may be partial and a queued or in-flight request may still complete; inspect pv-http and pv-https units, their .candidate and .rollback files with an administrator before retrying `pv ports:install` or `pv ports:uninstall`"
    )]
    PortProxyMutation {
        operation: &'static str,
        #[source]
        source: Box<PlatformError>,
    },

    #[cfg(target_os = "linux")]
    #[error("resolver D-Bus operation failed: {0}")]
    ResolverBus(#[source] Box<zbus::Error>),

    #[cfg(target_os = "linux")]
    #[error("resolver netlink operation failed: {0}")]
    ResolverNetlink(#[source] Box<rtnetlink::Error>),

    #[cfg(target_os = "linux")]
    #[error("resolver I/O failed: {0}")]
    ResolverIo(#[source] io::Error),

    #[cfg(target_os = "linux")]
    #[error(
        "resolver {operation} failed: {source}; DNS state may be partial and a queued or in-flight request may still complete; inspect pv-dns.service, pv-dns.service.candidate and pv0 with an administrator before retrying `pv dns:install` or `pv dns:uninstall`"
    )]
    ResolverMutation {
        operation: &'static str,
        #[source]
        source: Box<PlatformError>,
    },

    #[cfg(target_os = "linux")]
    #[error("daemon registration {operation} failed: {message}")]
    DaemonRegistrationState {
        operation: &'static str,
        message: String,
    },

    #[error("system integration operation failed: {0}")]
    SystemIntegration(String),

    #[error("could not run system integration command `{command}`: {source}")]
    SystemIntegrationCommand {
        command: String,
        #[source]
        source: io::Error,
    },

    #[error("system integration command `{command}` exited with {status}")]
    SystemIntegrationCommandStatus { command: String, status: String },

    #[error("PV privileged helper is unavailable; run `pv setup`")]
    PrivilegedHelperUnavailable,

    #[error(
        "PV privileged helper protocol {actual} is incompatible with required protocol {expected}; run `pv setup`"
    )]
    PrivilegedHelperProtocolMismatch { expected: u32, actual: u32 },

    #[error("PV privileged helper rejected the request: {message}")]
    PrivilegedHelperRejected { message: String },

    #[error("PV privileged helper returned {code}: {message}")]
    PrivilegedHelperRemote { code: String, message: String },

    #[error("PV privileged helper I/O failed: {0}")]
    PrivilegedHelperIo(#[source] io::Error),

    #[error("PV privileged helper protocol failed: {0}")]
    PrivilegedHelperProtocol(#[from] serde_json::Error),

    #[error("PV privileged helper installation failed: {0}")]
    PrivilegedHelperInstallation(String),

    #[cfg(target_os = "linux")]
    #[error("{source}; cleanup warning: {cleanup_warning}")]
    PrivilegedHelperVerification {
        #[source]
        source: Box<PlatformError>,
        cleanup_warning: String,
    },

    #[error("PV privileged helper authentication failed: {0}")]
    PrivilegedHelperAuthentication(String),

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[error("could not inspect process identity: {source}")]
    ProcessIdentityInspection {
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

#[cfg(target_os = "linux")]
impl From<zbus::Error> for PlatformError {
    fn from(error: zbus::Error) -> Self {
        Self::DaemonRegistrationBus(Box::new(error))
    }
}
