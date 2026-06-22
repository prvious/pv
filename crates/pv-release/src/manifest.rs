use camino::Utf8Path;
use resources::{ArtifactManifest, ResourceName, TrackName};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

use crate::defaults::ManifestDefaults;
use crate::record::{
    PhpExtensionRecord, Provenance, ReleaseRecord, RevocationRecord, SourceInput,
    load_release_records, load_revocation_records,
};

pub fn generate_manifest_file(
    records: &Utf8Path,
    revocations: &Utf8Path,
    output: &Utf8Path,
    base_url: &str,
) -> crate::Result<()> {
    generate_manifest_file_with_defaults(records, revocations, None, output, base_url)
}

pub fn generate_manifest_file_with_defaults(
    records: &Utf8Path,
    revocations: &Utf8Path,
    defaults: Option<&Utf8Path>,
    output: &Utf8Path,
    base_url: &str,
) -> crate::Result<()> {
    let records = load_release_records(records)?;
    let revocations = load_revocation_records(revocations)?;
    let defaults = match defaults {
        Some(path) => ManifestDefaults::load(path)?,
        None => ManifestDefaults::empty(),
    };
    let manifest =
        generate_manifest_json_with_defaults(&records, &revocations, &defaults, base_url)?;

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
    let defaults = ManifestDefaults::empty();
    generate_manifest_json_inner(records, revocations, &defaults, base_url)
}

pub fn generate_manifest_json_with_defaults(
    records: &[ReleaseRecord],
    revocations: &[RevocationRecord],
    defaults: &ManifestDefaults,
    base_url: &str,
) -> crate::Result<String> {
    generate_manifest_json_inner(records, revocations, defaults, base_url)
}

fn generate_manifest_json_inner(
    records: &[ReleaseRecord],
    revocations: &[RevocationRecord],
    defaults: &ManifestDefaults,
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
    let mut grouped =
        BTreeMap::<ResourceName, BTreeMap<TrackName, Vec<ManifestArtifactJson>>>::new();

    for record in records {
        let artifact = ManifestArtifactJson::from_record(
            record,
            revocations_by_target.get(&release_key(record)).copied(),
            base_url,
        );
        grouped
            .entry(record.resource().clone())
            .or_default()
            .entry(record.track().clone())
            .or_default()
            .push(artifact);
    }
    validate_manifest_defaults(defaults, &grouped)?;

    let resources = grouped
        .into_iter()
        .map(|(name, tracks)| ManifestResourceJson::from_tracks(name, tracks, defaults))
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    php_extensions: Vec<ManifestPhpExtensionJson>,
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    source_inputs: Vec<ManifestSourceInputJson>,
    recipe: String,
    pv_commit: String,
    build_run_id: String,
}

#[derive(Serialize)]
struct ManifestPhpExtensionJson {
    name: String,
    load_kind: String,
    path: String,
}

#[derive(Serialize)]
struct ManifestSourceInputJson {
    name: String,
    source_url: String,
    source_sha256: String,
}

impl ManifestResourceJson {
    fn from_tracks(
        name: ResourceName,
        tracks: BTreeMap<TrackName, Vec<ManifestArtifactJson>>,
        defaults: &ManifestDefaults,
    ) -> crate::Result<Self> {
        let default_track = default_track_for_resource(&name, &tracks, defaults)?;

        let tracks = tracks
            .into_iter()
            .map(|(name, artifacts)| ManifestTrackJson {
                name: name.as_str().to_string(),
                artifacts,
            })
            .collect::<Vec<_>>();

        Ok(Self {
            name: name.as_str().to_string(),
            default_track: default_track.as_str().to_string(),
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
            php_extensions: record
                .php_extensions()
                .iter()
                .map(ManifestPhpExtensionJson::from_record)
                .collect(),
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
            source_inputs: provenance
                .source_inputs()
                .iter()
                .map(ManifestSourceInputJson::from_source_input)
                .collect(),
            recipe: provenance.recipe().to_string(),
            pv_commit: provenance.pv_commit().to_string(),
            build_run_id: provenance.build_run_id().to_string(),
        }
    }
}

impl ManifestPhpExtensionJson {
    fn from_record(record: &PhpExtensionRecord) -> Self {
        Self {
            name: record.name().to_string(),
            load_kind: record.load_kind().to_string(),
            path: record.path().to_string(),
        }
    }
}

impl ManifestSourceInputJson {
    fn from_source_input(source_input: &SourceInput) -> Self {
        Self {
            name: source_input.name().to_string(),
            source_url: source_input.source_url().to_string(),
            source_sha256: source_input.source_sha256().to_string(),
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

fn default_track_for_resource(
    resource: &ResourceName,
    tracks: &BTreeMap<TrackName, Vec<ManifestArtifactJson>>,
    defaults: &ManifestDefaults,
) -> crate::Result<TrackName> {
    if let Some(default_track) = defaults.default_track_for(resource) {
        if tracks.contains_key(default_track) {
            return Ok(default_track.clone());
        }

        return Err(crate::ReleaseError::GeneratedManifestInvalid {
            reason: format!(
                "resource `{resource}` has explicit default_track `{default_track}` but generated tracks are (`{}`)",
                track_names(tracks)
            ),
        });
    }

    if tracks.len() > 1 {
        return Err(crate::ReleaseError::GeneratedManifestInvalid {
            reason: format!(
                "resource `{resource}` has multiple tracks (`{}`) but no explicit default_track metadata",
                track_names(tracks)
            ),
        });
    }

    let Some(track) = tracks.keys().next() else {
        return Err(crate::ReleaseError::GeneratedManifestInvalid {
            reason: format!("resource `{resource}` has no tracks"),
        });
    };

    Ok(track.clone())
}

fn validate_manifest_defaults(
    defaults: &ManifestDefaults,
    grouped: &BTreeMap<ResourceName, BTreeMap<TrackName, Vec<ManifestArtifactJson>>>,
) -> crate::Result<()> {
    for (resource, default_track) in defaults.entries() {
        let Some(tracks) = grouped.get(resource) else {
            return Err(crate::ReleaseError::GeneratedManifestInvalid {
                reason: format!(
                    "manifest default metadata includes resource `{resource}` but no release records generated that resource"
                ),
            });
        };

        if !tracks.contains_key(default_track) {
            return Err(crate::ReleaseError::GeneratedManifestInvalid {
                reason: format!(
                    "resource `{resource}` has explicit default_track `{default_track}` but generated tracks are (`{}`)",
                    track_names(tracks)
                ),
            });
        }
    }

    Ok(())
}

fn track_names(tracks: &BTreeMap<TrackName, Vec<ManifestArtifactJson>>) -> String {
    tracks
        .keys()
        .map(|track| track.as_str())
        .collect::<Vec<_>>()
        .join(", ")
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
