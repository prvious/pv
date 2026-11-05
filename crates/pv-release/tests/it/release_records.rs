use anyhow::Result;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::assert_debug_snapshot;
use pv_release::record::{
    ReleaseRecord, RevocationRecord, load_release_records, load_revocation_records,
};
use pv_release::{ReleaseError, Result as ReleaseResult};

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
    let mismatched_artifact_version = VALID_RELEASE_RECORD.replace(
        "\"artifact_version\": \"7.2.5-pv1\"",
        "\"artifact_version\": \"7.2.5-pv2\"",
    );
    let missing_license = VALID_RELEASE_RECORD.replace(
        "\"license_files\": [\"LICENSE\"],",
        "\"license_files\": [],",
    );
    let invalid_source_input_name = FRANKENPHP_RELEASE_RECORD_WITH_SOURCE_INPUTS
        .replace("\"name\": \"php\"", "\"name\": \"PHP source\"");
    let invalid_source_input_url = FRANKENPHP_RELEASE_RECORD_WITH_SOURCE_INPUTS.replace(
        "https://www.php.net/distributions/php-8.4.20.tar.gz",
        "not a url",
    );
    let invalid_source_input_sha256 = FRANKENPHP_RELEASE_RECORD_WITH_SOURCE_INPUTS.replace(
        "a2def5d534d57c6a0236f2265de7537608af871900a4f7955eff463e9e38247d",
        "bad",
    );

    let errors = [
        ("absolute_object_key", absolute_object_key),
        ("parent_object_key", parent_object_key),
        ("mismatched_object_key", mismatched_object_key),
        ("invalid_source_url", invalid_source_url),
        ("invalid_source_sha256", invalid_source_sha256),
        ("invalid_recipe", invalid_recipe),
        ("invalid_commit", invalid_commit),
        ("empty_build_run", empty_build_run),
        ("mismatched_artifact_version", mismatched_artifact_version),
        ("missing_license", missing_license),
        ("invalid_source_input_name", invalid_source_input_name),
        ("invalid_source_input_url", invalid_source_input_url),
        ("invalid_source_input_sha256", invalid_source_input_sha256),
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
fn release_records_reject_invalid_php_extension_metadata() -> Result<()> {
    let valid = release_record_with_php_extensions();
    let cases = [
        (
            "empty_name",
            valid.replace("\"name\": \"redis\"", "\"name\": \"\""),
        ),
        (
            "invalid_load_kind",
            valid.replace("\"load_kind\": \"extension\"", "\"load_kind\": \"module\""),
        ),
        (
            "empty_path",
            valid.replace(
                "\"path\": \"lib/php/extensions/redis.so\"",
                "\"path\": \"\"",
            ),
        ),
        (
            "current_dir_path",
            valid.replace(
                "\"path\": \"lib/php/extensions/redis.so\"",
                "\"path\": \".\"",
            ),
        ),
        (
            "parent_path",
            valid.replace(
                "\"path\": \"lib/php/extensions/redis.so\"",
                "\"path\": \"../redis.so\"",
            ),
        ),
        (
            "duplicate_name",
            valid.replace("\"name\": \"xdebug\"", "\"name\": \"redis\""),
        ),
        (
            "non_php_resource",
            VALID_RELEASE_RECORD.replacen(
                "\"provenance\": {",
                "\"php_extensions\": [\n    {\n      \"name\": \"redis\",\n      \"load_kind\": \"extension\",\n      \"path\": \"lib/php/extensions/redis.so\"\n    }\n  ],\n  \"provenance\": {",
                1,
            ),
        ),
    ]
    .into_iter()
    .map(|(name, json)| {
        Ok((
            name,
            release_record_error(ReleaseRecord::from_json(
                Utf8Path::new("invalid-php-extension.json"),
                &json,
            ))?,
        ))
    })
    .collect::<Result<Vec<_>>>()?;

    assert_debug_snapshot!(cases);

    Ok(())
}

fn release_record_with_php_extensions() -> String {
    FRANKENPHP_RELEASE_RECORD_WITH_SOURCE_INPUTS.replacen(
        "\"provenance\": {",
        "\"php_extensions\": [\n    {\n      \"name\": \"redis\",\n      \"load_kind\": \"extension\",\n      \"path\": \"lib/php/extensions/redis.so\"\n    },\n    {\n      \"name\": \"xdebug\",\n      \"load_kind\": \"zend_extension\",\n      \"path\": \"lib/php/extensions/xdebug.so\"\n    }\n  ],\n  \"provenance\": {",
        1,
    )
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

#[test]
fn revocation_loader_rejects_conflicting_revocations() -> Result<()> {
    let cases = [
        ("duplicate", VALID_REVOCATION_RECORD.to_string()),
        (
            "different_reason",
            VALID_REVOCATION_RECORD.replace("security issue", "broken archive"),
        ),
        (
            "different_replacement",
            VALID_REVOCATION_RECORD.replace(
                "\"replacement_artifact_version\": \"7.2.6-pv1\"",
                "\"replacement_artifact_version\": \"7.2.7-pv1\"",
            ),
        ),
        (
            "different_revoked_at",
            VALID_REVOCATION_RECORD.replace(
                "\"revoked_at\": \"2026-06-06T13:00:00Z\"",
                "\"revoked_at\": \"2026-06-06T14:00:00Z\"",
            ),
        ),
    ]
    .into_iter()
    .map(|(name, second_record)| Ok((name, load_revocation_pair(&second_record)?)))
    .collect::<Result<Vec<_>>>()?;

    assert_debug_snapshot!(cases);

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
