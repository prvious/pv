pub mod allocation;
pub mod cache;
pub mod command;
pub mod download;
pub mod error;
mod fs;
pub mod http;
pub mod identity;
pub mod install;
pub mod manifest;
pub mod platform;
pub mod registry;
pub mod runtime;

pub use allocation::{
    EnvPlaceholderContract, ResourceAllocationKind, ResourceAllocationName,
    allocation_env_placeholders, generated_allocation_name, resource_env_placeholders,
};
pub use cache::{ArtifactManifestCache, ArtifactManifestRefresh, ArtifactManifestSource};
pub use command::{
    ManagedResourceCommandError, ManagedResourceCommandResult, ManagedResourceCommands,
    ManagedResourceInstall, ManagedResourceRemovalIntent, ManagedResourceRevokedLatest,
    ManagedResourceTrack, ManagedResourceUninstallOptions, ManagedResourceUpdate, PhpPairInstall,
    PhpPairRemovalIntent, PhpPairUpdate,
};
pub use download::{ArtifactDownload, ArtifactDownloader};
pub use error::{ResourcesError, Result};
pub use http::{ResourceHttpClient, UreqResourceHttpClient};
pub use identity::{
    ArtifactVersion, ConcreteTrackName, PublishedAt, PvVersion, ResourceName, Sha256Digest,
    TrackName, TrackSelector,
};
pub use install::{ArtifactInstall, ArtifactInstaller, ResourceAdapter};
pub use manifest::{ArtifactManifest, ManifestArtifact, ManifestSelection, RevocationState};
pub use platform::{ArtifactPlatform, TargetPlatform};
pub use registry::{ResourceCapability, ResourceDescriptor, ResourceKind};
pub use runtime::{
    RuntimeArtifactAdapter, composer_adapter, frankenphp_adapter, mailpit_adapter, php_adapter,
    redis_adapter,
};
