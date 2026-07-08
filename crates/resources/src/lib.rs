pub mod allocation;
pub mod cache;
pub mod command;
pub mod download;
pub mod endpoint;
pub mod error;
mod fs;
pub mod http;
pub mod identity;
pub mod install;
pub mod manifest;
pub mod php_defaults;
pub mod php_extensions;
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
    ManagedResourceTrack, ManagedResourceUninstallOptions, ManagedResourceUpdate,
    ManagedResourceUpdateBlocker, ManagedResourceUpdateCheck, ManagedResourceUpdateCheckTrack,
    ManagedResourceUpdateRevocation, ManagedResourceUpdateStatus, PhpPairInstall,
    PhpPairRemovalIntent, PhpPairUpdate,
};
pub use download::{
    ArtifactDownload, ArtifactDownloader, DownloadProgress, DownloadProgressEvent,
    NoDownloadProgress,
};
pub use endpoint::{
    ARTIFACT_MANIFEST_URL_BUILD_ENV, STABLE_ARTIFACT_MANIFEST_URL, default_artifact_manifest_url,
};
pub use error::{ResourcesError, Result};
pub use http::{ResourceHttpClient, UreqResourceHttpClient};
pub use identity::{
    ArtifactVersion, ConcreteTrackName, PublishedAt, PvVersion, ResourceName, Sha256Digest,
    TrackName, TrackSelector,
};
pub use install::{ArtifactInstall, ArtifactInstaller, ResourceAdapter};
pub use manifest::{ArtifactManifest, ManifestArtifact, ManifestSelection, RevocationState};
pub use php_defaults::{
    PHP_TRACK_DEFAULT_INI, PhpTrackDefaults, ensure_php_track_defaults, php_track_defaults,
    php_track_environment, php_track_exec_environment,
};
pub use php_extensions::{
    PHP_EXTENSION_METADATA_PATH, PhpExtensionLoadKind, PhpExtensionModule, PhpExtensionResolution,
    ensure_php_runtime_overlay, php_runtime_environment, php_runtime_exec_environment,
    read_php_extension_metadata, resolve_persisted_php_extension_modules,
    resolve_php_extension_request,
};
pub use platform::{ArtifactPlatform, TargetPlatform};
pub use registry::{ResourceCapability, ResourceDescriptor, ResourceKind};
pub use runtime::{
    RuntimeArtifactAdapter, composer_adapter, frankenphp_adapter, mailpit_adapter, mysql_adapter,
    php_adapter, postgres_adapter, redis_adapter, rustfs_adapter,
};
