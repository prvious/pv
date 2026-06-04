use std::io;

use camino::Utf8PathBuf;
use thiserror::Error;

use crate::ca::CaRepairReason;

#[derive(Debug, Error)]
pub enum PlatformError {
    #[error("could not generate PV local CA: {0}")]
    CaGeneration(#[from] rcgen::Error),

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

    #[error("generated PV local CA files are missing after writing")]
    LocalCaPostWriteMissing,

    #[error("generated PV local CA requires repair after writing: {reason:?}")]
    LocalCaPostWriteRepairRequired { reason: CaRepairReason },

    #[error("generated PV local CA is unreadable after writing at {path}: {message}")]
    LocalCaPostWriteUnreadable { path: Utf8PathBuf, message: String },

    #[error("macOS keychain inspection failed: {0}")]
    Keychain(String),

    #[error("could not inspect socket table: {0}")]
    SocketTable(#[from] netstat::Error),

    #[error("could not run netstat for socket inspection: {0}")]
    SocketTableCommand(#[source] io::Error),

    #[error("netstat socket inspection exited with {status}")]
    SocketTableCommandStatus { status: String },

    #[error("could not decode netstat socket table: {0}")]
    SocketTableCommandUtf8(#[from] std::string::FromUtf8Error),
}
