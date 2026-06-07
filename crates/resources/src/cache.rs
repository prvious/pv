use camino::{Utf8Path, Utf8PathBuf};
use url::Url;

use crate::fs;
use crate::http::ResourceHttpClient;
use crate::{ArtifactManifest, ResourcesError, Result};

const MANIFEST_CACHE_FILE: &str = "manifest.json";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactManifestCache {
    cache_path: Utf8PathBuf,
}

#[derive(Debug)]
pub struct ArtifactManifestRefresh {
    manifest: ArtifactManifest,
    source: ArtifactManifestSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactManifestSource {
    Latest,
    Cached { reason: String },
}

impl ArtifactManifestRefresh {
    fn latest(manifest: ArtifactManifest) -> Self {
        Self {
            manifest,
            source: ArtifactManifestSource::Latest,
        }
    }

    fn cached(manifest: ArtifactManifest, reason: String) -> Self {
        Self {
            manifest,
            source: ArtifactManifestSource::Cached { reason },
        }
    }

    pub fn manifest(&self) -> &ArtifactManifest {
        &self.manifest
    }

    pub fn into_manifest(self) -> ArtifactManifest {
        self.manifest
    }

    pub fn source(&self) -> &ArtifactManifestSource {
        &self.source
    }

    pub fn is_from_cache(&self) -> bool {
        matches!(self.source, ArtifactManifestSource::Cached { .. })
    }
}

impl ArtifactManifestCache {
    pub fn new(downloads_dir: impl Into<Utf8PathBuf>) -> Self {
        Self {
            cache_path: downloads_dir.into().join(MANIFEST_CACHE_FILE),
        }
    }

    pub fn refresh(
        &self,
        manifest_url: &str,
        client: &(impl ResourceHttpClient + ?Sized),
    ) -> Result<ArtifactManifestRefresh> {
        validate_manifest_url(manifest_url)?;

        let json = match client.get_text(manifest_url) {
            Ok(json) => json,
            Err(fetch_error @ ResourcesError::HttpRequestFailed { .. }) => {
                let fallback_reason = fetch_error.to_string();
                return self
                    .load_cached()
                    .map_err(|cache_error| ResourcesError::ManifestUnavailable {
                        url: manifest_url.to_string(),
                        cache_path: self.cache_path.to_string(),
                        reason: format!("{fetch_error}; cache fallback failed: {cache_error}"),
                    })
                    .map(|manifest| ArtifactManifestRefresh::cached(manifest, fallback_reason));
            }
            Err(fetch_error) => return Err(fetch_error),
        };
        let manifest = ArtifactManifest::parse(&json)?;
        fs::write_string_atomically(&self.cache_path, &json)?;

        Ok(ArtifactManifestRefresh::latest(manifest))
    }

    pub fn refresh_latest(
        &self,
        manifest_url: &str,
        client: &(impl ResourceHttpClient + ?Sized),
    ) -> Result<ArtifactManifestRefresh> {
        validate_manifest_url(manifest_url)?;

        let json = client.get_text(manifest_url)?;
        let manifest = ArtifactManifest::parse(&json)?;
        fs::write_string_atomically(&self.cache_path, &json)?;

        Ok(ArtifactManifestRefresh::latest(manifest))
    }

    pub fn load_cached(&self) -> Result<ArtifactManifest> {
        let json = fs::read_to_string(&self.cache_path)?;

        ArtifactManifest::parse(&json)
    }

    pub fn path(&self) -> &Utf8Path {
        &self.cache_path
    }
}

fn validate_manifest_url(url: &str) -> Result<()> {
    if url.contains('\\') {
        return Err(ResourcesError::InvalidManifestUrl {
            url: url.to_string(),
        });
    }

    let parsed = match Url::parse(url) {
        Ok(parsed) => parsed,
        Err(_error) => {
            return Err(ResourcesError::InvalidManifestUrl {
                url: url.to_string(),
            });
        }
    };

    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(ResourcesError::InvalidManifestUrl {
            url: url.to_string(),
        });
    }

    Ok(())
}
