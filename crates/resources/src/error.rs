use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ResourcesError {
    #[error("unknown Managed Resource `{name}`")]
    UnknownResource { name: String },

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
}

pub type Result<T> = std::result::Result<T, ResourcesError>;
