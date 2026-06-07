use anyhow::Result;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::{assert_debug_snapshot, assert_snapshot};
use pv_release::defaults::ManifestDefaults;
use pv_release::manifest::generate_manifest_file_with_defaults;
use pv_release::{ReleaseError, Result as ReleaseResult};
use resources::{ArtifactManifest, ResourceName};
use serde_json::Value;

#[test]
fn manifest_defaults_parse_public_toml_api() -> Result<()> {
    let defaults = ManifestDefaults::from_toml(
        Utf8Path::new("release/artifacts/default-tracks.toml"),
        r#"
[[resource]]
name = "php"
default_track = "8.4"
"#,
    )?;
    let resource = ResourceName::new("php")?;
    let default_track = defaults
        .default_track_for(&resource)
        .map(|track| track.as_str())
        .unwrap_or("");

    assert_eq!(default_track, "8.4");

    Ok(())
}

#[test]
fn manifest_generator_uses_default_track_metadata_for_multi_track_resources() -> Result<()> {
    let tempdir = tempdir()?;
    let records_dir = tempdir.path().join("records");
    let revocations_dir = tempdir.path().join("revocations");
    let defaults = tempdir.path().join("default-tracks.toml");
    let output = tempdir.path().join("manifests/manifest.json");

    create_dir_all(&records_dir)?;
    create_dir_all(&revocations_dir)?;
    write_file(
        &records_dir.join("php-8.3.31-pv1-darwin-arm64.json"),
        PHP_8_3_RECORD,
    )?;
    write_file(
        &records_dir.join("php-8.4.20-pv1-darwin-arm64.json"),
        PHP_8_4_RECORD,
    )?;
    write_file(
        &defaults,
        r#"
[[resource]]
name = "php"
default_track = "8.4"
"#,
    )?;

    generate_manifest_file_with_defaults(
        &records_dir,
        &revocations_dir,
        Some(&defaults),
        &output,
        "https://artifacts.example.test",
    )?;
    let manifest_json = read_file(&output)?;
    ArtifactManifest::parse(&manifest_json)?;
    let manifest: Value = serde_json::from_str(&manifest_json)?;
    let default_track = manifest["resources"]
        .as_array()
        .and_then(|resources| {
            resources
                .iter()
                .find(|resource| resource["name"].as_str() == Some("php"))
        })
        .and_then(|resource| resource["default_track"].as_str())
        .unwrap_or("");
    assert_eq!(default_track, "8.4");
    assert_snapshot!(manifest_json);

    Ok(())
}

#[test]
fn manifest_generator_rejects_default_track_metadata_for_missing_generated_track() -> Result<()> {
    let error = generate_manifest_error_with_defaults(
        &[
            ("php-8.3.31-pv1-darwin-arm64.json", PHP_8_3_RECORD),
            ("php-8.4.20-pv1-darwin-arm64.json", PHP_8_4_RECORD),
        ],
        r#"
[[resource]]
name = "php"
default_track = "8.5"
"#,
    )?;

    assert!(matches!(
        error,
        ReleaseError::GeneratedManifestInvalid { .. }
    ));
    assert_debug_snapshot!(error);

    Ok(())
}

#[test]
fn manifest_generator_rejects_default_track_metadata_for_missing_generated_resource() -> Result<()>
{
    let error = generate_manifest_error_with_defaults(
        &[("php-8.4.20-pv1-darwin-arm64.json", PHP_8_4_RECORD)],
        r#"
[[resource]]
name = "mysql"
default_track = "8.0"
"#,
    )?;

    assert!(matches!(
        error,
        ReleaseError::GeneratedManifestInvalid { .. }
    ));
    assert_debug_snapshot!(error);

    Ok(())
}

fn generate_manifest_error_with_defaults(
    releases: &[(&str, &str)],
    defaults: &str,
) -> Result<ReleaseError> {
    let tempdir = tempdir()?;
    let records_dir = tempdir.path().join("records");
    let revocations_dir = tempdir.path().join("revocations");
    let defaults_path = tempdir.path().join("default-tracks.toml");
    let output = tempdir.path().join("manifests/manifest.json");

    create_dir_all(&records_dir)?;
    create_dir_all(&revocations_dir)?;
    for (name, content) in releases {
        write_file(&records_dir.join(name), content)?;
    }
    write_file(&defaults_path, defaults)?;

    let error = manifest_error(generate_manifest_file_with_defaults(
        &records_dir,
        &revocations_dir,
        Some(&defaults_path),
        &output,
        "https://artifacts.example.test",
    ))?;
    assert!(!path_exists(&output));

    Ok(error)
}

fn manifest_error(result: ReleaseResult<()>) -> Result<ReleaseError> {
    match result {
        Ok(()) => Err(anyhow::anyhow!(
            "manifest generation succeeded unexpectedly"
        )),
        Err(error) => Ok(error),
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests create local metadata fixtures"
)]
fn create_dir_all(path: &Utf8Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests write local metadata fixtures"
)]
fn write_file(path: &Utf8Path, content: &str) -> Result<()> {
    std::fs::write(path, content)?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests read generated local manifests"
)]
fn read_file(path: &Utf8Path) -> Result<String> {
    Ok(std::fs::read_to_string(path)?)
}

fn path_exists(path: &Utf8Path) -> bool {
    path.exists()
}

const PHP_8_3_RECORD: &str = r#"{
  "resource": "php",
  "track": "8.3",
  "upstream_version": "8.3.31",
  "pv_build_revision": "pv1",
  "artifact_version": "8.3.31-pv1",
  "platform": "darwin-arm64",
  "object_key": "resources/php/8.3/8.3.31-pv1/darwin-arm64/php-8.3.31-pv1-darwin-arm64.tar.gz",
  "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "size": 42,
  "published_at": "2026-06-07T12:00:00Z",
  "minimum_pv_version": "0.1.0",
  "license_files": ["LICENSE"],
  "notice_files": ["NOTICE"],
  "provenance": {
    "source_url": "https://www.php.net/distributions/php-8.3.31.tar.gz",
    "source_sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "recipe": "release/artifacts/recipes/php/build.sh",
    "pv_commit": "0123456789abcdef0123456789abcdef01234567",
    "build_run_id": "local-test"
  }
}"#;

const PHP_8_4_RECORD: &str = r#"{
  "resource": "php",
  "track": "8.4",
  "upstream_version": "8.4.20",
  "pv_build_revision": "pv1",
  "artifact_version": "8.4.20-pv1",
  "platform": "darwin-arm64",
  "object_key": "resources/php/8.4/8.4.20-pv1/darwin-arm64/php-8.4.20-pv1-darwin-arm64.tar.gz",
  "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
  "size": 42,
  "published_at": "2026-06-07T12:00:00Z",
  "minimum_pv_version": "0.1.0",
  "license_files": ["LICENSE"],
  "notice_files": ["NOTICE"],
  "provenance": {
    "source_url": "https://www.php.net/distributions/php-8.4.20.tar.gz",
    "source_sha256": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
    "recipe": "release/artifacts/recipes/php/build.sh",
    "pv_commit": "0123456789abcdef0123456789abcdef01234567",
    "build_run_id": "local-test"
  }
}"#;
