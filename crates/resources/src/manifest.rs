use crate::error::{ResourcesError, Result};
use crate::identity::{
    ArtifactVersion, PublishedAt, PvVersion, ResourceName, Sha256Digest, TrackName, TrackSelector,
};
use crate::platform::{ArtifactPlatform, TargetPlatform};
use crate::registry;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use url::Url;

#[derive(Debug)]
pub struct ArtifactManifest {
    schema_version: ManifestSchemaVersion,
    minimum_pv_version: PvVersion,
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
    resource_name: ResourceName,
    track: TrackName,
    artifact_version: ArtifactVersion,
    upstream_version: String,
    pv_build_revision: String,
    platform: ArtifactPlatform,
    url: String,
    sha256: Sha256Digest,
    size: u64,
    published_at: PublishedAt,
    revocation_state: RevocationState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevocationState {
    Active,
    Revoked { reason: String },
}

#[derive(Debug, Clone, Copy)]
pub enum ManifestSelection<'a> {
    Latest(&'a ManifestArtifact),
    RevokedFallback {
        artifact: &'a ManifestArtifact,
        revoked_latest: &'a ManifestArtifact,
    },
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
        resource: &ResourceName,
        track: &TrackName,
        target: TargetPlatform,
    ) -> Result<ManifestSelection<'_>> {
        let resource = self.resource(resource)?;
        let track = resource.track(track.as_str())?;

        track.select_latest(resource.name.as_str(), target)
    }

    pub fn select_artifact(
        &self,
        resource: &ResourceName,
        track: &TrackName,
        artifact_version: &ArtifactVersion,
        target: TargetPlatform,
    ) -> Result<Option<&ManifestArtifact>> {
        let resource = self.resource(resource)?;
        let track = resource.track(track.as_str())?;

        Ok(track
            .platform_selected_candidates(target)
            .into_iter()
            .find(|artifact| artifact.artifact_version() == artifact_version))
    }

    pub fn resolve_track(
        &self,
        resource: &ResourceName,
        selector: TrackSelector,
    ) -> Result<&TrackName> {
        let resource = self.resource(resource)?;

        match selector {
            TrackSelector::Latest => Ok(&resource.default_track),
            TrackSelector::Track(track) => resource.track(track.as_str()).map(|track| &track.name),
        }
    }

    pub fn schema_version(&self) -> u64 {
        self.schema_version.as_u64()
    }

    pub fn minimum_pv_version(&self) -> &PvVersion {
        &self.minimum_pv_version
    }

    pub fn resource_tracks(&self) -> impl Iterator<Item = (&ResourceName, &TrackName)> + '_ {
        self.resources.iter().flat_map(|resource| {
            resource
                .tracks
                .iter()
                .map(|track| (&resource.name, &track.name))
        })
    }

    fn from_raw(raw: RawManifest) -> Result<Self> {
        let schema_version = ManifestSchemaVersion::parse(raw.schema_version)?;
        let minimum_pv_version = PvVersion::parse(raw.minimum_pv_version)?;
        let current_pv_version = PvVersion::current()?;

        if minimum_pv_version > current_pv_version {
            return Err(ResourcesError::RequiresNewerPv {
                minimum_pv_version: minimum_pv_version.as_str().to_string(),
                current_pv_version: current_pv_version.as_str().to_string(),
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
            schema_version,
            minimum_pv_version,
            resources,
        })
    }

    fn resource(&self, resource: &ResourceName) -> Result<&ManifestResource> {
        if let Some(resource) = self
            .resources
            .iter()
            .find(|candidate| candidate.name == *resource)
        {
            return Ok(resource);
        }

        registry::resolve_canonical(resource.as_str())?;
        Err(ResourcesError::ResourceNotInManifest {
            resource: resource.as_str().to_string(),
        })
    }
}

impl ManifestResource {
    fn from_raw(raw: RawResource) -> Result<Self> {
        let name = ResourceName::new(raw.name.clone())?;
        reject_reserved_track_name(&raw.default_track)?;
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

                ManifestTrack::from_raw(track, &name)
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
            .ok_or_else(|| ResourcesError::TrackNotFound {
                resource: self.name.as_str().to_string(),
                track: track.to_string(),
            })
    }
}

impl ManifestTrack {
    fn from_raw(raw: RawTrack, resource: &ResourceName) -> Result<Self> {
        reject_reserved_track_name(&raw.name)?;
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
                            "duplicate artifact identity in resource `{}` track `{}`",
                            resource.as_str(),
                            raw.name
                        ),
                    });
                }

                ManifestArtifact::from_raw(artifact, resource, &name)
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

        let track = Self { name, artifacts };
        track.validate_platform_specificity(resource.as_str())?;
        track.validate_unique_published_at_slots(resource.as_str())?;

        Ok(track)
    }

    fn select_latest(
        &self,
        resource: &str,
        target: TargetPlatform,
    ) -> Result<ManifestSelection<'_>> {
        let candidates = self.platform_selected_candidates(target);
        let installable = self
            .best_candidate(
                resource,
                target,
                candidates
                    .iter()
                    .copied()
                    .filter(|artifact| !artifact.revocation_state.is_revoked()),
            )?
            .ok_or_else(|| ResourcesError::NoInstallableArtifact {
                resource: resource.to_string(),
                track: self.name.as_str().to_string(),
                platform: target.as_str().to_string(),
            })?;
        let latest = self.best_candidate(resource, target, candidates.iter().copied())?;

        if let Some(revoked_latest) = latest
            && revoked_latest.revocation_state.is_revoked()
            && revoked_latest.published_at > installable.published_at
        {
            Ok(ManifestSelection::RevokedFallback {
                artifact: installable,
                revoked_latest,
            })
        } else {
            Ok(ManifestSelection::Latest(installable))
        }
    }

    fn best_candidate<'a>(
        &self,
        resource: &str,
        target: TargetPlatform,
        candidates: impl IntoIterator<Item = &'a ManifestArtifact>,
    ) -> Result<Option<&'a ManifestArtifact>> {
        let mut best: Option<&'a ManifestArtifact> = None;

        for artifact in candidates {
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

    fn platform_selected_candidates(&self, target: TargetPlatform) -> Vec<&ManifestArtifact> {
        let mut candidates = BTreeMap::new();

        for artifact in self
            .artifacts
            .iter()
            .filter(|artifact| artifact.matches(target))
        {
            candidates
                .entry(artifact.artifact_version.clone())
                .and_modify(|candidate: &mut &ManifestArtifact| {
                    if artifact.platform.is_exact() && !candidate.platform.is_exact() {
                        *candidate = artifact;
                    }
                })
                .or_insert(artifact);
        }

        candidates.into_values().collect()
    }

    fn validate_platform_specificity(&self, resource: &str) -> Result<()> {
        let mut seen = BTreeMap::new();
        for artifact in &self.artifacts {
            let has_any_and_exact = seen
                .entry(artifact.artifact_version.clone())
                .and_modify(|(has_any, has_exact)| {
                    if artifact.platform == ArtifactPlatform::Any {
                        *has_any = true;
                    } else {
                        *has_exact = true;
                    }
                })
                .or_insert_with(|| {
                    (
                        artifact.platform == ArtifactPlatform::Any,
                        artifact.platform.is_exact(),
                    )
                });

            if has_any_and_exact.0 && has_any_and_exact.1 {
                return Err(ResourcesError::InvalidManifest {
                    reason: format!(
                        "artifact version `{}` in resource `{resource}` track `{}` mixes `any` with exact platforms",
                        artifact.artifact_version, self.name
                    ),
                });
            }
        }

        Ok(())
    }

    fn validate_unique_published_at_slots(&self, resource: &str) -> Result<()> {
        for target in [TargetPlatform::DarwinArm64, TargetPlatform::DarwinAmd64] {
            let mut seen = BTreeSet::new();
            for artifact in self.platform_selected_candidates(target) {
                if !seen.insert(artifact.published_at.clone()) {
                    return Err(ResourcesError::InvalidManifest {
                        reason: format!(
                            "duplicate published_at candidate in resource `{resource}` track `{}`",
                            self.name
                        ),
                    });
                }
            }
        }

        Ok(())
    }
}

impl ManifestArtifact {
    fn from_raw(raw: RawArtifact, resource_name: &ResourceName, track: &TrackName) -> Result<Self> {
        Ok(Self {
            resource_name: resource_name.clone(),
            track: track.clone(),
            artifact_version: ArtifactVersion::new(raw.artifact_version)?,
            upstream_version: raw.upstream_version,
            pv_build_revision: raw.pv_build_revision,
            platform: ArtifactPlatform::new(&raw.platform)?,
            url: validate_artifact_url(raw.url)?,
            sha256: Sha256Digest::new(raw.sha256)?,
            size: raw.size,
            published_at: PublishedAt::parse(raw.published_at)?,
            revocation_state: RevocationState::from_raw(raw.revoked, raw.revocation_reason)?,
        })
    }

    pub fn resource_name(&self) -> &ResourceName {
        &self.resource_name
    }

    pub fn track(&self) -> &TrackName {
        &self.track
    }

    pub fn artifact_version(&self) -> &ArtifactVersion {
        &self.artifact_version
    }

    pub fn upstream_version(&self) -> &str {
        &self.upstream_version
    }

    pub fn pv_build_revision(&self) -> &str {
        &self.pv_build_revision
    }

    pub fn platform(&self) -> ArtifactPlatform {
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

    pub fn published_at(&self) -> &PublishedAt {
        &self.published_at
    }

    pub fn revocation_state(&self) -> &RevocationState {
        &self.revocation_state
    }

    fn matches(&self, target: TargetPlatform) -> bool {
        self.platform.matches(target)
    }
}

fn validate_artifact_url(url: String) -> Result<String> {
    if url.contains('\\') {
        return Err(ResourcesError::InvalidArtifactUrl { url });
    }

    let parsed = match Url::parse(&url) {
        Ok(parsed) => parsed,
        Err(_error) => return Err(ResourcesError::InvalidArtifactUrl { url }),
    };
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(ResourcesError::InvalidArtifactUrl { url });
    }

    let Some(file_name) = parsed
        .path_segments()
        .and_then(|mut segments| segments.next_back())
    else {
        return Err(ResourcesError::InvalidArtifactUrl { url });
    };
    if file_name.is_empty() || file_name == "." || file_name == ".." || file_name.contains('\\') {
        return Err(ResourcesError::InvalidArtifactUrl { url });
    }

    Ok(url)
}

impl RevocationState {
    fn from_raw(revoked: bool, reason: Option<String>) -> Result<Self> {
        match (revoked, reason) {
            (false, None) => Ok(Self::Active),
            (false, Some(_reason)) => Err(ResourcesError::InvalidRevocationState {
                reason: "revocation_reason requires revoked=true",
            }),
            (true, Some(reason)) if !reason.trim().is_empty() => Ok(Self::Revoked { reason }),
            (true, _) => Err(ResourcesError::InvalidRevocationState {
                reason: "revoked artifacts require a non-empty revocation_reason",
            }),
        }
    }

    pub fn is_revoked(&self) -> bool {
        matches!(self, Self::Revoked { .. })
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Active => None,
            Self::Revoked { reason } => Some(reason),
        }
    }
}

impl<'a> ManifestSelection<'a> {
    pub fn artifact(self) -> &'a ManifestArtifact {
        match self {
            Self::Latest(artifact) | Self::RevokedFallback { artifact, .. } => artifact,
        }
    }

    pub fn revoked_latest(self) -> Option<&'a ManifestArtifact> {
        match self {
            Self::Latest(_) => None,
            Self::RevokedFallback { revoked_latest, .. } => Some(revoked_latest),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManifestSchemaVersion {
    V1,
}

impl ManifestSchemaVersion {
    const SUPPORTED: Self = Self::V1;

    fn parse(value: u64) -> Result<Self> {
        match value {
            1 => Ok(Self::V1),
            _ => Err(ResourcesError::UnsupportedManifestSchema {
                schema_version: value,
                supported_schema_version: Self::SUPPORTED.as_u64(),
            }),
        }
    }

    fn as_u64(self) -> u64 {
        match self {
            Self::V1 => 1,
        }
    }
}

fn reject_reserved_track_name(name: &str) -> Result<()> {
    if TrackSelector::is_reserved_alias(name) {
        Err(ResourcesError::ReservedTrackName {
            name: name.to_string(),
        })
    } else {
        Ok(())
    }
}
