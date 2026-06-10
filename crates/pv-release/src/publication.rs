use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::Utf8TempDir;
use resources::{
    ArtifactManifest, ArtifactPlatform, ResourceName, ResourcesError, TargetPlatform, TrackName,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

use crate::defaults::ManifestDefaults;
use crate::record::ReleaseRecord;

const REQUIRED_NATIVE_PUBLICATION_PLATFORMS: &[(TargetPlatform, ArtifactPlatform)] = &[
    // StaticPHP v3 currently blocks reliable FrankenPHP publication for Intel
    // macOS. Keep public preview publication Apple Silicon-only until that
    // upstream build path is stable enough to publish darwin-amd64 artifacts.
    (TargetPlatform::DarwinArm64, ArtifactPlatform::DarwinArm64),
];

#[derive(Clone, Debug)]
pub struct PublicationRequest {
    pub source_archives: Utf8PathBuf,
    pub candidate_records: Utf8PathBuf,
    pub published_records: Utf8PathBuf,
    pub published_revocations: Utf8PathBuf,
    pub defaults: Utf8PathBuf,
    pub stage: Utf8PathBuf,
    pub base_url: String,
    pub versioned_manifest_key: String,
    pub stable_manifest_key: String,
}

#[derive(Debug)]
struct ReleaseRecordFile {
    path: Utf8PathBuf,
    record: ReleaseRecord,
}

#[derive(Serialize)]
struct PublicationPlan {
    immutable_uploads: Vec<PublicationPlanObject>,
    versioned_manifest: PublicationPlanObject,
    stable_manifest: PublicationPlanObject,
}

#[derive(Serialize)]
struct PublicationPlanObject {
    local_path: String,
    object_key: String,
}

struct CandidatePublication {
    source_archive: Utf8PathBuf,
    source_record: Utf8PathBuf,
    archive_local_path: Utf8PathBuf,
    archive_object_key: String,
    record_local_path: Utf8PathBuf,
    record_object_key: String,
}

pub fn prepare_publication(request: &PublicationRequest) -> crate::Result<()> {
    validate_publication_key(&request.versioned_manifest_key)?;
    validate_publication_key(&request.stable_manifest_key)?;
    validate_stable_manifest_key(&request.stable_manifest_key)?;

    let candidate_records = load_release_record_files(&request.candidate_records)?;
    let published_records = load_release_record_files(&request.published_records)?;
    let published_identities = published_records
        .iter()
        .map(|record_file| record_file.record.identity().manifest_key())
        .collect::<BTreeSet<_>>();
    for candidate in &candidate_records {
        let identity = candidate.record.identity().manifest_key();
        if published_identities.contains(&identity) {
            return Err(crate::ReleaseError::DuplicateArtifactIdentity { identity });
        }
    }

    let defaults = ManifestDefaults::load(&request.defaults)?;
    let mut candidates = Vec::new();
    for candidate in &candidate_records {
        let archive_name = archive_name(candidate.record.object_key())?;
        let source_archive = find_source_archive(&request.source_archives, archive_name)?;
        let license_files = candidate
            .record
            .license_files()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let notice_files = candidate
            .record
            .notice_files()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let validation =
            crate::archive::validate_archive(&source_archive, &license_files, &notice_files)?;
        candidate.record.verify_archive(&validation)?;

        let archive_local_path = Utf8PathBuf::from("archives").join(candidate.record.object_key());
        let archive_object_key = candidate.record.object_key().to_string();
        let record_object_key = record_object_key(candidate.record.object_key())?;
        let record_local_path = Utf8PathBuf::from(&record_object_key);

        candidates.push(CandidatePublication {
            source_archive,
            source_record: candidate.path.clone(),
            archive_local_path,
            archive_object_key,
            record_local_path,
            record_object_key,
        });
    }

    validate_publication_object_keys(request, &candidates)?;
    validate_publication_local_paths(request, &candidates)?;
    validate_default_release_record_platform_matrix(
        &defaults,
        &published_records,
        &candidate_records,
    )?;
    stage_immutable_uploads(request, &candidates)?;
    let tempdir = Utf8TempDir::new().map_err(|error| filesystem_error(&request.stage, error))?;
    let combined_records = tempdir.path().join("records");
    combine_manifest_records(
        &combined_records,
        &request.published_records,
        request,
        &candidates,
    )?;

    let versioned_manifest_path = request.stage.join(&request.versioned_manifest_key);
    ensure_immutable_target_absent(&request.versioned_manifest_key, &versioned_manifest_path)?;
    crate::manifest::generate_manifest_file_with_defaults(
        &combined_records,
        &request.published_revocations,
        Some(&request.defaults),
        &versioned_manifest_path,
        &request.base_url,
    )?;
    let manifest_json = read_to_string(&versioned_manifest_path)?;
    let manifest = ArtifactManifest::parse(&manifest_json).map_err(|error| {
        crate::ReleaseError::GeneratedManifestInvalid {
            reason: error.to_string(),
        }
    })?;
    validate_public_manifest_platform_matrix(&manifest)?;

    let stable_manifest_path = request.stage.join(&request.stable_manifest_key);
    write(&stable_manifest_path, &manifest_json)?;

    let plan = publication_plan(request, &candidates);
    let plan_json = serde_json::to_string_pretty(&plan).map_err(|error| {
        crate::ReleaseError::InvalidPublicationInput {
            path: request.stage.to_string(),
            reason: error.to_string(),
        }
    })?;
    write(
        &request.stage.join("publication-plan.json"),
        &format!("{plan_json}\n"),
    )
}

fn validate_default_release_record_platform_matrix(
    defaults: &ManifestDefaults,
    published_records: &[ReleaseRecordFile],
    candidate_records: &[ReleaseRecordFile],
) -> crate::Result<()> {
    let mut platforms_by_default = BTreeMap::<(String, String), BTreeSet<ArtifactPlatform>>::new();
    for record_file in published_records.iter().chain(candidate_records) {
        let record = &record_file.record;
        if let Some(default_track) = defaults.default_track_for(record.resource())
            && record.track() == default_track
        {
            platforms_by_default
                .entry((
                    record.resource().as_str().to_string(),
                    record.track().as_str().to_string(),
                ))
                .or_default()
                .insert(record.platform());
        }
    }

    for (resource, track) in defaults.entries() {
        let key = (resource.as_str().to_string(), track.as_str().to_string());
        let platforms = platforms_by_default.get(&key);
        if resource.as_str() == "composer" {
            if platforms
                .map(|platforms| platforms.contains(&ArtifactPlatform::Any))
                .unwrap_or(false)
            {
                continue;
            }

            return Err(crate::ReleaseError::GeneratedManifestInvalid {
                reason: format!(
                    "public stable manifest default resource `{resource}` track `{track}` is missing required portable platform: any",
                ),
            });
        }

        let missing = REQUIRED_NATIVE_PUBLICATION_PLATFORMS
            .iter()
            .map(|(_target, platform)| *platform)
            .filter(|platform| {
                !platforms
                    .map(|platforms| platforms.contains(platform))
                    .unwrap_or(false)
            })
            .map(ArtifactPlatform::as_str)
            .collect::<Vec<_>>();

        if !missing.is_empty() {
            return Err(crate::ReleaseError::GeneratedManifestInvalid {
                reason: format!(
                    "public stable manifest default resource `{resource}` track `{track}` is missing required platform(s): {}",
                    missing.join(", ")
                ),
            });
        }
    }

    Ok(())
}

fn validate_public_manifest_platform_matrix(manifest: &ArtifactManifest) -> crate::Result<()> {
    for (resource, track) in manifest.resource_tracks() {
        if resource.as_str() == "composer" {
            validate_public_portable_track(manifest, resource, track)?;
            continue;
        }

        let missing = REQUIRED_NATIVE_PUBLICATION_PLATFORMS
            .iter()
            .filter_map(|(target, expected_platform)| {
                match selects_expected_platform(
                    manifest,
                    resource,
                    track,
                    *target,
                    *expected_platform,
                ) {
                    Ok(true) => None,
                    Ok(false) => Some(Ok(target.as_str())),
                    Err(error) => Some(Err(error)),
                }
            })
            .collect::<crate::Result<Vec<_>>>()?;

        if !missing.is_empty() {
            return Err(crate::ReleaseError::GeneratedManifestInvalid {
                reason: format!(
                    "public stable manifest resource `{resource}` track `{track}` is missing required platform(s): {}",
                    missing.join(", ")
                ),
            });
        }
    }

    Ok(())
}

fn validate_public_portable_track(
    manifest: &ArtifactManifest,
    resource: &ResourceName,
    track: &TrackName,
) -> crate::Result<()> {
    match manifest.select_latest(resource, track, TargetPlatform::DarwinArm64) {
        Ok(selection) if selection.artifact().platform() == ArtifactPlatform::Any => Ok(()),
        Ok(_) | Err(ResourcesError::NoInstallableArtifact { .. }) => {
            Err(crate::ReleaseError::GeneratedManifestInvalid {
                reason: format!(
                    "public stable manifest resource `{resource}` track `{track}` is missing required portable platform: any",
                ),
            })
        }
        Err(error) => Err(generated_manifest_error(error)),
    }
}

fn selects_expected_platform(
    manifest: &ArtifactManifest,
    resource: &ResourceName,
    track: &TrackName,
    target: TargetPlatform,
    expected_platform: ArtifactPlatform,
) -> crate::Result<bool> {
    match manifest.select_latest(resource, track, target) {
        Ok(selection) => Ok(selection.artifact().platform() == expected_platform),
        Err(ResourcesError::NoInstallableArtifact { .. }) => Ok(false),
        Err(error) => Err(generated_manifest_error(error)),
    }
}

fn generated_manifest_error(error: ResourcesError) -> crate::ReleaseError {
    crate::ReleaseError::GeneratedManifestInvalid {
        reason: error.to_string(),
    }
}

fn validate_publication_object_keys(
    request: &PublicationRequest,
    candidates: &[CandidatePublication],
) -> crate::Result<()> {
    let mut seen = BTreeMap::new();
    record_publication_object_key(
        &mut seen,
        &request.versioned_manifest_key,
        "versioned manifest",
    )?;
    record_publication_object_key(&mut seen, &request.stable_manifest_key, "stable manifest")?;

    for candidate in candidates {
        record_publication_object_key(
            &mut seen,
            &candidate.archive_object_key,
            "candidate archive",
        )?;
        record_publication_object_key(
            &mut seen,
            &candidate.record_object_key,
            "candidate release record",
        )?;
    }

    Ok(())
}

fn record_publication_object_key(
    seen: &mut BTreeMap<String, String>,
    object_key: &str,
    purpose: &str,
) -> crate::Result<()> {
    if let Some(existing) = seen.insert(object_key.to_string(), purpose.to_string()) {
        Err(crate::ReleaseError::InvalidPublicationInput {
            path: object_key.to_string(),
            reason: format!("publication object key collides between `{existing}` and `{purpose}`"),
        })
    } else {
        Ok(())
    }
}

fn validate_publication_local_paths(
    request: &PublicationRequest,
    candidates: &[CandidatePublication],
) -> crate::Result<()> {
    let mut seen = BTreeMap::new();
    record_publication_local_path(
        &mut seen,
        &request.versioned_manifest_key,
        "versioned manifest",
    )?;
    record_publication_local_path(&mut seen, &request.stable_manifest_key, "stable manifest")?;
    record_publication_local_path(&mut seen, "publication-plan.json", "publication plan")?;

    for candidate in candidates {
        record_publication_local_path(
            &mut seen,
            candidate.archive_local_path.as_str(),
            "candidate archive",
        )?;
        record_publication_local_path(
            &mut seen,
            candidate.record_local_path.as_str(),
            "candidate release record",
        )?;
    }

    Ok(())
}

fn record_publication_local_path(
    seen: &mut BTreeMap<String, String>,
    local_path: &str,
    purpose: &str,
) -> crate::Result<()> {
    if let Some(existing) = seen.insert(local_path.to_string(), purpose.to_string()) {
        Err(crate::ReleaseError::InvalidPublicationInput {
            path: local_path.to_string(),
            reason: format!("publication local path collides between `{existing}` and `{purpose}`"),
        })
    } else {
        Ok(())
    }
}

fn stage_immutable_uploads(
    request: &PublicationRequest,
    candidates: &[CandidatePublication],
) -> crate::Result<()> {
    for candidate in candidates {
        let archive_stage_path = request.stage.join(&candidate.archive_local_path);
        ensure_immutable_target_absent(&candidate.archive_object_key, &archive_stage_path)?;
        copy_file(&candidate.source_archive, &archive_stage_path)?;

        let record_stage_path = request.stage.join(&candidate.record_local_path);
        ensure_immutable_target_absent(&candidate.record_object_key, &record_stage_path)?;
        copy_file(&candidate.source_record, &record_stage_path)?;
    }

    Ok(())
}

fn combine_manifest_records(
    combined_records: &Utf8Path,
    published_records: &Utf8Path,
    request: &PublicationRequest,
    candidates: &[CandidatePublication],
) -> crate::Result<()> {
    create_dir_all(combined_records)?;
    copy_json_tree(published_records, &combined_records.join("published"))?;
    for candidate in candidates {
        let staged_record = request.stage.join(&candidate.record_local_path);
        copy_file(
            &staged_record,
            &combined_records
                .join("candidates")
                .join(&candidate.record_local_path),
        )?;
    }

    Ok(())
}

fn publication_plan(
    request: &PublicationRequest,
    candidates: &[CandidatePublication],
) -> PublicationPlan {
    let immutable_uploads = candidates
        .iter()
        .flat_map(|candidate| {
            [
                PublicationPlanObject {
                    local_path: candidate.archive_local_path.to_string(),
                    object_key: candidate.archive_object_key.clone(),
                },
                PublicationPlanObject {
                    local_path: candidate.record_local_path.to_string(),
                    object_key: candidate.record_object_key.clone(),
                },
            ]
        })
        .collect::<Vec<_>>();

    PublicationPlan {
        immutable_uploads,
        versioned_manifest: PublicationPlanObject {
            local_path: request.versioned_manifest_key.clone(),
            object_key: request.versioned_manifest_key.clone(),
        },
        stable_manifest: PublicationPlanObject {
            local_path: request.stable_manifest_key.clone(),
            object_key: request.stable_manifest_key.clone(),
        },
    }
}

fn load_release_record_files(root: &Utf8Path) -> crate::Result<Vec<ReleaseRecordFile>> {
    let mut paths = Vec::new();
    collect_json_files(root, &mut paths)?;
    paths.sort();

    let mut records = Vec::new();
    let mut seen = BTreeSet::new();
    for path in paths {
        let json = read_to_string(&path)?;
        let record = ReleaseRecord::from_json(&path, &json)?;
        let identity = record.identity().manifest_key();
        if !seen.insert(identity.clone()) {
            return Err(crate::ReleaseError::DuplicateArtifactIdentity { identity });
        }
        records.push(ReleaseRecordFile { path, record });
    }

    Ok(records)
}

fn copy_json_tree(source_root: &Utf8Path, output_root: &Utf8Path) -> crate::Result<()> {
    let mut paths = Vec::new();
    collect_json_files(source_root, &mut paths)?;
    for source in paths {
        let relative = source
            .strip_prefix(source_root)
            .map_err(|error| filesystem_error(&source, error))?;
        copy_file(&source, &output_root.join(relative))?;
    }

    Ok(())
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

fn find_source_archive(source_root: &Utf8Path, archive_name: &str) -> crate::Result<Utf8PathBuf> {
    let mut matches = Vec::new();
    collect_named_files(source_root, archive_name, &mut matches)?;

    match matches.len() {
        0 => Err(crate::ReleaseError::InvalidPublicationInput {
            path: source_root.to_string(),
            reason: format!("missing source archive `{archive_name}`"),
        }),
        1 => {
            matches
                .into_iter()
                .next()
                .ok_or_else(|| crate::ReleaseError::InvalidPublicationInput {
                    path: source_root.to_string(),
                    reason: format!("missing source archive `{archive_name}`"),
                })
        }
        count => Err(crate::ReleaseError::InvalidPublicationInput {
            path: source_root.to_string(),
            reason: format!("found {count} source archives named `{archive_name}`"),
        }),
    }
}

fn collect_named_files(
    root: &Utf8Path,
    file_name: &str,
    matches: &mut Vec<Utf8PathBuf>,
) -> crate::Result<()> {
    for entry in root
        .read_dir_utf8()
        .map_err(|error| filesystem_error(root, error))?
    {
        let entry = entry.map_err(|error| filesystem_error(root, error))?;
        let path = entry.path();
        if path.is_dir() {
            collect_named_files(path, file_name, matches)?;
        } else if path.file_name() == Some(file_name) {
            matches.push(path.to_path_buf());
        }
    }

    Ok(())
}

fn archive_name(object_key: &str) -> crate::Result<&str> {
    Utf8Path::new(object_key).file_name().ok_or_else(|| {
        crate::ReleaseError::InvalidPublicationInput {
            path: object_key.to_string(),
            reason: "object key must end with an archive file name".to_string(),
        }
    })
}

fn record_object_key(archive_object_key: &str) -> crate::Result<String> {
    let Some(prefix) = archive_object_key.strip_suffix(".tar.gz") else {
        return Err(crate::ReleaseError::InvalidPublicationInput {
            path: archive_object_key.to_string(),
            reason: "archive object key must end with `.tar.gz`".to_string(),
        });
    };

    Ok(format!("records/{prefix}.json"))
}

fn validate_publication_key(value: &str) -> crate::Result<()> {
    let candidate = Utf8Path::new(value);
    if !candidate.is_absolute()
        && !value.is_empty()
        && !value.contains('\\')
        && !value.split('/').any(str::is_empty)
        && !candidate
            .components()
            .any(|component| matches!(component.as_str(), "." | ".."))
    {
        Ok(())
    } else {
        Err(crate::ReleaseError::InvalidPublicationInput {
            path: value.to_string(),
            reason: "object key must be a safe relative path".to_string(),
        })
    }
}

fn validate_stable_manifest_key(value: &str) -> crate::Result<()> {
    if value == "manifest.json" {
        Ok(())
    } else {
        Err(crate::ReleaseError::InvalidPublicationInput {
            path: value.to_string(),
            reason: "stable manifest key must be `manifest.json`".to_string(),
        })
    }
}

fn ensure_immutable_target_absent(key: &str, path: &Utf8Path) -> crate::Result<()> {
    if path_exists(path) {
        Err(crate::ReleaseError::ImmutablePublicationObjectExists {
            key: key.to_string(),
        })
    } else {
        Ok(())
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV release tooling stages local publication files"
)]
fn copy_file(source: &Utf8Path, output: &Utf8Path) -> crate::Result<()> {
    if let Some(parent) = output.parent() {
        create_dir_all(parent)?;
    }
    std::fs::copy(source, output).map_err(|error| filesystem_error(output, error))?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV release tooling writes generated publication files"
)]
fn write(path: &Utf8Path, content: &str) -> crate::Result<()> {
    if let Some(parent) = path.parent() {
        create_dir_all(parent)?;
    }
    std::fs::write(path, content).map_err(|error| filesystem_error(path, error))
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV release tooling creates local publication directories"
)]
fn create_dir_all(path: &Utf8Path) -> crate::Result<()> {
    std::fs::create_dir_all(path).map_err(|error| filesystem_error(path, error))
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV release tooling reads generated publication files"
)]
fn read_to_string(path: &Utf8Path) -> crate::Result<String> {
    std::fs::read_to_string(path).map_err(|error| filesystem_error(path, error))
}

fn path_exists(path: &Utf8Path) -> bool {
    path.exists()
}

fn filesystem_error(path: &Utf8Path, error: impl ToString) -> crate::ReleaseError {
    crate::ReleaseError::Filesystem {
        path: path.to_string(),
        reason: error.to_string(),
    }
}
