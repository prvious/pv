use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ResourcesError {
    #[error("unknown Managed Resource `{name}`")]
    UnknownResource { name: String },

    #[error("artifact manifest does not include Managed Resource `{resource}`")]
    ResourceNotInManifest { resource: String },

    #[error("artifact manifest resource `{resource}` has no track `{track}`")]
    TrackNotFound { resource: String, track: String },

    #[error(
        "unsupported artifact manifest schema version {schema_version}, expected {supported_schema_version}"
    )]
    UnsupportedManifestSchema {
        schema_version: u64,
        supported_schema_version: u64,
    },

    #[error(
        "artifact manifest requires PV {minimum_pv_version}, current PV is {current_pv_version}"
    )]
    RequiresNewerPv {
        minimum_pv_version: String,
        current_pv_version: String,
    },

    #[error(
        "artifact manifests must use canonical resource name `{canonical}`, not alias `{alias}`"
    )]
    ManifestUsesAlias {
        alias: String,
        canonical: &'static str,
    },

    #[error("invalid {kind} `{value}`")]
    InvalidIdentity { kind: &'static str, value: String },

    #[error("invalid artifact manifest: {reason}")]
    InvalidManifest { reason: String },

    #[error("artifact manifest track name `{name}` is reserved")]
    ReservedTrackName { name: String },

    #[error("invalid artifact revocation state: {reason}")]
    InvalidRevocationState { reason: &'static str },

    #[error("unsupported artifact platform `{platform}`")]
    UnsupportedPlatform { platform: String },

    #[error("artifact selection is ambiguous for {resource} track {track} on {platform}")]
    AmbiguousArtifactSelection {
        resource: String,
        track: String,
        platform: String,
    },

    #[error("no installable artifact exists for {resource} track {track} on {platform}")]
    NoInstallableArtifact {
        resource: String,
        track: String,
        platform: String,
    },

    #[error("failed to parse published_at `{value}`")]
    InvalidPublishedAt { value: String },

    #[error("artifact manifest is unavailable from `{url}` and cache `{cache_path}`: {reason}")]
    ManifestUnavailable {
        url: String,
        cache_path: String,
        reason: String,
    },

    #[error("HTTP request failed for `{url}`: {reason}")]
    HttpRequestFailed { url: String, reason: String },

    #[error("HTTP status {status_code} for `{url}`")]
    HttpStatusFailed { url: String, status_code: u16 },

    #[error("failed to write download from `{url}`: {reason}")]
    DownloadWriteFailed { url: String, reason: String },

    #[error("invalid artifact manifest URL `{url}`")]
    InvalidManifestUrl { url: String },

    #[error("invalid artifact URL `{url}`")]
    InvalidArtifactUrl { url: String },

    #[error("artifact checksum mismatch for `{url}`: expected {expected}, got {actual}")]
    ArtifactChecksumMismatch {
        url: String,
        expected: String,
        actual: String,
    },

    #[error("filesystem error at `{path}`: {reason}")]
    Filesystem { path: String, reason: String },
}

pub type Result<T> = std::result::Result<T, ResourcesError>;
