use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ReleaseError {
    #[error("invalid release record `{path}`: {reason}")]
    InvalidReleaseRecord { path: String, reason: String },

    #[error("invalid app release record `{path}`: {reason}")]
    InvalidAppReleaseRecord { path: String, reason: String },

    #[error("invalid revocation record `{path}`: {reason}")]
    InvalidRevocationRecord { path: String, reason: String },

    #[error("invalid manifest default tracks `{path}`: {reason}")]
    InvalidDefaultTracks { path: String, reason: String },

    #[error("invalid recipe metadata `{path}`: {reason}")]
    InvalidRecipeMetadata { path: String, reason: String },

    #[error("duplicate artifact identity `{identity}`")]
    DuplicateArtifactIdentity { identity: String },

    #[error("duplicate app release record for {platform}")]
    DuplicateAppReleasePlatform { platform: String },

    #[error(
        "app release records must agree on {field}: expected `{expected}`, got `{actual}` in `{path}`"
    )]
    AppReleaseMetadataMismatch {
        field: &'static str,
        expected: String,
        actual: String,
        path: String,
    },

    #[error("revocation `{revocation}` references missing artifact `{identity}`")]
    RevocationTargetMissing {
        revocation: String,
        identity: String,
    },

    #[error("revocation `{revocation}` has invalid replacement `{replacement}`: {reason}")]
    RevocationReplacementInvalid {
        revocation: String,
        replacement: String,
        reason: String,
    },

    #[error("conflicting revocation for artifact `{identity}`")]
    ConflictingRevocation { identity: String },

    #[error("duplicate revocation for artifact `{identity}`")]
    DuplicateRevocation { identity: String },

    #[error("invalid artifact archive `{path}`: {reason}")]
    InvalidArchive { path: String, reason: String },

    #[error("invalid publication input `{path}`: {reason}")]
    InvalidPublicationInput { path: String, reason: String },

    #[error("publication would overwrite immutable object `{key}`")]
    ImmutablePublicationObjectExists { key: String },

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

    #[error("smoke hook `{hook}` failed with status {status}")]
    SmokeHookFailed { hook: String, status: String },

    #[error("smoke hook `{hook}` timed out after {timeout}")]
    SmokeHookTimedOut { hook: String, timeout: String },

    #[error("generated manifest is invalid: {reason}")]
    GeneratedManifestInvalid { reason: String },

    #[error("generated app manifest is invalid: {reason}")]
    GeneratedAppManifestInvalid { reason: String },

    #[error("filesystem error at `{path}`: {reason}")]
    Filesystem { path: String, reason: String },
}

pub type Result<T> = std::result::Result<T, ReleaseError>;
