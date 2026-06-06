use camino::{Utf8Path, Utf8PathBuf};
use resources::{
    ArtifactPlatform, ArtifactVersion, PublishedAt, PvVersion, ResourceName, ResourcesError,
    Sha256Digest, TrackName,
};
use serde::Deserialize;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArtifactIdentity {
    resource: ResourceName,
    track: TrackName,
    upstream_version: String,
    pv_build_revision: String,
    platform: ArtifactPlatform,
}

#[derive(Clone, Debug)]
#[expect(
    dead_code,
    reason = "release records expose narrow getters only when later tasks need fields"
)]
pub struct ReleaseRecord {
    path: Utf8PathBuf,
    identity: ArtifactIdentity,
    artifact_version: ArtifactVersion,
    object_key: String,
    sha256: Sha256Digest,
    size: u64,
    published_at_raw: String,
    published_at: PublishedAt,
    minimum_pv_version: PvVersion,
    license_files: Vec<String>,
    notice_files: Vec<String>,
    provenance: Provenance,
}

#[derive(Clone, Debug)]
#[expect(
    dead_code,
    reason = "revocation records expose narrow getters only when later tasks need fields"
)]
pub struct RevocationRecord {
    path: Utf8PathBuf,
    resource: ResourceName,
    track: TrackName,
    artifact_version: ArtifactVersion,
    platform: ArtifactPlatform,
    reason: String,
    revoked_at: PublishedAt,
    replacement_artifact_version: Option<ArtifactVersion>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RevocationMetadata {
    reason: String,
    revoked_at: PublishedAt,
    replacement_artifact_version: Option<ArtifactVersion>,
}

#[derive(Clone, Debug, Deserialize)]
struct RawReleaseRecord {
    resource: String,
    track: String,
    upstream_version: String,
    pv_build_revision: String,
    artifact_version: String,
    platform: String,
    object_key: String,
    sha256: String,
    size: u64,
    published_at: String,
    minimum_pv_version: String,
    license_files: Vec<String>,
    #[serde(default)]
    notice_files: Vec<String>,
    provenance: Provenance,
}

#[derive(Clone, Debug, Deserialize)]
#[expect(
    dead_code,
    reason = "provenance metadata is retained for later manifest and diagnostics tasks"
)]
pub struct Provenance {
    source_url: String,
    source_sha256: String,
    recipe: String,
    pv_commit: String,
    build_run_id: String,
}

#[derive(Clone, Debug, Deserialize)]
struct RawRevocationRecord {
    resource: String,
    track: String,
    artifact_version: String,
    platform: String,
    reason: String,
    revoked_at: String,
    replacement_artifact_version: Option<String>,
}

impl ArtifactIdentity {
    pub fn new(
        resource: &str,
        track: &str,
        upstream_version: &str,
        pv_build_revision: &str,
        platform: &str,
    ) -> crate::Result<Self> {
        Ok(Self {
            resource: ResourceName::new(resource)
                .map_err(|error| invalid_release_identity("<identity>", "resource", error))?,
            track: TrackName::new(track)
                .map_err(|error| invalid_release_identity("<identity>", "track", error))?,
            upstream_version: require_non_empty_release(
                Utf8Path::new("<identity>"),
                "upstream_version",
                upstream_version,
            )?
            .to_string(),
            pv_build_revision: require_non_empty_release(
                Utf8Path::new("<identity>"),
                "pv_build_revision",
                pv_build_revision,
            )?
            .to_string(),
            platform: ArtifactPlatform::new(platform)
                .map_err(|error| invalid_release_identity("<identity>", "platform", error))?,
        })
    }

    pub fn manifest_key(&self) -> String {
        format!(
            "{}:{}:{}:{}:{}",
            self.resource, self.track, self.upstream_version, self.pv_build_revision, self.platform
        )
    }
}

impl ReleaseRecord {
    pub fn from_json(path: &Utf8Path, json: &str) -> crate::Result<Self> {
        let raw: RawReleaseRecord =
            serde_json::from_str(json).map_err(|error| invalid_release(path, error.to_string()))?;

        let expected_artifact_version =
            format!("{}-{}", raw.upstream_version, raw.pv_build_revision);
        if raw.artifact_version != expected_artifact_version {
            return Err(invalid_release(
                path,
                format!(
                    "artifact_version `{}` must equal `{expected_artifact_version}`",
                    raw.artifact_version
                ),
            ));
        }

        if raw.license_files.is_empty() {
            return Err(invalid_release(path, "license_files must not be empty"));
        }
        validate_relative_file_list(path, "license_files", &raw.license_files)?;
        validate_relative_file_list(path, "notice_files", &raw.notice_files)?;

        let identity = ArtifactIdentity {
            resource: ResourceName::new(raw.resource)
                .map_err(|error| invalid_release_identity(path, "resource", error))?,
            track: TrackName::new(raw.track)
                .map_err(|error| invalid_release_identity(path, "track", error))?,
            upstream_version: require_non_empty_release(
                path,
                "upstream_version",
                &raw.upstream_version,
            )?
            .to_string(),
            pv_build_revision: require_non_empty_release(
                path,
                "pv_build_revision",
                &raw.pv_build_revision,
            )?
            .to_string(),
            platform: ArtifactPlatform::new(&raw.platform)
                .map_err(|error| invalid_release_identity(path, "platform", error))?,
        };

        Ok(Self {
            path: path.to_path_buf(),
            identity,
            artifact_version: ArtifactVersion::new(raw.artifact_version)
                .map_err(|error| invalid_release_identity(path, "artifact_version", error))?,
            object_key: require_non_empty_release(path, "object_key", &raw.object_key)?.to_string(),
            sha256: Sha256Digest::new(raw.sha256)
                .map_err(|error| invalid_release_identity(path, "sha256", error))?,
            size: raw.size,
            published_at_raw: raw.published_at.clone(),
            published_at: PublishedAt::parse(raw.published_at)
                .map_err(|error| invalid_release_identity(path, "published_at", error))?,
            minimum_pv_version: PvVersion::parse(raw.minimum_pv_version)
                .map_err(|error| invalid_release_identity(path, "minimum_pv_version", error))?,
            license_files: raw.license_files,
            notice_files: raw.notice_files,
            provenance: raw.provenance,
        })
    }

    pub fn identity(&self) -> ArtifactIdentity {
        self.identity.clone()
    }
}

impl RevocationRecord {
    pub fn from_json(path: &Utf8Path, json: &str) -> crate::Result<Self> {
        let raw: RawRevocationRecord = serde_json::from_str(json)
            .map_err(|error| invalid_revocation(path, error.to_string()))?;
        let reason = require_non_empty_revocation(path, "reason", &raw.reason)?.to_string();

        Ok(Self {
            path: path.to_path_buf(),
            resource: ResourceName::new(raw.resource)
                .map_err(|error| invalid_revocation_identity(path, "resource", error))?,
            track: TrackName::new(raw.track)
                .map_err(|error| invalid_revocation_identity(path, "track", error))?,
            artifact_version: ArtifactVersion::new(raw.artifact_version)
                .map_err(|error| invalid_revocation_identity(path, "artifact_version", error))?,
            platform: ArtifactPlatform::new(&raw.platform)
                .map_err(|error| invalid_revocation_identity(path, "platform", error))?,
            reason,
            revoked_at: PublishedAt::parse(raw.revoked_at)
                .map_err(|error| invalid_revocation_identity(path, "revoked_at", error))?,
            replacement_artifact_version: raw
                .replacement_artifact_version
                .map(ArtifactVersion::new)
                .transpose()
                .map_err(|error| {
                    invalid_revocation_identity(path, "replacement_artifact_version", error)
                })?,
        })
    }

    fn target_key(&self) -> String {
        format!(
            "{}:{}:{}:{}",
            self.resource, self.track, self.artifact_version, self.platform
        )
    }

    fn revocation_metadata(&self) -> RevocationMetadata {
        RevocationMetadata {
            reason: self.reason.clone(),
            revoked_at: self.revoked_at.clone(),
            replacement_artifact_version: self.replacement_artifact_version.clone(),
        }
    }
}

pub fn load_release_records(root: &Utf8Path) -> crate::Result<Vec<ReleaseRecord>> {
    let mut records = Vec::new();
    let mut seen = BTreeSet::new();

    for path in json_files(root)? {
        let json = read_to_string(&path)?;
        let record = ReleaseRecord::from_json(&path, &json)?;
        let identity = record.identity.manifest_key();
        if !seen.insert(identity.clone()) {
            return Err(crate::ReleaseError::DuplicateArtifactIdentity { identity });
        }
        records.push(record);
    }

    Ok(records)
}

pub fn load_revocation_records(root: &Utf8Path) -> crate::Result<Vec<RevocationRecord>> {
    let mut records = Vec::new();
    let mut seen = BTreeMap::new();

    for path in json_files(root)? {
        let json = read_to_string(&path)?;
        let record = RevocationRecord::from_json(&path, &json)?;
        let target = record.target_key();
        let metadata = record.revocation_metadata();
        match seen.entry(target) {
            Entry::Vacant(entry) => {
                entry.insert(metadata);
            }
            Entry::Occupied(entry) if entry.get() != &metadata => {
                return Err(crate::ReleaseError::ConflictingRevocation {
                    identity: entry.key().clone(),
                });
            }
            Entry::Occupied(_entry) => {}
        }
        records.push(record);
    }

    Ok(records)
}

fn json_files(root: &Utf8Path) -> crate::Result<Vec<Utf8PathBuf>> {
    let mut paths = Vec::new();
    collect_json_files(root, &mut paths)?;
    paths.sort();
    Ok(paths)
}

fn collect_json_files(root: &Utf8Path, paths: &mut Vec<Utf8PathBuf>) -> crate::Result<()> {
    for entry in root
        .read_dir_utf8()
        .map_err(|error| filesystem_error(root, error))?
    {
        let entry = entry.map_err(|error| filesystem_error(root, error))?;
        let path = entry.path();
        if path.is_dir() {
            collect_json_files(path, paths)?;
        } else if path.extension() == Some("json") {
            paths.push(path.to_path_buf());
        }
    }

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV release tooling reads repository-local release metadata records"
)]
fn read_to_string(path: &Utf8Path) -> crate::Result<String> {
    std::fs::read_to_string(path).map_err(|error| filesystem_error(path, error))
}

fn validate_relative_file_list(
    path: &Utf8Path,
    field: &str,
    values: &[String],
) -> crate::Result<()> {
    for value in values {
        let candidate = Utf8Path::new(value);
        if candidate.is_absolute()
            || value.is_empty()
            || value.contains('\\')
            || candidate
                .components()
                .any(|component| matches!(component.as_str(), "." | ".."))
        {
            return Err(invalid_release(
                path,
                format!("{field} contains invalid relative path `{value}`"),
            ));
        }
    }

    Ok(())
}

fn require_non_empty_release<'a>(
    path: &Utf8Path,
    field: &str,
    value: &'a str,
) -> crate::Result<&'a str> {
    if value.trim().is_empty() {
        Err(invalid_release(path, format!("{field} must not be empty")))
    } else {
        Ok(value)
    }
}

fn require_non_empty_revocation<'a>(
    path: &Utf8Path,
    field: &str,
    value: &'a str,
) -> crate::Result<&'a str> {
    if value.trim().is_empty() {
        Err(invalid_revocation(
            path,
            format!("{field} must not be empty"),
        ))
    } else {
        Ok(value)
    }
}

fn invalid_release(path: &Utf8Path, reason: impl Into<String>) -> crate::ReleaseError {
    crate::ReleaseError::InvalidReleaseRecord {
        path: path.to_string(),
        reason: reason.into(),
    }
}

fn invalid_revocation(path: &Utf8Path, reason: impl Into<String>) -> crate::ReleaseError {
    crate::ReleaseError::InvalidRevocationRecord {
        path: path.to_string(),
        reason: reason.into(),
    }
}

fn invalid_release_identity(
    path: impl ToString,
    field: &str,
    error: ResourcesError,
) -> crate::ReleaseError {
    crate::ReleaseError::InvalidReleaseRecord {
        path: path.to_string(),
        reason: format!("invalid {field}: {error}"),
    }
}

fn invalid_revocation_identity(
    path: impl ToString,
    field: &str,
    error: ResourcesError,
) -> crate::ReleaseError {
    crate::ReleaseError::InvalidRevocationRecord {
        path: path.to_string(),
        reason: format!("invalid {field}: {error}"),
    }
}

fn filesystem_error(path: &Utf8Path, error: impl ToString) -> crate::ReleaseError {
    crate::ReleaseError::Filesystem {
        path: path.to_string(),
        reason: error.to_string(),
    }
}
