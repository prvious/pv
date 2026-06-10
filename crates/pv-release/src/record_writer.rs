use camino::{Utf8Path, Utf8PathBuf};
use data_encoding::HEXLOWER;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::Read;

#[derive(Clone, Debug)]
pub struct WriteReleaseRecordRequest {
    pub record: Utf8PathBuf,
    pub archive: Utf8PathBuf,
    pub resource: String,
    pub track: String,
    pub upstream_version: String,
    pub pv_build_revision: String,
    pub platform: String,
    pub object_key: String,
    pub source_url: String,
    pub source_sha256: String,
    pub recipe: String,
    pub pv_commit: String,
    pub build_run_id: String,
    pub minimum_pv_version: String,
    pub published_at: String,
    pub license_files: Vec<String>,
    pub notice_files: Vec<String>,
    pub source_inputs: Vec<SourceInputRequest>,
}

#[derive(Clone, Debug)]
pub struct SourceInputRequest {
    pub name: String,
    pub source_url: String,
    pub source_sha256: String,
}

#[derive(Serialize)]
struct ReleaseRecordJson<'a> {
    resource: &'a str,
    track: &'a str,
    upstream_version: &'a str,
    pv_build_revision: &'a str,
    artifact_version: String,
    platform: &'a str,
    object_key: &'a str,
    sha256: String,
    size: u64,
    published_at: &'a str,
    minimum_pv_version: &'a str,
    license_files: &'a [String],
    notice_files: &'a [String],
    provenance: ProvenanceJson<'a>,
}

#[derive(Serialize)]
struct ProvenanceJson<'a> {
    source_url: &'a str,
    source_sha256: &'a str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    source_inputs: Vec<SourceInputJson<'a>>,
    recipe: &'a str,
    pv_commit: &'a str,
    build_run_id: &'a str,
}

#[derive(Serialize)]
struct SourceInputJson<'a> {
    name: &'a str,
    source_url: &'a str,
    source_sha256: &'a str,
}

pub fn write_release_record(request: &WriteReleaseRecordRequest) -> crate::Result<()> {
    let (sha256, size) = digest_and_size(&request.archive)?;
    let source_inputs = request
        .source_inputs
        .iter()
        .map(|source_input| SourceInputJson {
            name: &source_input.name,
            source_url: &source_input.source_url,
            source_sha256: &source_input.source_sha256,
        })
        .collect::<Vec<_>>();
    let record = ReleaseRecordJson {
        resource: &request.resource,
        track: &request.track,
        upstream_version: &request.upstream_version,
        pv_build_revision: &request.pv_build_revision,
        artifact_version: format!("{}-{}", request.upstream_version, request.pv_build_revision),
        platform: &request.platform,
        object_key: &request.object_key,
        sha256,
        size,
        published_at: &request.published_at,
        minimum_pv_version: &request.minimum_pv_version,
        license_files: &request.license_files,
        notice_files: &request.notice_files,
        provenance: ProvenanceJson {
            source_url: &request.source_url,
            source_sha256: &request.source_sha256,
            source_inputs,
            recipe: &request.recipe,
            pv_commit: &request.pv_commit,
            build_run_id: &request.build_run_id,
        },
    };

    let mut json = serde_json::to_string_pretty(&record)
        .map_err(|error| invalid_record(&request.record, error))?;
    json.push('\n');
    crate::record::ReleaseRecord::from_json(&request.record, &json)?;

    if let Some(parent) = request
        .record
        .parent()
        .filter(|parent| !parent.as_str().is_empty())
    {
        create_dir_all(parent)?;
    }
    write(&request.record, json.as_bytes())
}

fn digest_and_size(path: &Utf8Path) -> crate::Result<(String, u64)> {
    let mut file = open_file(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 8192];
    let mut size = 0;

    loop {
        let bytes_read = file
            .read(&mut buffer)
            .map_err(|error| filesystem_error(path, error))?;
        if bytes_read == 0 {
            break;
        }
        size += bytes_read as u64;
        hasher.update(&buffer[..bytes_read]);
    }

    Ok((HEXLOWER.encode(&hasher.finalize()), size))
}

#[expect(
    clippy::disallowed_types,
    reason = "PV release tooling owns direct archive file reads for release records"
)]
fn open_file(path: &Utf8Path) -> crate::Result<std::fs::File> {
    std::fs::File::open(path).map_err(|error| filesystem_error(path, error))
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV release tooling creates generated release record directories"
)]
fn create_dir_all(path: &Utf8Path) -> crate::Result<()> {
    std::fs::create_dir_all(path).map_err(|error| filesystem_error(path, error))
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV release tooling writes generated release records"
)]
fn write(path: &Utf8Path, content: &[u8]) -> crate::Result<()> {
    std::fs::write(path, content).map_err(|error| filesystem_error(path, error))
}

fn invalid_record(path: &Utf8Path, reason: impl ToString) -> crate::ReleaseError {
    crate::ReleaseError::InvalidReleaseRecord {
        path: path.to_string(),
        reason: reason.to_string(),
    }
}

fn filesystem_error(path: &Utf8Path, error: impl ToString) -> crate::ReleaseError {
    crate::ReleaseError::Filesystem {
        path: path.to_string(),
        reason: error.to_string(),
    }
}
