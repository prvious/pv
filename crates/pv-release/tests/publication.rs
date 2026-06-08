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

    let archive = source_archives
        .join("downloaded-artifact")
        .join("redis-8.2.1-pv1-darwin-arm64.tar.gz");
    write_archive(
        &archive,
        &[
            ("redis-8.2.1-pv1-darwin-arm64/LICENSE", b"license" as &[u8]),
            ("redis-8.2.1-pv1-darwin-arm64/NOTICE", b"notice" as &[u8]),
            ("redis-8.2.1-pv1-darwin-arm64/bin/redis-server", b"redis"),
        ],
    )?;
    let (sha256, size) = archive_digest_and_size(&archive)?;
    let record_json = redis_record_json(&sha256, size);
    let record = candidate_records.join("redis-8.2.1-pv1-darwin-arm64.json");
    write_file(&record, &record_json)?;

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
        "records/resources/redis/8.2/8.2.1-pv1/darwin-arm64/redis-8.2.1-pv1-darwin-arm64.json"
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
        record_json
    );

    assert_snapshot!(read_file(&stage.join("publication-plan.json"))?);

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
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            42,
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
fn publication_stage_rejects_manifest_key_collision_with_candidate_record_before_write()
-> Result<()> {
    let fixture = PublicationFixture::new()?;
    fixture.write_valid_candidate()?;

    let colliding_key =
        "records/resources/redis/8.2/8.2.1-pv1/darwin-arm64/redis-8.2.1-pv1-darwin-arm64.json";
    let error = publication_error(prepare_publication(
        &fixture.request_with_keys("manifests/runs/123456789/manifest.json", colliding_key),
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

    let error = publication_error(prepare_publication(&fixture.request_with_keys(
        "manifests/runs/123456789/manifest.json",
        "publication-plan.json",
    )))?;

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

    let colliding_key = "manifests/runs/123456789/manifest.json";
    let error = publication_error(prepare_publication(
        &fixture.request_with_keys(colliding_key, colliding_key),
    ))?;

    assert_debug_snapshot!(publication_error_summary(error, fixture.root()));
    assert!(!path_exists(fixture.stage()));
    assert!(!path_exists(&fixture.stage().join(colliding_key)));

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
        let archive = self
            .source_archives
            .join("downloaded-artifact")
            .join("redis-8.2.1-pv1-darwin-arm64.tar.gz");
        write_archive(
            &archive,
            &[
                ("redis-8.2.1-pv1-darwin-arm64/LICENSE", b"license" as &[u8]),
                ("redis-8.2.1-pv1-darwin-arm64/NOTICE", b"notice" as &[u8]),
                ("redis-8.2.1-pv1-darwin-arm64/bin/redis-server", b"redis"),
            ],
        )?;
        let (sha256, size) = archive_digest_and_size(&archive)?;
        write_file(
            &self
                .candidate_records
                .join("redis-8.2.1-pv1-darwin-arm64.json"),
            &redis_record_json(&sha256, size),
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

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests check generated publication outputs"
)]
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

fn redis_record_json(sha256: &str, size: u64) -> String {
    format!(
        r#"{{
  "resource": "redis",
  "track": "8.2",
  "upstream_version": "8.2.1",
  "pv_build_revision": "pv1",
  "artifact_version": "8.2.1-pv1",
  "platform": "darwin-arm64",
  "object_key": "resources/redis/8.2/8.2.1-pv1/darwin-arm64/redis-8.2.1-pv1-darwin-arm64.tar.gz",
  "sha256": "{sha256}",
  "size": {size},
  "published_at": "2026-06-08T12:00:00Z",
  "minimum_pv_version": "0.1.0",
  "license_files": ["LICENSE"],
  "notice_files": ["NOTICE"],
  "provenance": {{
    "source_url": "https://download.redis.io/releases/redis-8.2.1.tar.gz",
    "source_sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "recipe": "release/artifacts/recipes/redis/build.sh",
    "pv_commit": "0123456789abcdef0123456789abcdef01234567",
    "build_run_id": "123456789"
  }}
}}"#,
    )
}
