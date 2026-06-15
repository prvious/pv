use camino::{Utf8Path, Utf8PathBuf};
use data_encoding::HEXLOWER;
use self_update::{AppUpdateManifest, AppUpdateVersion};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Read;

use crate::app::AppReleaseRecord;

const VERSIONED_APP_MANIFEST_NAME: &str = "pv-app-manifest.json";
const VERSIONED_INSTALLER_NAME: &str = "install.sh";
const STABLE_APP_MANIFEST_KEY: &str = "pv-app-manifest.json";
const STABLE_INSTALLER_KEY: &str = "install.sh";

#[derive(Clone, Debug)]
pub struct AppPublicationRequest {
    pub source_binaries: Utf8PathBuf,
    pub candidate_records: Utf8PathBuf,
    pub stage: Utf8PathBuf,
    pub source_run_id: String,
    pub base_url: String,
    pub current_app_manifest: Option<Utf8PathBuf>,
    pub current_app_installer: Option<Utf8PathBuf>,
}

#[derive(Debug)]
struct AppPublicationCandidate {
    record: AppReleaseRecord,
    source_binary: Utf8PathBuf,
    source_record: Utf8PathBuf,
    binary_local_path: Utf8PathBuf,
    binary_object_key: String,
    record_local_path: Utf8PathBuf,
    record_object_key: String,
}

#[derive(Serialize)]
struct AppPublicationPlan {
    immutable_uploads: Vec<AppPublicationPlanObject>,
    stable_app_manifest: AppPublicationPlanObject,
    stable_installer: AppPublicationPlanObject,
}

#[derive(Serialize)]
struct AppPublicationPlanObject {
    local_path: String,
    object_key: String,
}

pub fn stage_app_publication(request: &AppPublicationRequest) -> crate::Result<()> {
    validate_source_run_id(&request.source_run_id)?;
    let records = crate::app::load_app_release_records(&request.candidate_records)?;
    validate_records_match_source_run(&records, &request.source_run_id)?;
    validate_candidate_is_not_older_than_current(
        &records,
        request.current_app_manifest.as_deref(),
        request.current_app_installer.as_deref(),
    )?;
    let manifest = crate::app::generate_app_manifest_json(&records, &request.base_url)?;
    let installer = crate::app::generate_app_installer_script(&records, &request.base_url)?;

    let candidates = app_publication_candidates(request, &records)?;
    let versioned_app_manifest_key =
        versioned_generated_key(&request.source_run_id, VERSIONED_APP_MANIFEST_NAME);
    let versioned_installer_key =
        versioned_generated_key(&request.source_run_id, VERSIONED_INSTALLER_NAME);
    validate_app_publication_object_keys(
        &candidates,
        &versioned_app_manifest_key,
        &versioned_installer_key,
    )?;
    validate_app_publication_local_paths(
        &candidates,
        &versioned_app_manifest_key,
        &versioned_installer_key,
    )?;

    for candidate in &candidates {
        verify_binary(&candidate.record, &candidate.source_binary)?;
        let binary_stage_path = request.stage.join(&candidate.binary_local_path);
        ensure_immutable_target_absent(&candidate.binary_object_key, &binary_stage_path)?;
        let record_stage_path = request.stage.join(&candidate.record_local_path);
        ensure_immutable_target_absent(&candidate.record_object_key, &record_stage_path)?;
    }

    let versioned_manifest_path = request.stage.join(&versioned_app_manifest_key);
    ensure_immutable_target_absent(&versioned_app_manifest_key, &versioned_manifest_path)?;
    let versioned_installer_path = request.stage.join(&versioned_installer_key);
    ensure_immutable_target_absent(&versioned_installer_key, &versioned_installer_path)?;

    for candidate in &candidates {
        let binary_stage_path = request.stage.join(&candidate.binary_local_path);
        copy_file(&candidate.source_binary, &binary_stage_path)?;

        let record_stage_path = request.stage.join(&candidate.record_local_path);
        copy_file(&candidate.source_record, &record_stage_path)?;
    }

    write(&versioned_manifest_path, &manifest)?;
    write(&versioned_installer_path, &installer)?;
    write(&request.stage.join(STABLE_APP_MANIFEST_KEY), &manifest)?;
    write(&request.stage.join(STABLE_INSTALLER_KEY), &installer)?;

    let plan = app_publication_plan(
        &candidates,
        &versioned_app_manifest_key,
        &versioned_installer_key,
    );
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

fn app_publication_candidates(
    request: &AppPublicationRequest,
    records: &[AppReleaseRecord],
) -> crate::Result<Vec<AppPublicationCandidate>> {
    records
        .iter()
        .map(|record| {
            validate_app_binary_object_key(record.object_key())?;
            let source_binary = request.source_binaries.join(record.object_key());
            let record_object_key = format!(
                "pv/records/{}/pv-{}.json",
                record.version(),
                record.platform().as_str()
            );
            validate_app_record_object_key(&record_object_key)?;

            Ok(AppPublicationCandidate {
                record: record.clone(),
                source_binary,
                source_record: record.path().to_path_buf(),
                binary_local_path: Utf8PathBuf::from(record.object_key()),
                binary_object_key: record.object_key().to_string(),
                record_local_path: Utf8PathBuf::from(&record_object_key),
                record_object_key,
            })
        })
        .collect()
}

fn validate_records_match_source_run(
    records: &[AppReleaseRecord],
    source_run_id: &str,
) -> crate::Result<()> {
    for record in records {
        if record.provenance().build_run_id() != source_run_id {
            return Err(crate::ReleaseError::InvalidPublicationInput {
                path: record.path().to_string(),
                reason: format!(
                    "app release record build_run_id `{}` does not match source_run_id `{source_run_id}`",
                    record.provenance().build_run_id()
                ),
            });
        }
    }

    Ok(())
}

fn validate_candidate_is_not_older_than_current(
    records: &[AppReleaseRecord],
    current_app_manifest: Option<&Utf8Path>,
    current_app_installer: Option<&Utf8Path>,
) -> crate::Result<()> {
    if current_app_manifest.is_none() && current_app_installer.is_none() {
        return Ok(());
    }

    let Some(first_record) = records.first() else {
        let path = current_app_manifest
            .or(current_app_installer)
            .map(Utf8Path::to_string)
            .unwrap_or_else(|| "current-stable-app".to_string());
        return Err(crate::ReleaseError::InvalidPublicationInput {
            path,
            reason: "current stable app metadata was provided but no candidate records were found"
                .to_string(),
        });
    };

    let candidate_version =
        AppUpdateVersion::parse(first_record.version().to_string()).map_err(|error| {
            crate::ReleaseError::InvalidPublicationInput {
                path: first_record.path().to_string(),
                reason: format!("failed to parse candidate app version: {error}"),
            }
        })?;
    let Some(current_version) =
        current_stable_app_version(current_app_manifest, current_app_installer)?
    else {
        return Ok(());
    };

    if candidate_version < current_version.version {
        return Err(crate::ReleaseError::InvalidPublicationInput {
            path: first_record.path().to_string(),
            reason: format!(
                "candidate app version `{candidate_version}` must not be older than current stable {} `{}`",
                current_version.source, current_version.version
            ),
        });
    }

    Ok(())
}

struct CurrentStableAppVersion {
    version: AppUpdateVersion,
    source: &'static str,
}

fn current_stable_app_version(
    current_app_manifest: Option<&Utf8Path>,
    current_app_installer: Option<&Utf8Path>,
) -> crate::Result<Option<CurrentStableAppVersion>> {
    let mut current_version = None;
    if let Some(current_app_manifest) = current_app_manifest {
        let version = current_stable_app_manifest_version(current_app_manifest)?;
        record_current_stable_app_version(&mut current_version, version, "app manifest");
    }

    if let Some(current_app_installer) = current_app_installer {
        let version = current_stable_app_installer_version(current_app_installer)?;
        record_current_stable_app_version(&mut current_version, version, "installer");
    }

    Ok(current_version)
}

fn record_current_stable_app_version(
    current_version: &mut Option<CurrentStableAppVersion>,
    version: AppUpdateVersion,
    source: &'static str,
) {
    let should_replace = current_version
        .as_ref()
        .map(|current_version| version > current_version.version)
        .unwrap_or(true);
    if should_replace {
        *current_version = Some(CurrentStableAppVersion { version, source });
    }
}

fn current_stable_app_manifest_version(
    current_app_manifest: &Utf8Path,
) -> crate::Result<AppUpdateVersion> {
    let current_json = read_to_string(current_app_manifest)?;
    let current_manifest = AppUpdateManifest::parse(&current_json).map_err(|error| {
        crate::ReleaseError::InvalidPublicationInput {
            path: current_app_manifest.to_string(),
            reason: format!("failed to parse current stable app manifest: {error}"),
        }
    })?;

    Ok(current_manifest.version().clone())
}

fn current_stable_app_installer_version(
    current_app_installer: &Utf8Path,
) -> crate::Result<AppUpdateVersion> {
    let installer = read_to_string(current_app_installer)?;
    let Some(version) = installer.lines().find_map(installer_version_line) else {
        return Err(crate::ReleaseError::InvalidPublicationInput {
            path: current_app_installer.to_string(),
            reason: "current stable installer does not define PV_VERSION".to_string(),
        });
    };

    AppUpdateVersion::parse(version.to_string()).map_err(|error| {
        crate::ReleaseError::InvalidPublicationInput {
            path: current_app_installer.to_string(),
            reason: format!("failed to parse current stable installer version: {error}"),
        }
    })
}

fn installer_version_line(line: &str) -> Option<&str> {
    let value = line.trim().strip_prefix("PV_VERSION=")?;
    value.strip_prefix('\'')?.strip_suffix('\'')
}

fn validate_app_binary_object_key(object_key: &str) -> crate::Result<()> {
    validate_not_managed_resource_key(object_key)?;
    if object_key.starts_with("pv/") {
        Ok(())
    } else {
        Err(crate::ReleaseError::InvalidPublicationInput {
            path: object_key.to_string(),
            reason: "app binary object key must be under `pv/`".to_string(),
        })
    }
}

fn validate_app_record_object_key(object_key: &str) -> crate::Result<()> {
    validate_not_managed_resource_key(object_key)?;
    if object_key.starts_with("pv/records/") {
        Ok(())
    } else {
        Err(crate::ReleaseError::InvalidPublicationInput {
            path: object_key.to_string(),
            reason: "app record object key must be under `pv/records/`".to_string(),
        })
    }
}

fn validate_not_managed_resource_key(object_key: &str) -> crate::Result<()> {
    if object_key == "manifest.json"
        || object_key.starts_with("resources/")
        || object_key.starts_with("records/")
        || object_key.starts_with("revocations/")
    {
        Err(crate::ReleaseError::InvalidPublicationInput {
            path: object_key.to_string(),
            reason: "app publication object key is reserved for Managed Resource publication"
                .to_string(),
        })
    } else {
        Ok(())
    }
}

fn validate_app_publication_object_keys(
    candidates: &[AppPublicationCandidate],
    versioned_app_manifest_key: &str,
    versioned_installer_key: &str,
) -> crate::Result<()> {
    let mut seen = BTreeMap::new();
    for candidate in candidates {
        record_app_publication_object_key(
            &mut seen,
            &candidate.binary_object_key,
            "app binary records",
        )?;
        record_app_publication_object_key(
            &mut seen,
            &candidate.record_object_key,
            "app release records",
        )?;
    }

    record_app_publication_object_key(
        &mut seen,
        versioned_app_manifest_key,
        "versioned app manifest",
    )?;
    record_app_publication_object_key(&mut seen, versioned_installer_key, "versioned installer")?;
    record_app_publication_object_key(&mut seen, STABLE_APP_MANIFEST_KEY, "stable app manifest")?;
    record_app_publication_object_key(&mut seen, STABLE_INSTALLER_KEY, "stable installer")
}

fn record_app_publication_object_key(
    seen: &mut BTreeMap<String, String>,
    object_key: &str,
    purpose: &str,
) -> crate::Result<()> {
    validate_not_managed_resource_key(object_key)?;
    if let Some(existing) = seen.insert(object_key.to_string(), purpose.to_string()) {
        let reason = if existing == "app binary records" && purpose == "app binary records" {
            "app publication object key collides between app binary records".to_string()
        } else {
            format!("app publication object key collides between `{existing}` and `{purpose}`")
        };
        Err(crate::ReleaseError::InvalidPublicationInput {
            path: object_key.to_string(),
            reason,
        })
    } else {
        Ok(())
    }
}

fn validate_app_publication_local_paths(
    candidates: &[AppPublicationCandidate],
    versioned_app_manifest_key: &str,
    versioned_installer_key: &str,
) -> crate::Result<()> {
    let mut seen = BTreeMap::new();
    for candidate in candidates {
        record_app_publication_local_path(
            &mut seen,
            candidate.binary_local_path.as_str(),
            "app binary",
        )?;
        record_app_publication_local_path(
            &mut seen,
            candidate.record_local_path.as_str(),
            "app release record",
        )?;
    }

    for (local_path, purpose) in [
        (versioned_app_manifest_key, "versioned app manifest"),
        (versioned_installer_key, "versioned installer"),
        (STABLE_APP_MANIFEST_KEY, "stable app manifest"),
        (STABLE_INSTALLER_KEY, "stable installer"),
        ("publication-plan.json", "publication plan"),
    ] {
        record_app_publication_local_path(&mut seen, local_path, purpose)?;
    }

    Ok(())
}

fn record_app_publication_local_path(
    seen: &mut BTreeMap<String, String>,
    local_path: &str,
    purpose: &str,
) -> crate::Result<()> {
    if let Some(existing) = seen.insert(local_path.to_string(), purpose.to_string()) {
        Err(crate::ReleaseError::InvalidPublicationInput {
            path: local_path.to_string(),
            reason: format!(
                "app publication local path collides between `{existing}` and `{purpose}`"
            ),
        })
    } else {
        Ok(())
    }
}

fn app_publication_plan(
    candidates: &[AppPublicationCandidate],
    versioned_app_manifest_key: &str,
    versioned_installer_key: &str,
) -> AppPublicationPlan {
    let mut immutable_uploads = candidates
        .iter()
        .flat_map(|candidate| {
            [
                AppPublicationPlanObject {
                    local_path: candidate.binary_local_path.to_string(),
                    object_key: candidate.binary_object_key.clone(),
                },
                AppPublicationPlanObject {
                    local_path: candidate.record_local_path.to_string(),
                    object_key: candidate.record_object_key.clone(),
                },
            ]
        })
        .collect::<Vec<_>>();
    immutable_uploads.extend([
        AppPublicationPlanObject {
            local_path: versioned_app_manifest_key.to_string(),
            object_key: versioned_app_manifest_key.to_string(),
        },
        AppPublicationPlanObject {
            local_path: versioned_installer_key.to_string(),
            object_key: versioned_installer_key.to_string(),
        },
    ]);

    AppPublicationPlan {
        immutable_uploads,
        stable_app_manifest: AppPublicationPlanObject {
            local_path: STABLE_APP_MANIFEST_KEY.to_string(),
            object_key: STABLE_APP_MANIFEST_KEY.to_string(),
        },
        stable_installer: AppPublicationPlanObject {
            local_path: STABLE_INSTALLER_KEY.to_string(),
            object_key: STABLE_INSTALLER_KEY.to_string(),
        },
    }
}

fn versioned_generated_key(source_run_id: &str, file_name: &str) -> String {
    format!("pv/manifests/runs/{source_run_id}/{file_name}")
}

fn validate_source_run_id(source_run_id: &str) -> crate::Result<()> {
    if source_run_id.is_empty()
        || !source_run_id.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        })
    {
        return Err(crate::ReleaseError::InvalidPublicationInput {
            path: "source-run-id".to_string(),
            reason: "source run id must be non-empty ASCII alphanumeric, dash, or underscore"
                .to_string(),
        });
    }

    Ok(())
}

fn verify_binary(record: &AppReleaseRecord, source_binary: &Utf8Path) -> crate::Result<()> {
    let (sha256, size) = digest_and_size(source_binary)?;
    if size != record.size() {
        return Err(crate::ReleaseError::SizeMismatch {
            path: source_binary.to_string(),
            expected: record.size(),
            actual: size,
        });
    }
    if sha256 != record.sha256() {
        return Err(crate::ReleaseError::ChecksumMismatch {
            path: source_binary.to_string(),
            expected: record.sha256().to_string(),
            actual: sha256,
        });
    }

    Ok(())
}

fn digest_and_size(path: &Utf8Path) -> crate::Result<(String, u64)> {
    let mut file = open_file(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 8192];
    let mut size = 0;

    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| filesystem_error(path, error))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size += read as u64;
    }

    Ok((HEXLOWER.encode(&hasher.finalize()), size))
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
    reason = "PV release tooling stages local app publication files"
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
    reason = "PV release tooling writes generated app publication plans"
)]
fn write(path: &Utf8Path, content: &str) -> crate::Result<()> {
    if let Some(parent) = path.parent() {
        create_dir_all(parent)?;
    }
    std::fs::write(path, content).map_err(|error| filesystem_error(path, error))
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV release tooling creates local app publication directories"
)]
fn create_dir_all(path: &Utf8Path) -> crate::Result<()> {
    std::fs::create_dir_all(path).map_err(|error| filesystem_error(path, error))
}

#[expect(
    clippy::disallowed_types,
    reason = "PV release tooling reads local app publication binary files"
)]
fn open_file(path: &Utf8Path) -> crate::Result<std::fs::File> {
    std::fs::File::open(path).map_err(|error| filesystem_error(path, error))
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV release tooling reads current stable app publication metadata"
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
