use anyhow::Result;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use data_encoding::HEXLOWER;
use flate2::Compression;
use flate2::write::GzEncoder;
use insta::{assert_debug_snapshot, assert_snapshot};
use pv_release::ReleaseError;
use pv_release::publication::{PublicationRequest, prepare_publication};
use resources::ArtifactManifest;
use sha2::{Digest, Sha256};
use tar::{Builder, Header};

type ErrorSummary = (String, String, String);

#[test]
fn publication_stage_rekeys_flat_archives_and_writes_upload_plan() -> Result<()> {
    let tempdir = tempdir()?;
    let source_archives = tempdir.path().join("source-archives");
    let candidate_records = tempdir.path().join("candidate-records");
    let published_records = tempdir.path().join("published-records");
    let published_revocations = tempdir.path().join("published-revocations");
    let defaults = tempdir.path().join("default-tracks.toml");
    let stage = tempdir.path().join("stage");

    create_dir_all(&source_archives.join("downloaded-artifact"))?;
    create_dir_all(&candidate_records)?;
    create_dir_all(&published_records)?;
    create_dir_all(&published_revocations)?;
    write_file(
        &defaults,
        r#"
[[resource]]
name = "redis"
default_track = "8.2"
"#,
    )?;

    let arm64_archive = source_archives
        .join("downloaded-artifact")
        .join("redis-8.2.1-pv1-darwin-arm64.tar.gz");
    write_archive(
        &arm64_archive,
        &[
            ("redis-8.2.1-pv1-darwin-arm64/LICENSE", b"license" as &[u8]),
            ("redis-8.2.1-pv1-darwin-arm64/NOTICE", b"notice" as &[u8]),
            ("redis-8.2.1-pv1-darwin-arm64/bin/redis-server", b"redis"),
        ],
    )?;
    let (arm64_sha256, arm64_size) = archive_digest_and_size(&arm64_archive)?;
    let arm64_record_json = redis_record_json(
        "8.2.1",
        "pv1",
        "2026-06-08T12:00:00Z",
        &arm64_sha256,
        arm64_size,
        "darwin-arm64",
    );
    let arm64_record = candidate_records.join("redis-8.2.1-pv1-darwin-arm64.json");
    write_file(&arm64_record, &arm64_record_json)?;

    let amd64_archive = source_archives
        .join("downloaded-artifact")
        .join("redis-8.2.1-pv1-darwin-amd64.tar.gz");
    write_archive(
        &amd64_archive,
        &[
            ("redis-8.2.1-pv1-darwin-amd64/LICENSE", b"license" as &[u8]),
            ("redis-8.2.1-pv1-darwin-amd64/NOTICE", b"notice" as &[u8]),
            ("redis-8.2.1-pv1-darwin-amd64/bin/redis-server", b"redis"),
        ],
    )?;
    let (amd64_sha256, amd64_size) = archive_digest_and_size(&amd64_archive)?;
    let amd64_record_json = redis_record_json(
        "8.2.1",
        "pv1",
        "2026-06-08T12:00:00Z",
        &amd64_sha256,
        amd64_size,
        "darwin-amd64",
    );
    let amd64_record = candidate_records.join("redis-8.2.1-pv1-darwin-amd64.json");
    write_file(&amd64_record, &amd64_record_json)?;

    prepare_publication(&PublicationRequest {
        source_archives,
        candidate_records,
        published_records,
        published_revocations,
        defaults,
        stage: stage.clone(),
        base_url: "https://artifacts.example.test".to_string(),
        versioned_manifest_key: "manifests/runs/123456789/manifest.json".to_string(),
        stable_manifest_key: "manifest.json".to_string(),
    })?;

    assert!(path_exists(&stage.join(
        "archives/resources/redis/8.2/8.2.1-pv1/darwin-arm64/redis-8.2.1-pv1-darwin-arm64.tar.gz"
    )));
    assert!(path_exists(&stage.join(
        "archives/resources/redis/8.2/8.2.1-pv1/darwin-amd64/redis-8.2.1-pv1-darwin-amd64.tar.gz"
    )));
    assert!(path_exists(&stage.join(
        "records/resources/redis/8.2/8.2.1-pv1/darwin-arm64/redis-8.2.1-pv1-darwin-arm64.json"
    )));
    assert!(path_exists(&stage.join(
        "records/resources/redis/8.2/8.2.1-pv1/darwin-amd64/redis-8.2.1-pv1-darwin-amd64.json"
    )));
    let versioned_manifest = read_file(&stage.join("manifests/runs/123456789/manifest.json"))?;
    let stable_manifest = read_file(&stage.join("manifest.json"))?;
    ArtifactManifest::parse(&versioned_manifest)?;
    ArtifactManifest::parse(&stable_manifest)?;
    assert_eq!(versioned_manifest, stable_manifest);
    assert_eq!(
        read_file(&stage.join(
            "records/resources/redis/8.2/8.2.1-pv1/darwin-arm64/redis-8.2.1-pv1-darwin-arm64.json"
        ))?,
        arm64_record_json
    );
    assert_eq!(
        read_file(&stage.join(
            "records/resources/redis/8.2/8.2.1-pv1/darwin-amd64/redis-8.2.1-pv1-darwin-amd64.json"
        ))?,
        amd64_record_json
    );

    assert_snapshot!(read_file(&stage.join("publication-plan.json"))?);

    Ok(())
}

#[test]
fn publication_stage_merges_published_records_revocations_and_candidates() -> Result<()> {
    let fixture = PublicationFixture::new()?;
    fixture.write_candidate("8.2.2", "pv1", "2026-06-08T13:00:00Z", b"redis-new")?;
    fixture.write_candidate_with_platform(
        "8.2.2",
        "pv1",
        "2026-06-08T13:00:00Z",
        "darwin-amd64",
        b"redis-new",
    )?;
    fixture.write_published_record("8.2.1", "pv1", "2026-06-08T12:00:00Z")?;
    fixture.write_published_revocation("8.2.1-pv1", "8.2.2-pv1")?;

    prepare_publication(
        &fixture.request_with_keys("manifests/runs/123456789/manifest.json", "manifest.json"),
    )?;

    let versioned_manifest = read_file(
        &fixture
            .stage()
            .join("manifests/runs/123456789/manifest.json"),
    )?;
    let stable_manifest = read_file(&fixture.stage().join("manifest.json"))?;
    ArtifactManifest::parse(&versioned_manifest)?;
    ArtifactManifest::parse(&stable_manifest)?;
    assert_eq!(versioned_manifest, stable_manifest);

    assert_snapshot!(stable_manifest);

    Ok(())
}

#[test]
fn publication_stage_rejects_missing_archive_before_manifest_write() -> Result<()> {
    let tempdir = tempdir()?;
    let source_archives = tempdir.path().join("source-archives");
    let candidate_records = tempdir.path().join("candidate-records");
    let published_records = tempdir.path().join("published-records");
    let published_revocations = tempdir.path().join("published-revocations");
    let defaults = tempdir.path().join("default-tracks.toml");
    let stage = tempdir.path().join("stage");

    create_dir_all(&source_archives)?;
    create_dir_all(&candidate_records)?;
    create_dir_all(&published_records)?;
    create_dir_all(&published_revocations)?;
    write_file(
        &defaults,
        r#"
[[resource]]
name = "redis"
default_track = "8.2"
"#,
    )?;
    write_file(
        &candidate_records.join("redis-8.2.1-pv1-darwin-arm64.json"),
        &redis_record_json(
            "8.2.1",
            "pv1",
            "2026-06-08T12:00:00Z",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            42,
            "darwin-arm64",
        ),
    )?;

    let error = publication_error(prepare_publication(&PublicationRequest {
        source_archives,
        candidate_records,
        published_records,
        published_revocations,
        defaults,
        stage: stage.clone(),
        base_url: "https://artifacts.example.test".to_string(),
        versioned_manifest_key: "manifests/runs/123456789/manifest.json".to_string(),
        stable_manifest_key: "manifest.json".to_string(),
    }))?;

    assert_debug_snapshot!(publication_error_summary(error, tempdir.path()));
    assert!(!path_exists(&stage.join("manifest.json")));

    Ok(())
}

#[test]
fn publication_stage_rejects_reserved_stable_manifest_key_before_write() -> Result<()> {
    let fixture = PublicationFixture::new()?;
    fixture.write_valid_candidate()?;

    let reserved_stable_key = "records/manifest.json";
    let error = publication_error(prepare_publication(&fixture.request_with_keys(
        "manifests/runs/123456789/manifest.json",
        reserved_stable_key,
    )))?;

    assert_debug_snapshot!(publication_error_summary(error, fixture.root()));
    assert!(!path_exists(fixture.stage()));
    assert!(!path_exists(&fixture.stage().join(reserved_stable_key)));

    Ok(())
}

#[test]
fn publication_stage_rejects_manifest_key_collision_with_candidate_record_before_write()
-> Result<()> {
    let fixture = PublicationFixture::new()?;
    fixture.write_valid_candidate()?;

    let colliding_key =
        "records/resources/redis/8.2/8.2.1-pv1/darwin-arm64/redis-8.2.1-pv1-darwin-arm64.json";
    let error = publication_error(prepare_publication(
        &fixture.request_with_keys(colliding_key, "manifest.json"),
    ))?;

    assert_debug_snapshot!(publication_error_summary(error, fixture.root()));
    assert!(!path_exists(fixture.stage()));
    assert!(!path_exists(&fixture.stage().join(colliding_key)));

    Ok(())
}

#[test]
fn publication_stage_rejects_manifest_key_collision_with_publication_plan_before_write()
-> Result<()> {
    let fixture = PublicationFixture::new()?;
    fixture.write_valid_candidate()?;

    let error = publication_error(prepare_publication(
        &fixture.request_with_keys("publication-plan.json", "manifest.json"),
    ))?;

    assert_debug_snapshot!(publication_error_summary(error, fixture.root()));
    assert!(!path_exists(fixture.stage()));
    assert!(!path_exists(&fixture.stage().join("publication-plan.json")));

    Ok(())
}

#[test]
fn publication_stage_rejects_stable_and_versioned_manifest_key_collision_before_write() -> Result<()>
{
    let fixture = PublicationFixture::new()?;
    fixture.write_valid_candidate()?;

    let colliding_key = "manifest.json";
    let error = publication_error(prepare_publication(
        &fixture.request_with_keys(colliding_key, colliding_key),
    ))?;

    assert_debug_snapshot!(publication_error_summary(error, fixture.root()));
    assert!(!path_exists(fixture.stage()));
    assert!(!path_exists(&fixture.stage().join(colliding_key)));

    Ok(())
}

#[test]
fn publication_stage_rejects_versioned_manifest_object_key_collision_with_candidate_archive()
-> Result<()> {
    let fixture = PublicationFixture::new()?;
    fixture.write_valid_candidate()?;

    let archive_key =
        "resources/redis/8.2/8.2.1-pv1/darwin-arm64/redis-8.2.1-pv1-darwin-arm64.tar.gz";
    let error = publication_error(prepare_publication(
        &fixture.request_with_keys(archive_key, "manifest.json"),
    ))?;

    assert_debug_snapshot!(publication_error_summary(error, fixture.root()));
    assert!(!path_exists(&fixture.stage().join(archive_key)));
    assert!(!path_exists(&fixture.stage().join("publication-plan.json")));

    Ok(())
}

#[test]
fn publication_stage_rejects_stable_manifest_key_outside_manifest_entrypoint() -> Result<()> {
    let fixture = PublicationFixture::new()?;
    fixture.write_valid_candidate()?;

    let archive_key =
        "resources/redis/8.2/8.2.1-pv1/darwin-arm64/redis-8.2.1-pv1-darwin-arm64.tar.gz";
    let error = publication_error(prepare_publication(
        &fixture.request_with_keys("manifests/runs/123456789/manifest.json", archive_key),
    ))?;

    assert_debug_snapshot!(publication_error_summary(error, fixture.root()));
    assert!(!path_exists(&fixture.stage().join(archive_key)));
    assert!(!path_exists(&fixture.stage().join("publication-plan.json")));

    Ok(())
}

#[test]
fn publication_stage_rejects_public_manifest_missing_native_platform_before_write() -> Result<()> {
    let fixture = PublicationFixture::new()?;
    fixture.write_candidate_for_platform("darwin-arm64")?;

    let error = publication_error(prepare_publication(
        &fixture.request_with_keys("manifests/runs/123456789/manifest.json", "manifest.json"),
    ))?;

    assert_debug_snapshot!(publication_error_summary(error, fixture.root()));
    assert!(!path_exists(fixture.stage()));

    Ok(())
}

struct PublicationFixture {
    tempdir: camino_tempfile::Utf8TempDir,
    source_archives: camino::Utf8PathBuf,
    candidate_records: camino::Utf8PathBuf,
    published_records: camino::Utf8PathBuf,
    published_revocations: camino::Utf8PathBuf,
    defaults: camino::Utf8PathBuf,
    stage: camino::Utf8PathBuf,
}

impl PublicationFixture {
    fn new() -> Result<Self> {
        let tempdir = tempdir()?;
        let source_archives = tempdir.path().join("source-archives");
        let candidate_records = tempdir.path().join("candidate-records");
        let published_records = tempdir.path().join("published-records");
        let published_revocations = tempdir.path().join("published-revocations");
        let defaults = tempdir.path().join("default-tracks.toml");
        let stage = tempdir.path().join("stage");

        create_dir_all(&source_archives.join("downloaded-artifact"))?;
        create_dir_all(&candidate_records)?;
        create_dir_all(&published_records)?;
        create_dir_all(&published_revocations)?;
        write_file(
            &defaults,
            r#"
[[resource]]
name = "redis"
default_track = "8.2"
"#,
        )?;

        Ok(Self {
            tempdir,
            source_archives,
            candidate_records,
            published_records,
            published_revocations,
            defaults,
            stage,
        })
    }

    fn root(&self) -> &Utf8Path {
        self.tempdir.path()
    }

    fn stage(&self) -> &Utf8Path {
        &self.stage
    }

    fn request_with_keys(
        &self,
        versioned_manifest_key: &str,
        stable_manifest_key: &str,
    ) -> PublicationRequest {
        PublicationRequest {
            source_archives: self.source_archives.clone(),
            candidate_records: self.candidate_records.clone(),
            published_records: self.published_records.clone(),
            published_revocations: self.published_revocations.clone(),
            defaults: self.defaults.clone(),
            stage: self.stage.clone(),
            base_url: "https://artifacts.example.test".to_string(),
            versioned_manifest_key: versioned_manifest_key.to_string(),
            stable_manifest_key: stable_manifest_key.to_string(),
        }
    }

    fn write_valid_candidate(&self) -> Result<()> {
        self.write_candidate_for_platform("darwin-arm64")?;
        self.write_candidate_for_platform("darwin-amd64")
    }

    fn write_candidate_for_platform(&self, platform: &str) -> Result<()> {
        self.write_candidate_with_platform(
            "8.2.1",
            "pv1",
            "2026-06-08T12:00:00Z",
            platform,
            b"redis",
        )
    }

    fn write_candidate(
        &self,
        upstream_version: &str,
        pv_build_revision: &str,
        published_at: &str,
        payload: &[u8],
    ) -> Result<()> {
        self.write_candidate_with_platform(
            upstream_version,
            pv_build_revision,
            published_at,
            "darwin-arm64",
            payload,
        )
    }

    fn write_candidate_with_platform(
        &self,
        upstream_version: &str,
        pv_build_revision: &str,
        published_at: &str,
        platform: &str,
        payload: &[u8],
    ) -> Result<()> {
        let artifact_version = artifact_version(upstream_version, pv_build_revision);
        let artifact_basename = redis_artifact_basename(&artifact_version, platform);
        let archive = self
            .source_archives
            .join("downloaded-artifact")
            .join(format!("{artifact_basename}.tar.gz"));
        write_archive(
            &archive,
            &[
                (&format!("{artifact_basename}/LICENSE"), b"license" as &[u8]),
                (&format!("{artifact_basename}/NOTICE"), b"notice" as &[u8]),
                (&format!("{artifact_basename}/bin/redis-server"), payload),
            ],
        )?;
        let (sha256, size) = archive_digest_and_size(&archive)?;
        write_file(
            &self
                .candidate_records
                .join(format!("{artifact_basename}.json")),
            &redis_record_json(
                upstream_version,
                pv_build_revision,
                published_at,
                &sha256,
                size,
                platform,
            ),
        )
    }

    fn write_published_record(
        &self,
        upstream_version: &str,
        pv_build_revision: &str,
        published_at: &str,
    ) -> Result<()> {
        let artifact_version = artifact_version(upstream_version, pv_build_revision);
        let platform = "darwin-arm64";
        let artifact_basename = redis_artifact_basename(&artifact_version, platform);
        let record = self
            .published_records
            .join("resources/redis/8.2")
            .join(&artifact_version)
            .join(platform)
            .join(format!("{artifact_basename}.json"));
        let archive = self
            .source_archives
            .join("downloaded-artifact")
            .join(format!("{artifact_basename}.tar.gz"));
        write_archive(
            &archive,
            &[
                (&format!("{artifact_basename}/LICENSE"), b"license" as &[u8]),
                (&format!("{artifact_basename}/NOTICE"), b"notice" as &[u8]),
                (
                    &format!("{artifact_basename}/bin/redis-server"),
                    b"redis-old",
                ),
            ],
        )?;
        let (sha256, size) = archive_digest_and_size(&archive)?;
        write_file(
            &record,
            &redis_record_json(
                upstream_version,
                pv_build_revision,
                published_at,
                &sha256,
                size,
                platform,
            ),
        )
    }

    fn write_published_revocation(
        &self,
        revoked_artifact_version: &str,
        replacement_artifact_version: &str,
    ) -> Result<()> {
        write_file(
            &self.published_revocations.join(format!(
                "redis-{revoked_artifact_version}-darwin-arm64.json"
            )),
            &redis_revocation_json(revoked_artifact_version, replacement_artifact_version),
        )
    }
}

fn publication_error(result: pv_release::Result<()>) -> Result<ReleaseError> {
    match result {
        Ok(()) => Err(anyhow::anyhow!(
            "publication preparation succeeded unexpectedly"
        )),
        Err(error) => Ok(error),
    }
}

fn publication_error_summary(error: ReleaseError, root: &Utf8Path) -> ErrorSummary {
    match error {
        ReleaseError::InvalidPublicationInput { path, reason } => (
            "InvalidPublicationInput".to_string(),
            relative_path(Utf8Path::new(&path), root),
            reason,
        ),
        ReleaseError::ImmutablePublicationObjectExists { key } => (
            "ImmutablePublicationObjectExists".to_string(),
            key,
            String::new(),
        ),
        ReleaseError::GeneratedManifestInvalid { reason } => (
            "GeneratedManifestInvalid".to_string(),
            String::new(),
            reason,
        ),
        error => ("Other".to_string(), String::new(), error.to_string()),
    }
}

fn relative_path(path: &Utf8Path, root: &Utf8Path) -> String {
    match path.strip_prefix(root) {
        Ok(path) => path.to_string(),
        Err(_error) => path.to_string(),
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests create local publication fixture directories"
)]
fn create_dir_all(path: &Utf8Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests write local publication fixtures"
)]
fn write_file(path: &Utf8Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests read generated publication outputs"
)]
fn read_file(path: &Utf8Path) -> Result<String> {
    Ok(std::fs::read_to_string(path)?)
}

fn path_exists(path: &Utf8Path) -> bool {
    path.exists()
}

#[expect(
    clippy::disallowed_types,
    reason = "release tooling tests create fixture archives directly"
)]
fn write_archive(path: &Utf8Path, entries: &[(&str, &[u8])]) -> Result<()> {
    let file = std::fs::File::create(path)?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut builder = Builder::new(encoder);

    for (path, content) in entries {
        let mut header = Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append_data(&mut header, path, *content)?;
    }

    let encoder = builder.into_inner()?;
    encoder.finish()?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests read local fixture archives to seed matching release records"
)]
fn archive_digest_and_size(path: &Utf8Path) -> Result<(String, u64)> {
    let bytes = std::fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);

    Ok((HEXLOWER.encode(&hasher.finalize()), bytes.len() as u64))
}

fn artifact_version(upstream_version: &str, pv_build_revision: &str) -> String {
    format!("{upstream_version}-{pv_build_revision}")
}

fn redis_artifact_basename(artifact_version: &str, platform: &str) -> String {
    format!("redis-{artifact_version}-{platform}")
}

fn redis_record_json(
    upstream_version: &str,
    pv_build_revision: &str,
    published_at: &str,
    sha256: &str,
    size: u64,
    platform: &str,
) -> String {
    let artifact_version = artifact_version(upstream_version, pv_build_revision);
    let artifact_basename = redis_artifact_basename(&artifact_version, platform);
    format!(
        r#"{{
  "resource": "redis",
  "track": "8.2",
  "upstream_version": "{upstream_version}",
  "pv_build_revision": "{pv_build_revision}",
  "artifact_version": "{artifact_version}",
  "platform": "{platform}",
  "object_key": "resources/redis/8.2/{artifact_version}/{platform}/{artifact_basename}.tar.gz",
  "sha256": "{sha256}",
  "size": {size},
  "published_at": "{published_at}",
  "minimum_pv_version": "0.1.0",
  "license_files": ["LICENSE"],
  "notice_files": ["NOTICE"],
  "provenance": {{
    "source_url": "https://download.redis.io/releases/redis-{upstream_version}.tar.gz",
    "source_sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "recipe": "release/artifacts/recipes/redis/build.sh",
    "pv_commit": "0123456789abcdef0123456789abcdef01234567",
    "build_run_id": "123456789"
  }}
}}"#,
    )
}

fn redis_revocation_json(
    revoked_artifact_version: &str,
    replacement_artifact_version: &str,
) -> String {
    format!(
        r#"{{
  "resource": "redis",
  "track": "8.2",
  "artifact_version": "{revoked_artifact_version}",
  "platform": "darwin-arm64",
  "reason": "bad package",
  "revoked_at": "2026-06-08T14:00:00Z",
  "replacement_artifact_version": "{replacement_artifact_version}"
}}"#,
    )
}
