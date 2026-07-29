use std::io;

use camino::Utf8PathBuf;
use thiserror::Error;

use crate::{CaRepairReason, PlatformCapability, PlatformTarget};

#[derive(Debug, Error)]
pub enum PlatformError {
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

    #[error("browser open failed with {status}")]
    BrowserOpenStatus { status: String },

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

    #[cfg(target_os = "macos")]
    #[error("could not inspect TCP listeners: {source}")]
    ListenerInspection {
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[cfg(target_os = "macos")]
    #[error("could not inspect process identity: {source}")]
    ProcessIdentityInspection {
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}
