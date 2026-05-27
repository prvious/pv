pub mod error;
pub mod identity;
pub mod manifest;
pub mod platform;
pub mod registry;

pub use error::{ResourcesError, Result};
pub use identity::{
    ArtifactVersion, PublishedAt, PvVersion, ResourceName, Sha256Digest, TrackName, TrackSelector,
};
pub use manifest::{ArtifactManifest, ManifestArtifact, ManifestSelection, RevocationState};
pub use platform::{ArtifactPlatform, TargetPlatform};
pub use registry::{ResourceCapability, ResourceDescriptor, ResourceKind};
