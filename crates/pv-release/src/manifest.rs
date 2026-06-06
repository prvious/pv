use camino::Utf8Path;
use resources::ArtifactManifest;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

use crate::record::{
    Provenance, ReleaseRecord, RevocationRecord, load_release_records, load_revocation_records,
};

pub fn generate_manifest_file(
    records: &Utf8Path,
    revocations: &Utf8Path,
    output: &Utf8Path,
    base_url: &str,
) -> crate::Result<()> {
    let records = load_release_records(records)?;
    let revocations = load_revocation_records(revocations)?;
    let manifest = generate_manifest_json(&records, &revocations, base_url)?;

    if let Some(parent) = output.parent() {
        create_dir_all(parent)?;
    }
    write(output, &manifest)
}

pub fn generate_manifest_json(
    records: &[ReleaseRecord],
    revocations: &[RevocationRecord],
    base_url: &str,
) -> crate::Result<String> {
    let Some((first_record, remaining_records)) = records.split_first() else {
        return Err(crate::ReleaseError::GeneratedManifestInvalid {
            reason: "release records must not be empty".to_string(),
        });
    };

    let release_keys = records.iter().map(release_key).collect::<BTreeSet<_>>();
    let revoked_keys = revocations
        .iter()
        .map(RevocationRecord::target_key)
        .collect::<BTreeSet<_>>();
    for revocation in revocations {
        let target = revocation.target_key();
        if !release_keys.contains(&target) {
            return Err(crate::ReleaseError::RevocationTargetMissing {
                revocation: target.clone(),
                identity: target,
            });
        }
        validate_replacement(&release_keys, &revoked_keys, revocation)?;
    }

    let mut minimum_pv_version = first_record.minimum_pv_version();
    for record in remaining_records {
        if record.minimum_pv_version() > minimum_pv_version {
            minimum_pv_version = record.minimum_pv_version();
        }
    }

    let revocations_by_target = revocations
        .iter()
        .map(|revocation| (revocation.target_key(), revocation))
        .collect::<BTreeMap<_, _>>();
    let mut grouped = BTreeMap::<String, BTreeMap<String, Vec<ManifestArtifactJson>>>::new();

    for record in records {
        let artifact = ManifestArtifactJson::from_record(
            record,
            revocations_by_target.get(&release_key(record)).copied(),
            base_url,
        );
        grouped
            .entry(record.resource().as_str().to_string())
            .or_default()
            .entry(record.track().as_str().to_string())
            .or_default()
            .push(artifact);
    }

    let resources = grouped
        .into_iter()
        .map(|(name, tracks)| ManifestResourceJson::from_tracks(name, tracks))
        .collect::<crate::Result<Vec<_>>>()?;
    let manifest = ManifestJson {
        schema_version: 1,
        minimum_pv_version: minimum_pv_version.as_str().to_string(),
        resources,
    };
    let json = serde_json::to_string_pretty(&manifest).map_err(|error| {
        crate::ReleaseError::GeneratedManifestInvalid {
            reason: error.to_string(),
        }
    })?;

    ArtifactManifest::parse(&json).map_err(|error| {
        crate::ReleaseError::GeneratedManifestInvalid {
            reason: error.to_string(),
        }
    })?;

    Ok(json)
}

#[derive(Serialize)]
struct ManifestJson {
    schema_version: u64,
    minimum_pv_version: String,
    resources: Vec<ManifestResourceJson>,
}

#[derive(Serialize)]
struct ManifestResourceJson {
    name: String,
    default_track: String,
    tracks: Vec<ManifestTrackJson>,
}

#[derive(Serialize)]
struct ManifestTrackJson {
    name: String,
    artifacts: Vec<ManifestArtifactJson>,
}

#[derive(Serialize)]
struct ManifestArtifactJson {
    artifact_version: String,
    upstream_version: String,
    pv_build_revision: String,
    platform: String,
    url: String,
    sha256: String,
    size: u64,
    published_at: String,
    provenance: ManifestProvenanceJson,
    #[serde(skip_serializing_if = "is_false")]
    revoked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    revocation_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revoked_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    replacement_artifact_version: Option<String>,
}

#[derive(Serialize)]
struct ManifestProvenanceJson {
    source_url: String,
    source_sha256: String,
    recipe: String,
    pv_commit: String,
    build_run_id: String,
}

impl ManifestResourceJson {
    fn from_tracks(
        name: String,
        tracks: BTreeMap<String, Vec<ManifestArtifactJson>>,
    ) -> crate::Result<Self> {
        if tracks.len() > 1 {
            let track_names = tracks.keys().cloned().collect::<Vec<_>>().join(", ");
            return Err(crate::ReleaseError::GeneratedManifestInvalid {
                reason: format!(
                    "resource `{name}` has multiple tracks (`{track_names}`) but no explicit default_track metadata"
                ),
            });
        }

        let tracks = tracks
            .into_iter()
            .map(|(name, artifacts)| ManifestTrackJson { name, artifacts })
            .collect::<Vec<_>>();

        let Some(track) = tracks.first() else {
            return Err(crate::ReleaseError::GeneratedManifestInvalid {
                reason: format!("resource `{name}` has no tracks"),
            });
        };
        let default_track = track.name.clone();

        Ok(Self {
            name,
            default_track,
            tracks,
        })
    }
}

impl ManifestArtifactJson {
    fn from_record(
        record: &ReleaseRecord,
        revocation: Option<&RevocationRecord>,
        base_url: &str,
    ) -> Self {
        Self {
            artifact_version: record.artifact_version().as_str().to_string(),
            upstream_version: record.upstream_version().to_string(),
            pv_build_revision: record.pv_build_revision().to_string(),
            platform: record.platform().as_str().to_string(),
            url: artifact_url(base_url, record.object_key()),
            sha256: record.sha256().as_str().to_string(),
            size: record.size(),
            published_at: record.published_at_raw().to_string(),
            provenance: ManifestProvenanceJson::from_provenance(record.provenance()),
            revoked: revocation.is_some(),
            revocation_reason: revocation.map(|revocation| revocation.reason().to_string()),
            revoked_at: revocation.map(|revocation| revocation.revoked_at().to_string()),
            replacement_artifact_version: revocation
                .and_then(RevocationRecord::replacement_artifact_version)
                .map(|artifact_version| artifact_version.as_str().to_string()),
        }
    }
}

impl ManifestProvenanceJson {
    fn from_provenance(provenance: &Provenance) -> Self {
        Self {
            source_url: provenance.source_url().to_string(),
            source_sha256: provenance.source_sha256().to_string(),
            recipe: provenance.recipe().to_string(),
            pv_commit: provenance.pv_commit().to_string(),
            build_run_id: provenance.build_run_id().to_string(),
        }
    }
}

fn release_key(record: &ReleaseRecord) -> String {
    format!(
        "{}:{}:{}:{}",
        record.resource(),
        record.track(),
        record.artifact_version(),
        record.platform()
    )
}

fn validate_replacement(
    release_keys: &BTreeSet<String>,
    revoked_keys: &BTreeSet<String>,
    revocation: &RevocationRecord,
) -> crate::Result<()> {
    let Some(replacement_key) = revocation.replacement_key() else {
        return Ok(());
    };
    let revocation_key = revocation.target_key();

    let reason = if replacement_key == revocation_key {
        Some("replacement must not point at the revoked artifact itself")
    } else if !release_keys.contains(&replacement_key) {
        Some("replacement release must exist for the same resource, track, and platform")
    } else if revoked_keys.contains(&replacement_key) {
        Some("replacement release must not also be revoked")
    } else {
        None
    };

    if let Some(reason) = reason {
        Err(crate::ReleaseError::RevocationReplacementInvalid {
            revocation: revocation_key,
            replacement: replacement_key,
            reason: reason.to_string(),
        })
    } else {
        Ok(())
    }
}

fn artifact_url(base_url: &str, object_key: &str) -> String {
    format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        object_key.trim_start_matches('/')
    )
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV release tooling writes generated artifact manifest files"
)]
fn create_dir_all(path: &Utf8Path) -> crate::Result<()> {
    std::fs::create_dir_all(path).map_err(|error| filesystem_error(path, error))
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV release tooling writes generated artifact manifest files"
)]
fn write(path: &Utf8Path, content: &str) -> crate::Result<()> {
    std::fs::write(path, content).map_err(|error| filesystem_error(path, error))
}

fn filesystem_error(path: &Utf8Path, error: impl ToString) -> crate::ReleaseError {
    crate::ReleaseError::Filesystem {
        path: path.to_string(),
        reason: error.to_string(),
    }
}
