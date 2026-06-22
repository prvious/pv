use anyhow::Result;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::assert_debug_snapshot;
use pv_release::record::{
    ArtifactIdentity, ReleaseRecord, RevocationRecord, load_release_records,
    load_revocation_records,
};
use pv_release::{ReleaseError, Result as ReleaseResult};

#[test]
fn release_records_parse_identity_and_required_fields() -> Result<()> {
    let record = ReleaseRecord::from_json(Utf8Path::new("redis-7.2.5.json"), VALID_RELEASE_RECORD)?;

    assert_debug_snapshot!(record);
    assert_eq!(
        record.identity(),
        ArtifactIdentity::new("redis", "7.2", "7.2.5", "pv1", "darwin-arm64")?
    );

    Ok(())
}

#[test]
fn release_records_reject_identity_mismatches_and_missing_license_metadata() -> Result<()> {
    let mismatched_artifact_version = VALID_RELEASE_RECORD.replace(
        "\"artifact_version\": \"7.2.5-pv1\"",
        "\"artifact_version\": \"7.2.5-pv2\"",
    );
    let missing_license = VALID_RELEASE_RECORD.replace(
        "\"license_files\": [\"LICENSE\"],",
        "\"license_files\": [],",
    );

    assert_debug_snapshot!(ReleaseRecord::from_json(
        Utf8Path::new("mismatch.json"),
        &mismatched_artifact_version,
    ));
    assert_debug_snapshot!(ReleaseRecord::from_json(
        Utf8Path::new("missing-license.json"),
        &missing_license,
    ));

    Ok(())
}

#[test]
fn release_records_reject_invalid_object_key_and_provenance_metadata() -> Result<()> {
    let absolute_object_key = VALID_RELEASE_RECORD.replace(
        "\"object_key\": \"resources/redis/7.2/7.2.5-pv1/darwin-arm64/redis-7.2.5-pv1-darwin-arm64.tar.gz\"",
        "\"object_key\": \"/resources/redis.tar.gz\"",
    );
    let parent_object_key = VALID_RELEASE_RECORD.replace(
        "\"object_key\": \"resources/redis/7.2/7.2.5-pv1/darwin-arm64/redis-7.2.5-pv1-darwin-arm64.tar.gz\"",
        "\"object_key\": \"resources/../redis.tar.gz\"",
    );
    let mismatched_object_key = VALID_RELEASE_RECORD.replace(
        "\"object_key\": \"resources/redis/7.2/7.2.5-pv1/darwin-arm64/redis-7.2.5-pv1-darwin-arm64.tar.gz\"",
        "\"object_key\": \"resources/mysql/8.0/7.2.5-pv1/darwin-arm64/redis-7.2.5-pv1-darwin-arm64.tar.gz\"",
    );
    let invalid_source_url = VALID_RELEASE_RECORD.replace(
        "https://download.redis.io/releases/redis-7.2.5.tar.gz",
        "not a url",
    );
    let invalid_source_sha256 = VALID_RELEASE_RECORD.replace(
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "bad",
    );
    let invalid_recipe =
        VALID_RELEASE_RECORD.replace("release/artifacts/recipes/redis/build.sh", "../build.sh");
    let invalid_commit =
        VALID_RELEASE_RECORD.replace("0123456789abcdef0123456789abcdef01234567", "not-a-commit");
    let empty_build_run =
        VALID_RELEASE_RECORD.replace("\"build_run_id\": \"local-test\"", "\"build_run_id\": \"\"");

    let errors = [
        ("absolute_object_key", absolute_object_key),
        ("parent_object_key", parent_object_key),
        ("mismatched_object_key", mismatched_object_key),
        ("invalid_source_url", invalid_source_url),
        ("invalid_source_sha256", invalid_source_sha256),
        ("invalid_recipe", invalid_recipe),
        ("invalid_commit", invalid_commit),
        ("empty_build_run", empty_build_run),
    ]
    .into_iter()
    .map(|(name, json)| {
        Ok((
            name,
            release_record_error(ReleaseRecord::from_json(
                Utf8Path::new("invalid.json"),
                &json,
            ))?,
        ))
    })
    .collect::<Result<Vec<_>>>()?;

    assert_debug_snapshot!(errors);

    Ok(())
}

#[test]
fn release_records_parse_additional_source_inputs() -> Result<()> {
    let record = ReleaseRecord::from_json(
        Utf8Path::new("frankenphp.json"),
        FRANKENPHP_RELEASE_RECORD_WITH_SOURCE_INPUTS,
    )?;

    assert_debug_snapshot!(record.provenance().source_inputs());

    Ok(())
}

#[test]
fn release_records_parse_php_extension_metadata() -> Result<()> {
    let record_with_php_extensions = FRANKENPHP_RELEASE_RECORD_WITH_SOURCE_INPUTS.replacen(
        "\"provenance\": {",
        "\"php_extensions\": [\n    {\n      \"name\": \"redis\",\n      \"load_kind\": \"extension\",\n      \"path\": \"lib/php/extensions/redis.so\"\n    },\n    {\n      \"name\": \"xdebug\",\n      \"load_kind\": \"zend_extension\",\n      \"path\": \"lib/php/extensions/xdebug.so\"\n    }\n  ],\n  \"provenance\": {",
        1,
    );

    let record = ReleaseRecord::from_json(
        Utf8Path::new("frankenphp-with-extensions.json"),
        &record_with_php_extensions,
    )?;

    assert_debug_snapshot!(record);

    Ok(())
}

#[test]
fn release_records_reject_invalid_source_inputs() -> Result<()> {
    let invalid_name = FRANKENPHP_RELEASE_RECORD_WITH_SOURCE_INPUTS
        .replace("\"name\": \"php\"", "\"name\": \"PHP source\"");
    let invalid_source_url = FRANKENPHP_RELEASE_RECORD_WITH_SOURCE_INPUTS.replace(
        "https://www.php.net/distributions/php-8.4.20.tar.gz",
        "not a url",
    );
    let invalid_source_sha256 = FRANKENPHP_RELEASE_RECORD_WITH_SOURCE_INPUTS.replace(
        "a2def5d534d57c6a0236f2265de7537608af871900a4f7955eff463e9e38247d",
        "bad",
    );

    assert_debug_snapshot!((
        ReleaseRecord::from_json(Utf8Path::new("invalid-name.json"), &invalid_name),
        ReleaseRecord::from_json(Utf8Path::new("invalid-url.json"), &invalid_source_url),
        ReleaseRecord::from_json(Utf8Path::new("invalid-sha.json"), &invalid_source_sha256),
    ));

    Ok(())
}

#[test]
fn release_records_reject_unknown_metadata_fields() -> Result<()> {
    let unknown_release_field =
        VALID_RELEASE_RECORD.replacen("{", "{\n  \"unknown_release_metadata\": \"ignored\",", 1);
    let unknown_provenance_field = VALID_RELEASE_RECORD.replacen(
        "\"provenance\": {",
        "\"provenance\": {\n    \"unknown_provenance_metadata\": \"ignored\",",
        1,
    );
    let unknown_source_input_field = FRANKENPHP_RELEASE_RECORD_WITH_SOURCE_INPUTS.replacen(
        "\"name\": \"frankenphp\"",
        "\"unknown_source_input_metadata\": \"ignored\",\n        \"name\": \"frankenphp\"",
        1,
    );
    let unknown_revocation_field = VALID_REVOCATION_RECORD.replacen(
        "\"reason\": \"security issue\"",
        "\"unknown_revocation_metadata\": \"ignored\",\n  \"reason\": \"security issue\"",
        1,
    );

    assert_invalid_release_record(ReleaseRecord::from_json(
        Utf8Path::new("unknown-release.json"),
        &unknown_release_field,
    ));
    assert_invalid_release_record(ReleaseRecord::from_json(
        Utf8Path::new("unknown-provenance.json"),
        &unknown_provenance_field,
    ));
    assert_invalid_release_record(ReleaseRecord::from_json(
        Utf8Path::new("unknown-source-input.json"),
        &unknown_source_input_field,
    ));
    assert!(
        matches!(
            RevocationRecord::from_json(
                Utf8Path::new("unknown-revocation.json"),
                &unknown_revocation_field,
            ),
            Err(ReleaseError::InvalidRevocationRecord { .. })
        ),
        "unknown revocation metadata should be rejected"
    );

    Ok(())
}

#[test]
fn release_record_loader_rejects_duplicate_artifact_identity() -> Result<()> {
    let tempdir = tempdir()?;
    write_file(&tempdir.path().join("one.json"), VALID_RELEASE_RECORD)?;
    write_file(&tempdir.path().join("two.json"), VALID_RELEASE_RECORD)?;

    assert_debug_snapshot!(load_release_records(tempdir.path()));

    Ok(())
}

fn release_record_error(result: ReleaseResult<ReleaseRecord>) -> Result<ReleaseError> {
    match result {
        Ok(record) => Err(anyhow::anyhow!(
            "release record parsed successfully: {record:#?}"
        )),
        Err(error) => Ok(error),
    }
}

fn assert_invalid_release_record(result: ReleaseResult<ReleaseRecord>) {
    assert!(
        matches!(result, Err(ReleaseError::InvalidReleaseRecord { .. })),
        "unknown release metadata should be rejected, got {result:?}"
    );
}

#[test]
fn revocation_records_parse_append_only_target_metadata() -> Result<()> {
    let record = RevocationRecord::from_json(
        Utf8Path::new("redis-7.2.5-pv1-revoked.json"),
        VALID_REVOCATION_RECORD,
    )?;

    assert_debug_snapshot!(record);

    Ok(())
}

#[test]
fn revocation_loader_rejects_conflicting_revocations() -> Result<()> {
    let tempdir = tempdir()?;
    write_file(&tempdir.path().join("one.json"), VALID_REVOCATION_RECORD)?;
    write_file(
        &tempdir.path().join("two.json"),
        &VALID_REVOCATION_RECORD.replace("security issue", "broken archive"),
    )?;

    assert_debug_snapshot!(load_revocation_records(tempdir.path()));

    Ok(())
}

#[test]
fn revocation_loader_rejects_duplicate_revocation_records() -> Result<()> {
    let result = load_revocation_pair(VALID_REVOCATION_RECORD)?;

    assert!(matches!(
        result,
        Err(ReleaseError::DuplicateRevocation { .. })
    ));
    assert_debug_snapshot!(result);

    Ok(())
}

#[test]
fn revocation_loader_rejects_same_reason_metadata_conflicts() -> Result<()> {
    let different_replacement = load_revocation_pair(&VALID_REVOCATION_RECORD.replace(
        "\"replacement_artifact_version\": \"7.2.6-pv1\"",
        "\"replacement_artifact_version\": \"7.2.7-pv1\"",
    ))?;
    let different_revoked_at = load_revocation_pair(&VALID_REVOCATION_RECORD.replace(
        "\"revoked_at\": \"2026-06-06T13:00:00Z\"",
        "\"revoked_at\": \"2026-06-06T14:00:00Z\"",
    ))?;

    assert!(matches!(
        &different_replacement,
        Err(ReleaseError::ConflictingRevocation { .. })
    ));
    assert!(matches!(
        &different_revoked_at,
        Err(ReleaseError::ConflictingRevocation { .. })
    ));
    assert_debug_snapshot!((different_replacement, different_revoked_at));

    Ok(())
}

fn load_revocation_pair(second_record: &str) -> Result<ReleaseResult<Vec<RevocationRecord>>> {
    let tempdir = tempdir()?;
    write_file(&tempdir.path().join("one.json"), VALID_REVOCATION_RECORD)?;
    write_file(&tempdir.path().join("two.json"), second_record)?;

    Ok(load_revocation_records(tempdir.path()))
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests write local fixture metadata records"
)]
fn write_file(path: &Utf8Path, content: &str) -> Result<()> {
    std::fs::write(path, content)?;
    Ok(())
}

const VALID_RELEASE_RECORD: &str = r#"{
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

const FRANKENPHP_RELEASE_RECORD_WITH_SOURCE_INPUTS: &str = r#"{
  "resource": "frankenphp",
  "track": "8.4",
  "upstream_version": "8.4.20-frankenphp1.12.3",
  "pv_build_revision": "pv1",
  "artifact_version": "8.4.20-frankenphp1.12.3-pv1",
  "platform": "darwin-arm64",
  "object_key": "resources/frankenphp/8.4/8.4.20-frankenphp1.12.3-pv1/darwin-arm64/frankenphp-8.4.20-frankenphp1.12.3-pv1-darwin-arm64.tar.gz",
  "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "size": 42,
  "published_at": "2026-06-06T12:00:00Z",
  "minimum_pv_version": "0.1.0",
  "license_files": ["LICENSE"],
  "notice_files": ["NOTICE"],
  "provenance": {
    "source_url": "https://github.com/php/frankenphp/archive/refs/tags/v1.12.3.tar.gz",
    "source_sha256": "2996fb95bbdf8410847fdcd59df04cd2e297568f6472ebe488af5fb5f3c79363",
    "source_inputs": [
      {
        "name": "frankenphp",
        "source_url": "https://github.com/php/frankenphp/archive/refs/tags/v1.12.3.tar.gz",
        "source_sha256": "2996fb95bbdf8410847fdcd59df04cd2e297568f6472ebe488af5fb5f3c79363"
      },
      {
        "name": "php",
        "source_url": "https://www.php.net/distributions/php-8.4.20.tar.gz",
        "source_sha256": "a2def5d534d57c6a0236f2265de7537608af871900a4f7955eff463e9e38247d"
      }
    ],
    "recipe": "release/artifacts/recipes/php/build.sh",
    "pv_commit": "0123456789abcdef0123456789abcdef01234567",
    "build_run_id": "local-test"
  }
}"#;

const VALID_REVOCATION_RECORD: &str = r#"{
  "resource": "redis",
  "track": "7.2",
  "artifact_version": "7.2.5-pv1",
  "platform": "darwin-arm64",
  "reason": "security issue",
  "revoked_at": "2026-06-06T13:00:00Z",
  "replacement_artifact_version": "7.2.6-pv1"
}"#;
