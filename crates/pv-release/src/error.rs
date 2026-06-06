use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ReleaseError {
    #[error("invalid release record `{path}`: {reason}")]
    InvalidReleaseRecord { path: String, reason: String },

    #[error("invalid revocation record `{path}`: {reason}")]
    InvalidRevocationRecord { path: String, reason: String },

    #[error("duplicate artifact identity `{identity}`")]
    DuplicateArtifactIdentity { identity: String },

    #[error("revocation `{revocation}` references missing artifact `{identity}`")]
    RevocationTargetMissing {
        revocation: String,
        identity: String,
    },

    #[error("conflicting revocation for artifact `{identity}`")]
    ConflictingRevocation { identity: String },

    #[error("invalid artifact archive `{path}`: {reason}")]
    InvalidArchive { path: String, reason: String },

    #[error("artifact archive `{path}` checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch {
        path: String,
        expected: String,
        actual: String,
    },

    #[error("artifact archive `{path}` size mismatch: expected {expected}, got {actual}")]
    SizeMismatch {
        path: String,
        expected: u64,
        actual: u64,
    },

    #[error("relocation scan failed for `{path}`: {reason}")]
    Relocation { path: String, reason: String },

    #[error("smoke hook `{hook}` failed with status {status}")]
    SmokeHookFailed { hook: String, status: String },

    #[error("generated manifest is invalid: {reason}")]
    GeneratedManifestInvalid { reason: String },

    #[error("filesystem error at `{path}`: {reason}")]
    Filesystem { path: String, reason: String },
}

pub type Result<T> = std::result::Result<T, ReleaseError>;
