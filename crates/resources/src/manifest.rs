use crate::error::{ResourcesError, Result};
use crate::identity::{ArtifactVersion, PublishedAt, ResourceName, Sha256Digest, TrackName};
use crate::platform::{ArtifactPlatform, TargetPlatform};
use crate::registry;
use serde::Deserialize;
use std::collections::BTreeSet;

#[derive(Debug)]
pub struct ArtifactManifest {
    schema_version: u64,
    minimum_pv_version: String,
    resources: Vec<ManifestResource>,
}

#[derive(Debug)]
struct ManifestResource {
    name: ResourceName,
    default_track: TrackName,
    tracks: Vec<ManifestTrack>,
}

#[derive(Debug)]
struct ManifestTrack {
    name: TrackName,
    artifacts: Vec<ManifestArtifact>,
}

#[derive(Debug, Clone)]
pub struct ManifestArtifact {
    artifact_version: ArtifactVersion,
    upstream_version: String,
    pv_build_revision: String,
    platform: ArtifactPlatform,
    url: String,
    sha256: Sha256Digest,
    size: u64,
    published_at: PublishedAt,
    revoked: bool,
    revocation_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawManifest {
    schema_version: u64,
    minimum_pv_version: String,
    resources: Vec<RawResource>,
}

#[derive(Debug, Deserialize)]
struct RawResource {
    name: String,
    default_track: String,
    tracks: Vec<RawTrack>,
}

#[derive(Debug, Deserialize)]
struct RawTrack {
    name: String,
    artifacts: Vec<RawArtifact>,
}

#[derive(Debug, Deserialize)]
struct RawArtifact {
    artifact_version: String,
    upstream_version: String,
    pv_build_revision: String,
    platform: String,
    url: String,
    sha256: String,
    size: u64,
    published_at: String,
    #[serde(default)]
    revoked: bool,
    #[serde(default)]
    revocation_reason: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ManifestSummary {
    pub schema_version: u64,
    pub minimum_pv_version: String,
    pub resources: Vec<ResourceSummary>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ResourceSummary {
    pub name: String,
    pub default_track: String,
    pub tracks: Vec<TrackSummary>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct TrackSummary {
    pub name: String,
    pub artifacts: Vec<ArtifactSummary>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ArtifactSummary {
    pub artifact_version: String,
    pub upstream_version: String,
    pub pv_build_revision: String,
    pub platform: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    pub published_at: String,
    pub revoked: bool,
    pub revocation_reason: Option<String>,
}

impl ArtifactManifest {
    pub fn parse(json: &str) -> Result<Self> {
        let raw: RawManifest =
            serde_json::from_str(json).map_err(|error| ResourcesError::InvalidManifest {
                reason: error.to_string(),
            })?;

        Self::from_raw(raw)
    }

    pub fn select_latest(
        &self,
        resource: &str,
        track: &str,
        target: TargetPlatform,
    ) -> Result<&ManifestArtifact> {
        let resource = self.resource(resource)?;
        let track = resource.track(track)?;

        track.select_latest(resource.name.as_str(), target)
    }

    pub fn resolve_track(&self, resource: &str, track_or_latest: &str) -> Result<&TrackName> {
        let resource = self.resource(resource)?;

        if track_or_latest == "latest" {
            Ok(&resource.default_track)
        } else {
            resource.track(track_or_latest).map(|track| &track.name)
        }
    }

    pub fn summary(&self) -> ManifestSummary {
        ManifestSummary {
            schema_version: self.schema_version,
            minimum_pv_version: self.minimum_pv_version.clone(),
            resources: self
                .resources
                .iter()
                .map(ManifestResource::summary)
                .collect(),
        }
    }

    fn from_raw(raw: RawManifest) -> Result<Self> {
        if raw.schema_version == 0 {
            return Err(ResourcesError::InvalidManifest {
                reason: "schema_version must be greater than zero".to_string(),
            });
        }

        let mut seen_resources = BTreeSet::new();
        let resources = raw
            .resources
            .into_iter()
            .map(|resource| {
                let descriptor = registry::resolve(&resource.name)?;
                if descriptor.is_alias(&resource.name) {
                    return Err(ResourcesError::ManifestUsesAlias {
                        alias: resource.name,
                        canonical: descriptor.name(),
                    });
                }

                if !seen_resources.insert(resource.name.clone()) {
                    return Err(ResourcesError::InvalidManifest {
                        reason: format!("duplicate resource `{}`", resource.name),
                    });
                }

                ManifestResource::from_raw(resource)
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            schema_version: raw.schema_version,
            minimum_pv_version: raw.minimum_pv_version,
            resources,
        })
    }

    fn resource(&self, resource: &str) -> Result<&ManifestResource> {
        self.resources
            .iter()
            .find(|candidate| candidate.name.as_str() == resource)
            .ok_or_else(|| ResourcesError::UnknownResource {
                name: resource.to_string(),
            })
    }
}

impl ManifestResource {
    fn from_raw(raw: RawResource) -> Result<Self> {
        let name = ResourceName::new(raw.name.clone())?;
        let default_track = TrackName::new(raw.default_track.clone())?;
        let mut seen_tracks = BTreeSet::new();
        let tracks = raw
            .tracks
            .into_iter()
            .map(|track| {
                if !seen_tracks.insert(track.name.clone()) {
                    return Err(ResourcesError::InvalidManifest {
                        reason: format!(
                            "duplicate track `{}` for resource `{}`",
                            track.name, raw.name
                        ),
                    });
                }

                ManifestTrack::from_raw(track, &raw.name)
            })
            .collect::<Result<Vec<_>>>()?;

        if !tracks.iter().any(|track| track.name == default_track) {
            return Err(ResourcesError::InvalidManifest {
                reason: format!(
                    "default track `{default_track}` does not exist for resource `{name}`"
                ),
            });
        }

        Ok(Self {
            name,
            default_track,
            tracks,
        })
    }

    fn track(&self, track: &str) -> Result<&ManifestTrack> {
        self.tracks
            .iter()
            .find(|candidate| candidate.name.as_str() == track)
            .ok_or_else(|| ResourcesError::InvalidManifest {
                reason: format!("resource `{}` has no track `{track}`", self.name),
            })
    }

    fn summary(&self) -> ResourceSummary {
        ResourceSummary {
            name: self.name.as_str().to_string(),
            default_track: self.default_track.as_str().to_string(),
            tracks: self.tracks.iter().map(ManifestTrack::summary).collect(),
        }
    }
}

impl ManifestTrack {
    fn from_raw(raw: RawTrack, resource: &str) -> Result<Self> {
        let name = TrackName::new(raw.name.clone())?;
        let mut seen_artifacts = BTreeSet::new();
        let artifacts = raw
            .artifacts
            .into_iter()
            .map(|artifact| {
                let identity = (artifact.artifact_version.clone(), artifact.platform.clone());
                if !seen_artifacts.insert(identity) {
                    return Err(ResourcesError::InvalidManifest {
                        reason: format!(
                            "duplicate artifact identity in resource `{resource}` track `{}`",
                            raw.name
                        ),
                    });
                }

                ManifestArtifact::from_raw(artifact)
            })
            .collect::<Result<Vec<_>>>()?;

        if artifacts.is_empty() {
            return Err(ResourcesError::InvalidManifest {
                reason: format!(
                    "resource `{resource}` track `{}` has no artifacts",
                    raw.name
                ),
            });
        }

        Ok(Self { name, artifacts })
    }

    fn select_latest(&self, resource: &str, target: TargetPlatform) -> Result<&ManifestArtifact> {
        if let Some(candidate) = self.best_candidate(resource, target, PlatformMatch::Exact)? {
            return Ok(candidate);
        }

        self.best_candidate(resource, target, PlatformMatch::Any)?
            .ok_or_else(|| ResourcesError::NoInstallableArtifact {
                resource: resource.to_string(),
                track: self.name.as_str().to_string(),
                platform: target.as_str().to_string(),
            })
    }

    fn best_candidate(
        &self,
        resource: &str,
        target: TargetPlatform,
        platform_match: PlatformMatch,
    ) -> Result<Option<&ManifestArtifact>> {
        let mut best: Option<&ManifestArtifact> = None;

        for artifact in self
            .artifacts
            .iter()
            .filter(|artifact| artifact.matches(target, platform_match))
        {
            match best {
                Some(current) if artifact.published_at == current.published_at => {
                    return Err(ResourcesError::AmbiguousArtifactSelection {
                        resource: resource.to_string(),
                        track: self.name.as_str().to_string(),
                        platform: target.as_str().to_string(),
                    });
                }
                Some(current) if artifact.published_at <= current.published_at => {}
                _ => best = Some(artifact),
            }
        }

        Ok(best)
    }

    fn summary(&self) -> TrackSummary {
        TrackSummary {
            name: self.name.as_str().to_string(),
            artifacts: self
                .artifacts
                .iter()
                .map(ManifestArtifact::summary)
                .collect(),
        }
    }
}

impl ManifestArtifact {
    fn from_raw(raw: RawArtifact) -> Result<Self> {
        Ok(Self {
            artifact_version: ArtifactVersion::new(raw.artifact_version)?,
            upstream_version: raw.upstream_version,
            pv_build_revision: raw.pv_build_revision,
            platform: ArtifactPlatform::new(&raw.platform)?,
            url: raw.url,
            sha256: Sha256Digest::new(raw.sha256)?,
            size: raw.size,
            published_at: PublishedAt::parse(raw.published_at)?,
            revoked: raw.revoked,
            revocation_reason: raw.revocation_reason,
        })
    }

    pub fn artifact_version(&self) -> &ArtifactVersion {
        &self.artifact_version
    }

    fn matches(&self, target: TargetPlatform, platform_match: PlatformMatch) -> bool {
        !self.revoked
            && self.platform.matches(target)
            && match platform_match {
                PlatformMatch::Exact => self.platform.is_exact(),
                PlatformMatch::Any => self.platform == ArtifactPlatform::Any,
            }
    }

    fn summary(&self) -> ArtifactSummary {
        ArtifactSummary {
            artifact_version: self.artifact_version.as_str().to_string(),
            upstream_version: self.upstream_version.clone(),
            pv_build_revision: self.pv_build_revision.clone(),
            platform: self.platform.as_str().to_string(),
            url: self.url.clone(),
            sha256: self.sha256.as_str().to_string(),
            size: self.size,
            published_at: self.published_at.as_rfc3339(),
            revoked: self.revoked,
            revocation_reason: self.revocation_reason.clone(),
        }
    }
}

#[derive(Clone, Copy)]
enum PlatformMatch {
    Exact,
    Any,
}
