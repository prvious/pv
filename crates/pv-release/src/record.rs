use camino::{Utf8Path, Utf8PathBuf};
use resources::{
    ArtifactPlatform, ArtifactVersion, PublishedAt, PvVersion, ResourceName, ResourcesError,
    Sha256Digest, TrackName,
};
use serde::Deserialize;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use url::Url;

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
    php_extensions: Vec<PhpExtensionRecord>,
    provenance: Provenance,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PhpExtensionRecord {
    name: String,
    load_kind: String,
    path: String,
}

#[derive(Clone)]
pub struct RevocationRecord {
    path: Utf8PathBuf,
    resource: ResourceName,
    track: TrackName,
    artifact_version: ArtifactVersion,
    platform: ArtifactPlatform,
    reason: String,
    revoked_at_raw: String,
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
#[serde(deny_unknown_fields)]
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
    #[serde(default)]
    php_extensions: Vec<PhpExtensionRecord>,
    provenance: Provenance,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    source_url: String,
    source_sha256: String,
    #[serde(default)]
    source_inputs: Vec<SourceInput>,
    recipe: String,
    pv_commit: String,
    build_run_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceInput {
    name: String,
    source_url: String,
    source_sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
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
        validate_relative_path(path, "object_key", &raw.object_key)?;
        validate_object_key_layout(path, &raw)?;
        validate_php_extensions(path, &raw.php_extensions)?;
        raw.provenance.validate(path)?;

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
            php_extensions: raw.php_extensions,
            provenance: raw.provenance,
        })
    }

    pub fn identity(&self) -> ArtifactIdentity {
        self.identity.clone()
    }

    pub fn resource(&self) -> &ResourceName {
        &self.identity.resource
    }

    pub fn track(&self) -> &TrackName {
        &self.identity.track
    }

    pub fn artifact_version(&self) -> &ArtifactVersion {
        &self.artifact_version
    }

    pub fn platform(&self) -> ArtifactPlatform {
        self.identity.platform
    }

    pub fn object_key(&self) -> &str {
        &self.object_key
    }

    pub fn sha256(&self) -> &Sha256Digest {
        &self.sha256
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn published_at_raw(&self) -> &str {
        &self.published_at_raw
    }

    pub fn minimum_pv_version(&self) -> &PvVersion {
        &self.minimum_pv_version
    }

    pub fn upstream_version(&self) -> &str {
        &self.identity.upstream_version
    }

    pub fn pv_build_revision(&self) -> &str {
        &self.identity.pv_build_revision
    }

    pub fn provenance(&self) -> &Provenance {
        &self.provenance
    }

    pub fn license_files(&self) -> &[String] {
        &self.license_files
    }

    pub fn notice_files(&self) -> &[String] {
        &self.notice_files
    }

    pub fn php_extensions(&self) -> &[PhpExtensionRecord] {
        &self.php_extensions
    }

    pub fn verify_archive(
        &self,
        validation: &crate::archive::ArchiveValidation,
    ) -> crate::Result<()> {
        if self.sha256.as_str() != validation.sha256() {
            return Err(crate::ReleaseError::ChecksumMismatch {
                path: validation.archive_path().to_string(),
                expected: self.sha256.as_str().to_string(),
                actual: validation.sha256().to_string(),
            });
        }

        if self.size != validation.size() {
            return Err(crate::ReleaseError::SizeMismatch {
                path: validation.archive_path().to_string(),
                expected: self.size,
                actual: validation.size(),
            });
        }

        Ok(())
    }
}

impl PhpExtensionRecord {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn load_kind(&self) -> &str {
        &self.load_kind
    }

    pub fn path(&self) -> &str {
        &self.path
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
            revoked_at_raw: raw.revoked_at.clone(),
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

    pub fn target_key(&self) -> String {
        format!(
            "{}:{}:{}:{}",
            self.resource, self.track, self.artifact_version, self.platform
        )
    }

    pub fn replacement_key(&self) -> Option<String> {
        self.replacement_artifact_version
            .as_ref()
            .map(|replacement| {
                format!(
                    "{}:{}:{}:{}",
                    self.resource, self.track, replacement, self.platform
                )
            })
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }

    pub fn revoked_at(&self) -> &str {
        &self.revoked_at_raw
    }

    pub fn replacement_artifact_version(&self) -> Option<&ArtifactVersion> {
        self.replacement_artifact_version.as_ref()
    }

    fn revocation_metadata(&self) -> RevocationMetadata {
        RevocationMetadata {
            reason: self.reason.clone(),
            revoked_at: self.revoked_at.clone(),
            replacement_artifact_version: self.replacement_artifact_version.clone(),
        }
    }
}

impl Provenance {
    fn validate(&self, path: &Utf8Path) -> crate::Result<()> {
        validate_https_url(path, "source_url", &self.source_url)?;
        Sha256Digest::new(self.source_sha256.clone())
            .map_err(|error| invalid_release_identity(path, "source_sha256", error))?;
        let mut source_input_names = BTreeSet::new();
        for source_input in &self.source_inputs {
            source_input.validate(path)?;
            if !source_input_names.insert(source_input.name()) {
                return Err(invalid_release(
                    path,
                    format!("duplicate source input `{}`", source_input.name()),
                ));
            }
        }
        validate_relative_path(path, "recipe", &self.recipe)?;
        validate_commit(path, &self.pv_commit)?;
        require_non_empty_release(path, "build_run_id", &self.build_run_id)?;

        Ok(())
    }

    pub fn source_url(&self) -> &str {
        &self.source_url
    }

    pub fn source_sha256(&self) -> &str {
        &self.source_sha256
    }

    pub fn source_inputs(&self) -> &[SourceInput] {
        &self.source_inputs
    }

    pub fn recipe(&self) -> &str {
        &self.recipe
    }

    pub fn pv_commit(&self) -> &str {
        &self.pv_commit
    }

    pub fn build_run_id(&self) -> &str {
        &self.build_run_id
    }
}

impl SourceInput {
    fn validate(&self, path: &Utf8Path) -> crate::Result<()> {
        validate_source_input_name(path, &self.name)?;
        validate_https_url(path, "source_inputs.source_url", &self.source_url)?;
        Sha256Digest::new(self.source_sha256.clone()).map_err(|error| {
            invalid_release_identity(path, "source_inputs.source_sha256", error)
        })?;

        Ok(())
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn source_url(&self) -> &str {
        &self.source_url
    }

    pub fn source_sha256(&self) -> &str {
        &self.source_sha256
    }
}

impl fmt::Debug for RevocationRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RevocationRecord")
            .field("path", &self.path)
            .field("resource", &self.resource)
            .field("track", &self.track)
            .field("artifact_version", &self.artifact_version)
            .field("platform", &self.platform)
            .field("reason", &self.reason)
            .field("revoked_at", &self.revoked_at)
            .field(
                "replacement_artifact_version",
                &self.replacement_artifact_version,
            )
            .finish()
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
            Entry::Occupied(entry) => {
                return Err(crate::ReleaseError::DuplicateRevocation {
                    identity: entry.key().clone(),
                });
            }
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
        if !relative_path_is_valid(value) {
            return Err(invalid_release(
                path,
                format!("{field} contains invalid relative path `{value}`"),
            ));
        }
    }

    Ok(())
}

fn validate_relative_path(path: &Utf8Path, field: &str, value: &str) -> crate::Result<()> {
    if relative_path_is_valid(value) {
        Ok(())
    } else {
        Err(invalid_release(
            path,
            format!("{field} contains invalid relative path `{value}`"),
        ))
    }
}

fn validate_php_extensions(
    path: &Utf8Path,
    extensions: &[PhpExtensionRecord],
) -> crate::Result<()> {
    let mut names = BTreeSet::new();
    for extension in extensions {
        validate_php_extension_name(path, &extension.name)?;
        validate_php_extension_load_kind(path, &extension.load_kind)?;
        validate_relative_path(path, "php_extensions.path", &extension.path)?;
        if !names.insert(extension.name.as_str()) {
            return Err(invalid_release(
                path,
                format!(
                    "php_extensions contains duplicate extension `{}`",
                    extension.name
                ),
            ));
        }
    }

    Ok(())
}

fn validate_php_extension_name(path: &Utf8Path, name: &str) -> crate::Result<()> {
    let valid = !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    if valid {
        return Ok(());
    }

    Err(invalid_release(
        path,
        format!("php_extensions contains invalid extension `{name}`"),
    ))
}

fn validate_php_extension_load_kind(path: &Utf8Path, load_kind: &str) -> crate::Result<()> {
    if matches!(load_kind, "extension" | "zend_extension") {
        return Ok(());
    }

    Err(invalid_release(
        path,
        format!("php_extensions contains invalid load kind `{load_kind}`"),
    ))
}

fn validate_object_key_layout(path: &Utf8Path, raw: &RawReleaseRecord) -> crate::Result<()> {
    let expected = format!(
        "resources/{}/{}/{}/{}/{}-{}-{}.tar.gz",
        raw.resource,
        raw.track,
        raw.artifact_version,
        raw.platform,
        raw.resource,
        raw.artifact_version,
        raw.platform
    );

    if raw.object_key == expected {
        Ok(())
    } else {
        Err(invalid_release(
            path,
            format!("object_key must be `{expected}`"),
        ))
    }
}

fn relative_path_is_valid(value: &str) -> bool {
    let candidate = Utf8Path::new(value);
    !candidate.is_absolute()
        && !value.is_empty()
        && !value.contains('\\')
        && !value.split('/').any(str::is_empty)
        && !candidate
            .components()
            .any(|component| matches!(component.as_str(), "." | ".."))
}

fn validate_https_url(path: &Utf8Path, field: &str, value: &str) -> crate::Result<()> {
    let value = require_non_empty_release(path, field, value)?;
    if value.contains('\\') {
        return Err(invalid_release(
            path,
            format!("{field} must be an https URL with a host"),
        ));
    }

    let parsed = Url::parse(value).map_err(|_error| {
        invalid_release(path, format!("{field} must be an https URL with a host"))
    })?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(invalid_release(
            path,
            format!("{field} must be an https URL with a host"),
        ));
    }

    Ok(())
}

fn validate_commit(path: &Utf8Path, value: &str) -> crate::Result<()> {
    if value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(invalid_release(
            path,
            "pv_commit must be a 40-character hex commit",
        ))
    }
}

fn validate_source_input_name(path: &Utf8Path, value: &str) -> crate::Result<()> {
    let value = require_non_empty_release(path, "source_inputs.name", value)?;
    if value
        .bytes()
        .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_'))
    {
        Ok(())
    } else {
        Err(invalid_release(
            path,
            format!("source_inputs.name contains invalid value `{value}`"),
        ))
    }
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
