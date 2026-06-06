use anyhow::Result;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::{assert_debug_snapshot, assert_snapshot};
use pv_release::ReleaseError;
use pv_release::manifest::generate_manifest_file;
use resources::ArtifactManifest;
use serde_json::Value;

#[test]
fn manifest_generator_merges_release_and_revocation_records() -> Result<()> {
    let tempdir = tempdir()?;
    let records_dir = tempdir.path().join("records");
    let revocations_dir = tempdir.path().join("revocations");
    let output = tempdir.path().join("dist/manifest.json");

    create_dir_all(&records_dir)?;
    create_dir_all(&revocations_dir)?;
    write_file(
        &records_dir.join("redis-7.2.5-pv1-darwin-arm64.json"),
        REDIS_7_2_5_ARM64,
    )?;
    write_file(
        &records_dir.join("redis-7.2.6-pv1-darwin-arm64.json"),
        REDIS_7_2_6_ARM64,
    )?;
    write_file(
        &revocations_dir.join("redis-7.2.5-pv1-darwin-arm64.json"),
        REDIS_7_2_5_REVOCATION,
    )?;

    generate_manifest_file(
        &records_dir,
        &revocations_dir,
        &output,
        "https://artifacts.example.test",
    )?;
    let manifest_json = read_file(&output)?;
    ArtifactManifest::parse(&manifest_json)?;
    assert_manifest_includes_release_operation_metadata(&manifest_json)?;

    assert_snapshot!(manifest_json);

    Ok(())
}

#[test]
fn manifest_generator_rejects_revocation_for_missing_artifact() -> Result<()> {
    let tempdir = tempdir()?;
    let records_dir = tempdir.path().join("records");
    let revocations_dir = tempdir.path().join("revocations");
    let output = tempdir.path().join("dist/manifest.json");

    create_dir_all(&records_dir)?;
    create_dir_all(&revocations_dir)?;
    write_file(
        &records_dir.join("redis-7.2.6-pv1-darwin-arm64.json"),
        REDIS_7_2_6_ARM64,
    )?;
    write_file(
        &revocations_dir.join("redis-7.2.5-pv1-darwin-arm64.json"),
        REDIS_7_2_5_REVOCATION,
    )?;

    let error = manifest_error(generate_manifest_file(
        &records_dir,
        &revocations_dir,
        &output,
        "https://artifacts.example.test",
    ))?;

    assert!(matches!(
        error,
        ReleaseError::RevocationTargetMissing { .. }
    ));
    assert_debug_snapshot!(error);

    Ok(())
}

#[test]
fn manifest_generator_rejects_multi_track_resource_without_default_metadata() -> Result<()> {
    let tempdir = tempdir()?;
    let records_dir = tempdir.path().join("records");
    let revocations_dir = tempdir.path().join("revocations");
    let output = tempdir.path().join("dist/manifest.json");

    create_dir_all(&records_dir)?;
    create_dir_all(&revocations_dir)?;
    write_file(
        &records_dir.join("redis-7.2.6-pv1-darwin-arm64.json"),
        REDIS_7_2_6_ARM64,
    )?;
    write_file(
        &records_dir.join("redis-8.0.0-pv1-darwin-arm64.json"),
        REDIS_8_0_0_ARM64,
    )?;

    let error = manifest_error(generate_manifest_file(
        &records_dir,
        &revocations_dir,
        &output,
        "https://artifacts.example.test",
    ))?;

    assert!(matches!(
        error,
        ReleaseError::GeneratedManifestInvalid { .. }
    ));
    assert!(!path_exists(&output));
    assert_debug_snapshot!(error);

    Ok(())
}

fn manifest_error(result: pv_release::Result<()>) -> Result<ReleaseError> {
    match result {
        Ok(()) => Err(anyhow::anyhow!(
            "manifest generation succeeded unexpectedly"
        )),
        Err(error) => Ok(error),
    }
}

fn assert_manifest_includes_release_operation_metadata(manifest_json: &str) -> Result<()> {
    let manifest: Value = serde_json::from_str(manifest_json)?;
    let revoked = artifact_by_version(&manifest, "7.2.5-pv1")?;
    let active = artifact_by_version(&manifest, "7.2.6-pv1")?;

    assert_eq!(
        string_field(provenance(revoked)?, "source_url")?,
        "https://download.redis.io/releases/redis-7.2.5.tar.gz"
    );
    assert_eq!(
        string_field(provenance(revoked)?, "source_sha256")?,
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    );
    assert_eq!(
        string_field(provenance(revoked)?, "recipe")?,
        "release/artifacts/recipes/redis/build.sh"
    );
    assert_eq!(
        string_field(provenance(revoked)?, "pv_commit")?,
        "0123456789abcdef0123456789abcdef01234567"
    );
    assert_eq!(
        string_field(provenance(revoked)?, "build_run_id")?,
        "local-test"
    );
    assert_eq!(string_field(revoked, "revoked_at")?, "2026-06-06T14:00:00Z");
    assert_eq!(
        string_field(revoked, "replacement_artifact_version")?,
        "7.2.6-pv1"
    );
    assert_eq!(
        string_field(provenance(active)?, "source_url")?,
        "https://download.redis.io/releases/redis-7.2.6.tar.gz"
    );

    Ok(())
}

fn artifact_by_version<'a>(manifest: &'a Value, version: &str) -> Result<&'a Value> {
    let Some(resources) = manifest.get("resources").and_then(Value::as_array) else {
        return Err(anyhow::anyhow!("manifest resources must be an array"));
    };

    for resource in resources {
        let Some(tracks) = resource.get("tracks").and_then(Value::as_array) else {
            return Err(anyhow::anyhow!("manifest tracks must be an array"));
        };
        for track in tracks {
            let Some(artifacts) = track.get("artifacts").and_then(Value::as_array) else {
                return Err(anyhow::anyhow!("manifest artifacts must be an array"));
            };
            for artifact in artifacts {
                if string_field(artifact, "artifact_version")? == version {
                    return Ok(artifact);
                }
            }
        }
    }

    Err(anyhow::anyhow!("artifact version `{version}` not found"))
}

fn provenance(artifact: &Value) -> Result<&Value> {
    artifact
        .get("provenance")
        .ok_or_else(|| anyhow::anyhow!("artifact must include provenance"))
}

fn string_field<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("field `{field}` must be a string"))
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests create local fixture directories"
)]
fn create_dir_all(path: &Utf8Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests write local fixture metadata records"
)]
fn write_file(path: &Utf8Path, content: &str) -> Result<()> {
    std::fs::write(path, content)?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests read generated manifest files"
)]
fn read_file(path: &Utf8Path) -> Result<String> {
    Ok(std::fs::read_to_string(path)?)
}

fn path_exists(path: &Utf8Path) -> bool {
    path.exists()
}

const REDIS_7_2_5_ARM64: &str = r#"{
  "resource": "redis",
  "track": "7.2",
  "upstream_version": "7.2.5",
  "pv_build_revision": "pv1",
  "artifact_version": "7.2.5-pv1",
  "platform": "darwin-arm64",
  "object_key": "resources/redis/7.2/7.2.5-pv1/darwin-arm64/redis-7.2.5-pv1-darwin-arm64.tar.gz",
  "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "size": 42,
  "published_at": "2026-06-06T12:00:00Z",
  "minimum_pv_version": "0.1.0",
  "license_files": ["LICENSE"],
  "notice_files": ["NOTICE"],
  "provenance": {
    "source_url": "https://download.redis.io/releases/redis-7.2.5.tar.gz",
    "source_sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "recipe": "release/artifacts/recipes/redis/build.sh",
    "pv_commit": "0123456789abcdef0123456789abcdef01234567",
    "build_run_id": "local-test"
  }
}"#;

const REDIS_7_2_6_ARM64: &str = r#"{
  "resource": "redis",
  "track": "7.2",
  "upstream_version": "7.2.6",
  "pv_build_revision": "pv1",
  "artifact_version": "7.2.6-pv1",
  "platform": "darwin-arm64",
  "object_key": "resources/redis/7.2/7.2.6-pv1/darwin-arm64/redis-7.2.6-pv1-darwin-arm64.tar.gz",
  "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
  "size": 84,
  "published_at": "2026-06-06T13:00:00Z",
  "minimum_pv_version": "0.1.0",
  "license_files": ["LICENSE"],
  "notice_files": ["NOTICE"],
  "provenance": {
    "source_url": "https://download.redis.io/releases/redis-7.2.6.tar.gz",
    "source_sha256": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
    "recipe": "release/artifacts/recipes/redis/build.sh",
    "pv_commit": "0123456789abcdef0123456789abcdef01234567",
    "build_run_id": "local-test"
  }
}"#;

const REDIS_8_0_0_ARM64: &str = r#"{
  "resource": "redis",
  "track": "8.0",
  "upstream_version": "8.0.0",
  "pv_build_revision": "pv1",
  "artifact_version": "8.0.0-pv1",
  "platform": "darwin-arm64",
  "object_key": "resources/redis/8.0/8.0.0-pv1/darwin-arm64/redis-8.0.0-pv1-darwin-arm64.tar.gz",
  "sha256": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
  "size": 168,
  "published_at": "2026-06-06T14:00:00Z",
  "minimum_pv_version": "0.1.0",
  "license_files": ["LICENSE"],
  "notice_files": ["NOTICE"],
  "provenance": {
    "source_url": "https://download.redis.io/releases/redis-8.0.0.tar.gz",
    "source_sha256": "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    "recipe": "release/artifacts/recipes/redis/build.sh",
    "pv_commit": "0123456789abcdef0123456789abcdef01234567",
    "build_run_id": "local-test"
  }
}"#;

const REDIS_7_2_5_REVOCATION: &str = r#"{
  "resource": "redis",
  "track": "7.2",
  "artifact_version": "7.2.5-pv1",
  "platform": "darwin-arm64",
  "reason": "security issue",
  "revoked_at": "2026-06-06T14:00:00Z",
  "replacement_artifact_version": "7.2.6-pv1"
}"#;
