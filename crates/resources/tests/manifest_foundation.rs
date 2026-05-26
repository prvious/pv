use anyhow::Result;
use insta::assert_debug_snapshot;
use resources::ArtifactManifest;
use resources::ResourcesError;
use resources::registry;
use resources::{ArtifactPlatform, TargetPlatform};
use resources::{ArtifactVersion, ResourceName, Sha256Digest, TrackName};

#[test]
fn registry_lists_all_pv_managed_artifact_resources() -> Result<()> {
    let names = registry::all()
        .iter()
        .map(|descriptor| descriptor.name())
        .collect::<Vec<_>>();

    assert_debug_snapshot!(names);

    Ok(())
}

#[test]
fn registry_normalizes_compiled_in_aliases() -> Result<()> {
    assert_eq!(registry::resolve("postgresql")?.name(), "postgres");
    assert_eq!(registry::resolve("pg")?.name(), "postgres");
    assert_eq!(registry::resolve("mail")?.name(), "mailpit");
    assert_eq!(registry::resolve("s3")?.name(), "rustfs");
    assert!(registry::resolve("postgresql")?.is_alias("postgresql"));
    assert!(registry::resolve("mysql")?.is_canonical("mysql"));

    Ok(())
}

#[test]
fn identity_types_reject_empty_values_and_bad_checksums() -> Result<()> {
    assert!(ResourceName::new("").is_err());
    assert!(TrackName::new("").is_err());
    assert!(ArtifactVersion::new("").is_err());
    assert!(Sha256Digest::new("not-a-sha").is_err());
    assert!(Sha256Digest::new("a".repeat(64)).is_ok());

    Ok(())
}

#[test]
fn platform_matching_prefers_exact_matches_over_any() -> Result<()> {
    let target = TargetPlatform::new("darwin-arm64")?;

    assert!(ArtifactPlatform::new("darwin-arm64")?.matches(target));
    assert!(ArtifactPlatform::new("any")?.matches(target));
    assert!(!ArtifactPlatform::new("darwin-amd64")?.matches(target));

    Ok(())
}

#[test]
fn manifest_parses_registry_backed_resources_tracks_and_artifacts() -> Result<()> {
    let manifest = ArtifactManifest::parse(VALID_MANIFEST)?;

    assert_debug_snapshot!(manifest.summary());

    Ok(())
}

#[test]
fn latest_selection_uses_published_at_with_exact_platform_preference() -> Result<()> {
    let manifest = ArtifactManifest::parse(SELECTION_MANIFEST)?;
    let selected = manifest.select_latest("redis", "7", TargetPlatform::new("darwin-arm64")?)?;

    assert_eq!(selected.artifact_version().as_str(), "7.2.5-pv1");

    Ok(())
}

#[test]
fn latest_selection_falls_back_to_any_when_exact_platform_is_missing() -> Result<()> {
    let manifest = ArtifactManifest::parse(SELECTION_MANIFEST)?;
    let selected = manifest.select_latest("composer", "2", TargetPlatform::new("darwin-arm64")?)?;

    assert_eq!(selected.artifact_version().as_str(), "2.8.0-pv2");

    Ok(())
}

#[test]
fn latest_track_alias_resolves_to_default_track() -> Result<()> {
    let manifest = ArtifactManifest::parse(SELECTION_MANIFEST)?;
    let track = manifest.resolve_track("redis", "latest")?;

    assert_eq!(track.as_str(), "7");

    Ok(())
}

#[test]
fn manifest_rejects_resource_aliases_and_unknown_resources() -> Result<()> {
    let alias_manifest = VALID_MANIFEST.replacen("\"name\": \"redis\"", "\"name\": \"pg\"", 1);
    let unknown_manifest =
        VALID_MANIFEST.replacen("\"name\": \"redis\"", "\"name\": \"unknown\"", 1);

    assert_debug_snapshot!(parse_manifest_error(&alias_manifest)?);
    assert_debug_snapshot!(parse_manifest_error(&unknown_manifest)?);

    Ok(())
}

#[test]
fn manifest_rejects_duplicate_tracks_and_artifacts() -> Result<()> {
    let duplicate_track = VALID_MANIFEST.replacen(
        "\"tracks\": [",
        r#""tracks": [
        {
          "name": "7",
          "artifacts": [
            {
              "artifact_version": "7.2.5-pv1",
              "upstream_version": "7.2.5",
              "pv_build_revision": "pv1",
              "platform": "darwin-arm64",
              "url": "https://artifacts.example.test/redis-7.2.5-pv1-darwin-arm64.tar.gz",
              "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
              "size": 12345,
              "published_at": "2026-05-26T14:30:00Z"
            }
          ]
        },"#,
        1,
    );
    let duplicate_artifact = VALID_MANIFEST.replacen(
        "\"artifacts\": [",
        r#""artifacts": [
            {
              "artifact_version": "7.2.5-pv1",
              "upstream_version": "7.2.5",
              "pv_build_revision": "pv1",
              "platform": "darwin-arm64",
              "url": "https://artifacts.example.test/redis-7.2.5-pv1-darwin-arm64.tar.gz",
              "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
              "size": 12345,
              "published_at": "2026-05-26T14:30:00Z"
            },"#,
        1,
    );

    assert_debug_snapshot!(parse_manifest_error(&duplicate_track)?);
    assert_debug_snapshot!(parse_manifest_error(&duplicate_artifact)?);

    Ok(())
}

#[test]
fn manifest_rejects_ambiguous_published_at_candidates() -> Result<()> {
    let ambiguous = SELECTION_MANIFEST.replacen(
        "\"published_at\": \"2026-05-25T14:30:00Z\"",
        "\"published_at\": \"2026-05-26T14:30:00Z\"",
        1,
    );
    let manifest = ArtifactManifest::parse(&ambiguous)?;
    let error = select_latest_error(
        &manifest,
        "redis",
        "7",
        TargetPlatform::new("darwin-arm64")?,
    )?;

    assert_debug_snapshot!(error);

    Ok(())
}

#[test]
fn manifest_rejects_invalid_checksum_and_published_at() -> Result<()> {
    let checksum_manifest = VALID_MANIFEST.replacen(
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "bad",
        1,
    );
    let published_at_manifest = VALID_MANIFEST.replacen("2026-05-26T14:30:00Z", "not-a-date", 1);

    assert_debug_snapshot!(parse_manifest_error(&checksum_manifest)?);
    assert_debug_snapshot!(parse_manifest_error(&published_at_manifest)?);

    Ok(())
}

const VALID_MANIFEST: &str = r#"
{
  "schema_version": 1,
  "minimum_pv_version": "0.1.0",
  "resources": [
    {
      "name": "redis",
      "default_track": "7",
      "tracks": [
        {
          "name": "7",
          "artifacts": [
            {
              "artifact_version": "7.2.5-pv1",
              "upstream_version": "7.2.5",
              "pv_build_revision": "pv1",
              "platform": "darwin-arm64",
              "url": "https://artifacts.example.test/redis-7.2.5-pv1-darwin-arm64.tar.gz",
              "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
              "size": 12345,
              "published_at": "2026-05-26T14:30:00Z"
            }
          ]
        }
      ]
    },
    {
      "name": "composer",
      "default_track": "2",
      "tracks": [
        {
          "name": "2",
          "artifacts": [
            {
              "artifact_version": "2.8.0-pv1",
              "upstream_version": "2.8.0",
              "pv_build_revision": "pv1",
              "platform": "any",
              "url": "https://artifacts.example.test/composer-2.8.0-pv1.tar.gz",
              "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
              "size": 23456,
              "published_at": "2026-05-26T15:30:00Z"
            }
          ]
        }
      ]
    }
  ]
}
"#;

const SELECTION_MANIFEST: &str = r#"
{
  "schema_version": 1,
  "minimum_pv_version": "0.1.0",
  "resources": [
    {
      "name": "redis",
      "default_track": "7",
      "tracks": [
        {
          "name": "7",
          "artifacts": [
            {
              "artifact_version": "7.2.4-pv1",
              "upstream_version": "7.2.4",
              "pv_build_revision": "pv1",
              "platform": "darwin-arm64",
              "url": "https://artifacts.example.test/redis-7.2.4-pv1-darwin-arm64.tar.gz",
              "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
              "size": 12345,
              "published_at": "2026-05-25T14:30:00Z"
            },
            {
              "artifact_version": "7.2.5-pv1",
              "upstream_version": "7.2.5",
              "pv_build_revision": "pv1",
              "platform": "darwin-arm64",
              "url": "https://artifacts.example.test/redis-7.2.5-pv1-darwin-arm64.tar.gz",
              "sha256": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
              "size": 12345,
              "published_at": "2026-05-26T14:30:00Z"
            },
            {
              "artifact_version": "7.2.6-pv1",
              "upstream_version": "7.2.6",
              "pv_build_revision": "pv1",
              "platform": "any",
              "url": "https://artifacts.example.test/redis-7.2.6-pv1-any.tar.gz",
              "sha256": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
              "size": 12345,
              "published_at": "2026-05-27T14:30:00Z"
            }
          ]
        }
      ]
    },
    {
      "name": "composer",
      "default_track": "2",
      "tracks": [
        {
          "name": "2",
          "artifacts": [
            {
              "artifact_version": "2.8.0-pv1",
              "upstream_version": "2.8.0",
              "pv_build_revision": "pv1",
              "platform": "any",
              "url": "https://artifacts.example.test/composer-2.8.0-pv1.tar.gz",
              "sha256": "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
              "size": 23456,
              "published_at": "2026-05-26T14:30:00Z",
              "revoked": true,
              "revocation_reason": "bad package"
            },
            {
              "artifact_version": "2.8.0-pv2",
              "upstream_version": "2.8.0",
              "pv_build_revision": "pv2",
              "platform": "any",
              "url": "https://artifacts.example.test/composer-2.8.0-pv2.tar.gz",
              "sha256": "1111111111111111111111111111111111111111111111111111111111111111",
              "size": 23456,
              "published_at": "2026-05-27T14:30:00Z"
            }
          ]
        }
      ]
    }
  ]
}
"#;

fn parse_manifest_error(json: &str) -> Result<ResourcesError> {
    match ArtifactManifest::parse(json) {
        Ok(manifest) => Err(anyhow::anyhow!(
            "manifest parsed successfully: {:#?}",
            manifest.summary()
        )),
        Err(error) => Ok(error),
    }
}

fn select_latest_error(
    manifest: &ArtifactManifest,
    resource: &str,
    track: &str,
    target: TargetPlatform,
) -> Result<ResourcesError> {
    match manifest.select_latest(resource, track, target) {
        Ok(artifact) => Err(anyhow::anyhow!(
            "artifact selected successfully: {}",
            artifact.artifact_version().as_str()
        )),
        Err(error) => Ok(error),
    }
}
