use camino::{Utf8Path, Utf8PathBuf};

use crate::fs;
use crate::http::ResourceHttpClient;
use crate::{ArtifactManifest, ResourcesError, Result};

const MANIFEST_CACHE_FILE: &str = "manifest.json";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactManifestCache {
    cache_path: Utf8PathBuf,
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
        client: &impl ResourceHttpClient,
    ) -> Result<ArtifactManifest> {
        match client.get_text(manifest_url) {
            Ok(json) => {
                let manifest = ArtifactManifest::parse(&json)?;
                fs::write_string_atomically(&self.cache_path, &json)?;

                Ok(manifest)
            }
            Err(fetch_error) => {
                self.load_cached()
                    .map_err(|cache_error| ResourcesError::ManifestUnavailable {
                        url: manifest_url.to_string(),
                        cache_path: self.cache_path.to_string(),
                        reason: format!("{fetch_error}; cache fallback failed: {cache_error}"),
                    })
            }
        }
    }

    pub fn load_cached(&self) -> Result<ArtifactManifest> {
        let json = fs::read_to_string(&self.cache_path)?;

        ArtifactManifest::parse(&json)
    }

    pub fn path(&self) -> &Utf8Path {
        &self.cache_path
    }
}
