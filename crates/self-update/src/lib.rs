use std::collections::BTreeSet;
use std::fmt;

use serde::Deserialize;
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use url::Url;

const SUPPORTED_SCHEMA_VERSION: u64 = 1;
const STABLE_CHANNEL: &str = "stable";

#[derive(Debug)]
pub struct AppUpdateManifest {
    schema_version: u64,
    channel: String,
    version: AppUpdateVersion,
    minimum_pv_version: AppUpdateVersion,
    published_at: AppUpdatePublishedAt,
    assets: Vec<AppUpdateAsset>,
}

#[derive(Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub struct AppUpdateVersion {
    major: u64,
    minor: u64,
    patch: u64,
    raw: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Sha256Digest(String);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppUpdatePublishedAt(String);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppUpdateAsset {
    platform: AppUpdatePlatform,
    url: String,
    sha256: Sha256Digest,
    size: u64,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum AppUpdatePlatform {
    DarwinArm64,
    DarwinAmd64,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum AppUpdateManifestError {
    #[error("invalid PV app update manifest: {reason}")]
    InvalidManifest { reason: String },

    #[error(
        "unsupported PV app update manifest schema version {schema_version}, expected {supported_schema_version}"
    )]
    UnsupportedManifestSchema {
        schema_version: u64,
        supported_schema_version: u64,
    },

    #[error("unsupported PV app update channel `{channel}`")]
    UnsupportedChannel { channel: String },

    #[error("invalid PV app update {kind} `{value}`")]
    InvalidIdentity { kind: &'static str, value: String },

    #[error(
        "PV app update manifest requires PV {minimum_pv_version}, current PV is {current_pv_version}"
    )]
    RequiresNewerPv {
        minimum_pv_version: String,
        current_pv_version: String,
    },

    #[error("failed to parse PV app update published_at `{value}`")]
    InvalidPublishedAt { value: String },

    #[error("unsupported PV app update platform `{platform}`")]
    UnsupportedPlatform { platform: String },

    #[error("invalid PV app update asset URL `{url}`")]
    InvalidAssetUrl { url: String },

    #[error("invalid PV app update asset size {size} for {platform}")]
    InvalidAssetSize { platform: String, size: u64 },

    #[error("duplicate PV app update asset for {platform}")]
    DuplicatePlatform { platform: String },

    #[error("PV app update manifest has no assets")]
    NoAssets,

    #[error("PV app update manifest has no asset for {platform}")]
    MissingPlatform { platform: String },

    #[error("current PV app update platform is unsupported on this host")]
    UnsupportedCurrentPlatform,
}

pub type Result<T> = std::result::Result<T, AppUpdateManifestError>;

#[derive(Debug, Deserialize)]
struct RawManifest {
    schema_version: u64,
    channel: String,
    version: String,
    minimum_pv_version: String,
    published_at: String,
    assets: Vec<RawAsset>,
}

#[derive(Debug, Deserialize)]
struct RawAsset {
    platform: String,
    url: String,
    sha256: String,
    size: u64,
}

impl AppUpdateManifest {
    pub fn parse(json: &str) -> Result<Self> {
        let raw: RawManifest = serde_json::from_str(json).map_err(|error| {
            AppUpdateManifestError::InvalidManifest {
                reason: error.to_string(),
            }
        })?;

        Self::from_raw(raw)
    }

    pub fn select_platform(&self, platform: AppUpdatePlatform) -> Result<&AppUpdateAsset> {
        self.assets
            .iter()
            .find(|asset| asset.platform == platform)
            .ok_or_else(|| AppUpdateManifestError::MissingPlatform {
                platform: platform.as_str().to_string(),
            })
    }

    pub fn select_current_platform(&self) -> Result<&AppUpdateAsset> {
        self.select_platform(AppUpdatePlatform::current()?)
    }

    pub fn schema_version(&self) -> u64 {
        self.schema_version
    }

    pub fn channel(&self) -> &str {
        &self.channel
    }

    pub fn version(&self) -> &AppUpdateVersion {
        &self.version
    }

    pub fn minimum_pv_version(&self) -> &AppUpdateVersion {
        &self.minimum_pv_version
    }

    pub fn published_at(&self) -> &AppUpdatePublishedAt {
        &self.published_at
    }

    pub fn assets(&self) -> &[AppUpdateAsset] {
        &self.assets
    }

    fn from_raw(raw: RawManifest) -> Result<Self> {
        if raw.schema_version != SUPPORTED_SCHEMA_VERSION {
            return Err(AppUpdateManifestError::UnsupportedManifestSchema {
                schema_version: raw.schema_version,
                supported_schema_version: SUPPORTED_SCHEMA_VERSION,
            });
        }

        if raw.channel != STABLE_CHANNEL {
            return Err(AppUpdateManifestError::UnsupportedChannel {
                channel: raw.channel,
            });
        }

        let version = AppUpdateVersion::parse(raw.version)?;
        let minimum_pv_version = AppUpdateVersion::parse(raw.minimum_pv_version)?;
        let current_pv_version = AppUpdateVersion::current()?;
        if minimum_pv_version > current_pv_version {
            return Err(AppUpdateManifestError::RequiresNewerPv {
                minimum_pv_version: minimum_pv_version.as_str().to_string(),
                current_pv_version: current_pv_version.as_str().to_string(),
            });
        }

        let published_at = AppUpdatePublishedAt::parse(raw.published_at)?;
        let assets = parse_assets(raw.assets)?;

        Ok(Self {
            schema_version: raw.schema_version,
            channel: raw.channel,
            version,
            minimum_pv_version,
            published_at,
            assets,
        })
    }
}

impl AppUpdateVersion {
    pub fn current() -> Result<Self> {
        Self::parse(env!("CARGO_PKG_VERSION"))
    }

    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.contains('/') || value.contains('\\') {
            return invalid_identity("version", value);
        }

        let mut parts = value.split('.');
        let Some(major) = parts.next() else {
            return invalid_identity("version", value);
        };
        let Some(minor) = parts.next() else {
            return invalid_identity("version", value);
        };
        let Some(patch) = parts.next() else {
            return invalid_identity("version", value);
        };
        if parts.next().is_some() {
            return invalid_identity("version", value);
        }

        let major = parse_version_component(major, &value)?;
        let minor = parse_version_component(minor, &value)?;
        let patch = parse_version_component(patch, &value)?;

        Ok(Self {
            major,
            minor,
            patch,
            raw: value,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.raw
    }
}

impl fmt::Display for AppUpdateVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Sha256Digest {
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            Ok(Self(value.to_ascii_lowercase()))
        } else {
            invalid_identity("sha256", value)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AppUpdatePublishedAt {
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        OffsetDateTime::parse(&value, &Rfc3339).map_err(|_| {
            AppUpdateManifestError::InvalidPublishedAt {
                value: value.clone(),
            }
        })?;

        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AppUpdateAsset {
    pub fn platform(&self) -> AppUpdatePlatform {
        self.platform
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn sha256(&self) -> &Sha256Digest {
        &self.sha256
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    fn from_raw(raw: RawAsset) -> Result<Self> {
        let platform = AppUpdatePlatform::parse(&raw.platform)?;
        if raw.size == 0 {
            return Err(AppUpdateManifestError::InvalidAssetSize {
                platform: platform.as_str().to_string(),
                size: raw.size,
            });
        }

        Ok(Self {
            platform,
            url: validate_asset_url(raw.url)?,
            sha256: Sha256Digest::parse(raw.sha256)?,
            size: raw.size,
        })
    }
}

impl AppUpdatePlatform {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "darwin-arm64" => Ok(Self::DarwinArm64),
            "darwin-amd64" => Ok(Self::DarwinAmd64),
            _ => Err(AppUpdateManifestError::UnsupportedPlatform {
                platform: value.to_string(),
            }),
        }
    }

    pub fn current() -> Result<Self> {
        current_platform()
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::DarwinArm64 => "darwin-arm64",
            Self::DarwinAmd64 => "darwin-amd64",
        }
    }
}

impl fmt::Display for AppUpdatePlatform {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

fn parse_assets(raw_assets: Vec<RawAsset>) -> Result<Vec<AppUpdateAsset>> {
    if raw_assets.is_empty() {
        return Err(AppUpdateManifestError::NoAssets);
    }

    let mut seen_platforms = BTreeSet::new();
    raw_assets
        .into_iter()
        .map(|asset| {
            let platform = AppUpdatePlatform::parse(&asset.platform)?;
            if !seen_platforms.insert(platform) {
                return Err(AppUpdateManifestError::DuplicatePlatform {
                    platform: platform.as_str().to_string(),
                });
            }

            AppUpdateAsset::from_raw(asset)
        })
        .collect()
}

fn validate_asset_url(url: String) -> Result<String> {
    if url.contains('\\') {
        return Err(AppUpdateManifestError::InvalidAssetUrl { url });
    }

    let parsed = match Url::parse(&url) {
        Ok(parsed) => parsed,
        Err(_error) => return Err(AppUpdateManifestError::InvalidAssetUrl { url }),
    };
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(AppUpdateManifestError::InvalidAssetUrl { url });
    }

    let Some(file_name) = parsed
        .path_segments()
        .and_then(|mut segments| segments.next_back())
    else {
        return Err(AppUpdateManifestError::InvalidAssetUrl { url });
    };
    if file_name.is_empty() || file_name == "." || file_name == ".." || file_name.contains('\\') {
        return Err(AppUpdateManifestError::InvalidAssetUrl { url });
    }

    Ok(url)
}

fn parse_version_component(component: &str, value: &str) -> Result<u64> {
    if component.is_empty() || component.len() > 1 && component.starts_with('0') {
        return invalid_identity("version", value.to_string());
    }

    component
        .parse::<u64>()
        .map_err(|_| AppUpdateManifestError::InvalidIdentity {
            kind: "version",
            value: value.to_string(),
        })
}

fn invalid_identity<T>(kind: &'static str, value: String) -> Result<T> {
    Err(AppUpdateManifestError::InvalidIdentity { kind, value })
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn current_platform() -> Result<AppUpdatePlatform> {
    Ok(AppUpdatePlatform::DarwinArm64)
}

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
fn current_platform() -> Result<AppUpdatePlatform> {
    Ok(AppUpdatePlatform::DarwinAmd64)
}

#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "macos", target_arch = "x86_64")
)))]
fn current_platform() -> Result<AppUpdatePlatform> {
    Err(AppUpdateManifestError::UnsupportedCurrentPlatform)
}
