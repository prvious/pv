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

pub use cache::{ArtifactManifestCache, ArtifactManifestRefresh, ArtifactManifestSource};
pub use command::{
    ManagedResourceCommandError, ManagedResourceCommandResult, ManagedResourceCommands,
    ManagedResourceInstall, ManagedResourceUninstall, ManagedResourceUpdate,
};
pub use download::{ArtifactDownload, ArtifactDownloader};
pub use error::{ResourcesError, Result};
pub use http::{ResourceHttpClient, UreqResourceHttpClient};
pub use identity::{
    ArtifactVersion, PublishedAt, PvVersion, ResourceName, Sha256Digest, TrackName, TrackSelector,
};
pub use install::{ArtifactInstall, ArtifactInstaller, ResourceAdapter};
pub use manifest::{ArtifactManifest, ManifestArtifact, ManifestSelection, RevocationState};
pub use platform::{ArtifactPlatform, TargetPlatform};
pub use registry::{ResourceCapability, ResourceDescriptor, ResourceKind};
