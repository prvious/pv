use anyhow::Result;
use insta::assert_debug_snapshot;
use resources::ArtifactManifest;
use resources::ManifestSelection;
use resources::PhpExtensionLoadKind;
use resources::ResourcesError;
use resources::registry;
use resources::{ArtifactPlatform, TargetPlatform};
use resources::{ArtifactVersion, ResourceName, Sha256Digest, TrackName, TrackSelector};

#[derive(Debug)]
#[expect(
    dead_code,
    reason = "snapshot-only structure is read through derived Debug"
)]
struct DescriptorSnapshot {
    name: &'static str,
    aliases: &'static [&'static str],
    kind: String,
    capabilities: Vec<String>,
}

#[derive(Debug)]
#[expect(
    dead_code,
    reason = "snapshot-only structure is read through derived Debug"
)]
struct InvalidUrlSnapshot {
    name: &'static str,
    error: ResourcesError,
}

#[test]
fn registry_lists_all_pv_managed_artifact_resources() -> Result<()> {
    let descriptors = registry::all()
        .iter()
        .map(|descriptor| DescriptorSnapshot {
            name: descriptor.name(),
            aliases: descriptor.aliases(),
            kind: format!("{:?}", descriptor.kind()),
            capabilities: descriptor
                .capabilities()
                .iter()
                .map(|capability| format!("{capability:?}"))
                .collect(),
        })
        .collect::<Vec<_>>();

    assert_debug_snapshot!(descriptors);

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
fn registry_only_resolves_aliases_through_resolve() -> Result<()> {
    assert_eq!(registry::resolve("postgresql")?.name(), "postgres");
    assert!(registry::resolve_canonical("postgresql").is_err());

    Ok(())
}

#[test]
fn identity_types_reject_empty_values_and_bad_checksums() -> Result<()> {
    assert!(ResourceName::new("").is_err());
    assert!(TrackName::new("").is_err());
    assert!(ArtifactVersion::new("").is_err());
    assert!(ResourceName::new(".").is_err());
    assert!(ResourceName::new("..").is_err());
    assert!(TrackName::new(".").is_err());
    assert!(TrackName::new("..").is_err());
    assert!(ArtifactVersion::new(".").is_err());
    assert!(ArtifactVersion::new("..").is_err());
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
    assert!(ArtifactPlatform::new("linux-amd64").is_err());
    assert!(TargetPlatform::new("linux-amd64").is_err());

    Ok(())
}

#[test]
fn manifest_parses_registry_backed_resources_tracks_and_artifacts() -> Result<()> {
    let manifest = ArtifactManifest::parse(&manifest_with_php_extension_metadata())?;
    let resource = ResourceName::new("redis")?;
    let track = TrackName::new("7")?;
    let selected =
        manifest.select_latest(&resource, &track, TargetPlatform::new("darwin-arm64")?)?;

    assert_eq!(
        selected
            .artifact()
            .php_extensions()
            .iter()
            .map(|module| module.name.as_str())
            .collect::<Vec<_>>(),
        ["redis", "xdebug"]
    );
    assert_eq!(
        selected.artifact().php_extensions()[1].load_kind,
        PhpExtensionLoadKind::ZendExtension
    );
    assert_debug_snapshot!(manifest);

    Ok(())
}

#[test]
fn manifest_rejects_mixed_any_and_exact_artifacts_for_one_version() -> Result<()> {
    let mixed_platforms = SELECTION_MANIFEST.replacen(
        "\"artifact_version\": \"7.2.6-pv1\"",
        "\"artifact_version\": \"7.2.5-pv1\"",
        1,
    );

    assert_debug_snapshot!(parse_manifest_error(&mixed_platforms)?);

    Ok(())
}

#[test]
fn latest_selection_falls_back_to_any_when_exact_platform_is_missing() -> Result<()> {
    let manifest = ArtifactManifest::parse(SELECTION_MANIFEST)?;
    let resource = ResourceName::new("composer")?;
    let track = TrackName::new("2")?;
    let selected =
        manifest.select_latest(&resource, &track, TargetPlatform::new("darwin-arm64")?)?;

    assert_eq!(selected.artifact().artifact_version().as_str(), "2.8.0-pv2");

    Ok(())
}

#[test]
fn latest_track_alias_resolves_to_default_track() -> Result<()> {
    let manifest = ArtifactManifest::parse(SELECTION_MANIFEST)?;
    let resource = ResourceName::new("redis")?;
    let track = manifest.resolve_track(&resource, TrackSelector::Latest)?;

    assert_eq!(track.as_str(), "7");

    Ok(())
}

#[test]
fn manifest_rejects_unsupported_schema_versions() -> Result<()> {
    let unsupported = VALID_MANIFEST.replacen("\"schema_version\": 1", "\"schema_version\": 2", 1);

    assert_debug_snapshot!(parse_manifest_error(&unsupported)?);

    Ok(())
}

#[test]
fn manifest_rejects_newer_minimum_pv_versions() -> Result<()> {
    let newer = VALID_MANIFEST.replacen(
        "\"minimum_pv_version\": \"0.1.0\"",
        "\"minimum_pv_version\": \"999.0.0\"",
        1,
    );
    let invalid = VALID_MANIFEST.replacen(
        "\"minimum_pv_version\": \"0.1.0\"",
        "\"minimum_pv_version\": \"0.01.0\"",
        1,
    );

    assert_debug_snapshot!(parse_manifest_error(&newer)?);
    assert_debug_snapshot!(parse_manifest_error(&invalid)?);

    Ok(())
}

#[test]
fn manifest_rejects_latest_as_a_concrete_track_name() -> Result<()> {
    let default_latest = VALID_MANIFEST.replacen(
        "\"default_track\": \"7\"",
        "\"default_track\": \"latest\"",
        1,
    );
    let track_latest = VALID_MANIFEST.replacen("\"name\": \"7\"", "\"name\": \"latest\"", 1);

    assert_debug_snapshot!(parse_manifest_error(&default_latest)?);
    assert_debug_snapshot!(parse_manifest_error(&track_latest)?);

    Ok(())
}

#[test]
fn manifest_rejects_missing_default_track() -> Result<()> {
    let missing_default_track =
        VALID_MANIFEST.replacen("\"default_track\": \"7\"", "\"default_track\": \"8\"", 1);

    assert_debug_snapshot!(parse_manifest_error(&missing_default_track)?);

    Ok(())
}

#[test]
fn latest_selection_chooses_newer_any_artifact_when_version_has_no_exact_platform() -> Result<()> {
    let manifest = ArtifactManifest::parse(SELECTION_MANIFEST)?;
    let resource = ResourceName::new("redis")?;
    let track = TrackName::new("7")?;
    let selected =
        manifest.select_latest(&resource, &track, TargetPlatform::new("darwin-arm64")?)?;

    assert_eq!(selected.artifact().artifact_version().as_str(), "7.2.6-pv1");

    Ok(())
}

#[test]
fn latest_selection_reports_revoked_newest_fallback() -> Result<()> {
    let manifest = ArtifactManifest::parse(REVOKED_SELECTION_MANIFEST)?;
    let resource = ResourceName::new("redis")?;
    let track = TrackName::new("7")?;
    let selected =
        manifest.select_latest(&resource, &track, TargetPlatform::new("darwin-arm64")?)?;

    match selected {
        ManifestSelection::RevokedFallback {
            artifact,
            revoked_latest,
        } => {
            assert_eq!(artifact.artifact_version().as_str(), "7.2.5-pv1");
            assert_eq!(revoked_latest.artifact_version().as_str(), "7.2.6-pv1");
        }
        ManifestSelection::Latest(_) => {
            return Err(anyhow::anyhow!("expected revoked fallback selection"));
        }
    }

    Ok(())
}

#[test]
fn latest_selection_reports_manifest_resource_misses_separately() -> Result<()> {
    let manifest = ArtifactManifest::parse(VALID_MANIFEST)?;
    let resource = ResourceName::new("mysql")?;
    let track = TrackName::new("8")?;

    assert_debug_snapshot!(select_latest_error(&manifest, &resource, &track)?);

    Ok(())
}

#[test]
fn latest_selection_reports_missing_tracks_separately() -> Result<()> {
    let manifest = ArtifactManifest::parse(VALID_MANIFEST)?;
    let resource = ResourceName::new("redis")?;
    let track = TrackName::new("8")?;

    assert_debug_snapshot!(select_latest_error(&manifest, &resource, &track)?);

    Ok(())
}

#[test]
fn latest_selection_errors_when_no_installable_artifact_exists() -> Result<()> {
    let revoked_only = REVOKED_SELECTION_MANIFEST.replacen(
        "\"published_at\": \"2026-05-26T14:30:00Z\"",
        "\"published_at\": \"2026-05-26T14:30:00Z\",\n              \"revoked\": true,\n              \"revocation_reason\": \"bad package\"",
        1,
    );
    let manifest = ArtifactManifest::parse(&revoked_only)?;
    let resource = ResourceName::new("redis")?;
    let track = TrackName::new("7")?;

    assert_debug_snapshot!(select_latest_error(&manifest, &resource, &track)?);

    Ok(())
}

#[test]
fn latest_selection_errors_when_no_platform_candidate_matches() -> Result<()> {
    let amd64_only = VALID_MANIFEST.replacen(
        "\"platform\": \"darwin-arm64\"",
        "\"platform\": \"darwin-amd64\"",
        1,
    );
    let manifest = ArtifactManifest::parse(&amd64_only)?;
    let resource = ResourceName::new("redis")?;
    let track = TrackName::new("7")?;

    assert_debug_snapshot!(select_latest_error(&manifest, &resource, &track)?);

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
fn manifest_rejects_invalid_revocation_state() -> Result<()> {
    let missing_reason = VALID_MANIFEST.replacen(
        "\"published_at\": \"2026-05-26T14:30:00Z\"",
        "\"published_at\": \"2026-05-26T14:30:00Z\",\n              \"revoked\": true",
        1,
    );
    let unexpected_reason = VALID_MANIFEST.replacen(
        "\"published_at\": \"2026-05-26T14:30:00Z\"",
        "\"published_at\": \"2026-05-26T14:30:00Z\",\n              \"revocation_reason\": \"bad package\"",
        1,
    );

    assert_debug_snapshot!(parse_manifest_error(&missing_reason)?);
    assert_debug_snapshot!(parse_manifest_error(&unexpected_reason)?);

    Ok(())
}

#[test]
fn manifest_rejects_duplicate_resources_tracks_and_artifacts() -> Result<()> {
    let duplicate_resource =
        VALID_MANIFEST.replacen("\"name\": \"composer\"", "\"name\": \"redis\"", 1);
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
    let empty_artifacts = VALID_MANIFEST.replacen(
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
            }
          ]"#,
        r#""artifacts": []"#,
        1,
    );

    assert_debug_snapshot!(parse_manifest_error(&duplicate_resource)?);
    assert_debug_snapshot!(parse_manifest_error(&duplicate_track)?);
    assert_debug_snapshot!(parse_manifest_error(&duplicate_artifact)?);
    assert_debug_snapshot!(parse_manifest_error(&empty_artifacts)?);

    Ok(())
}

#[test]
fn manifest_rejects_ambiguous_published_at_candidates() -> Result<()> {
    let ambiguous = SELECTION_MANIFEST.replacen(
        "\"published_at\": \"2026-05-25T14:30:00Z\"",
        "\"published_at\": \"2026-05-26T14:30:00Z\"",
        1,
    );
    assert_debug_snapshot!(parse_manifest_error(&ambiguous)?);

    Ok(())
}

#[test]
fn manifest_rejects_revoked_ambiguous_published_at_candidates() -> Result<()> {
    let ambiguous = REVOKED_SELECTION_MANIFEST.replacen(
        "{\n              \"artifact_version\": \"7.2.6-pv1\"",
        r#"{
              "artifact_version": "7.2.7-pv1",
              "upstream_version": "7.2.7",
              "pv_build_revision": "pv1",
              "platform": "darwin-arm64",
              "url": "https://artifacts.example.test/redis-7.2.7-pv1-darwin-arm64.tar.gz",
              "sha256": "4444444444444444444444444444444444444444444444444444444444444444",
              "size": 12345,
              "published_at": "2026-05-27T14:30:00Z",
              "revoked": true,
              "revocation_reason": "bad package"
            },
            {
              "artifact_version": "7.2.6-pv1""#,
        1,
    );
    assert_debug_snapshot!(parse_manifest_error(&ambiguous)?);

    Ok(())
}

#[test]
fn manifest_rejects_active_revoked_ambiguous_published_at_candidates() -> Result<()> {
    let ambiguous = REVOKED_SELECTION_MANIFEST.replacen(
        "\"published_at\": \"2026-05-26T14:30:00Z\"",
        "\"published_at\": \"2026-05-27T14:30:00Z\"",
        1,
    );
    assert_debug_snapshot!(parse_manifest_error(&ambiguous)?);

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
    let platform_manifest = VALID_MANIFEST.replacen(
        "\"platform\": \"darwin-arm64\"",
        "\"platform\": \"linux-amd64\"",
        1,
    );

    assert_debug_snapshot!(parse_manifest_error(&checksum_manifest)?);
    assert_debug_snapshot!(parse_manifest_error(&published_at_manifest)?);
    assert_debug_snapshot!(parse_manifest_error(&platform_manifest)?);

    Ok(())
}

#[test]
fn manifest_rejects_invalid_artifact_urls() -> Result<()> {
    let invalid_urls = [
        (
            "non_https",
            "http://artifacts.example.test/redis-7.2.5-pv1-darwin-arm64.tar.gz",
        ),
        (
            "missing_host",
            "https:///redis-7.2.5-pv1-darwin-arm64.tar.gz",
        ),
        ("missing_file_name", "https://artifacts.example.test/"),
        ("dot_file_name", "https://artifacts.example.test/."),
        ("parent_file_name", "https://artifacts.example.test/.."),
        (
            "backslash_file_name",
            "https://artifacts.example.test/redis\\7.2.5-pv1-darwin-arm64.tar.gz",
        ),
        (
            "invalid_authority",
            "https://exa mple.test/redis-7.2.5-pv1-darwin-arm64.tar.gz",
        ),
        (
            "invalid_ipv6_authority",
            "https://[:::1]/redis-7.2.5-pv1-darwin-arm64.tar.gz",
        ),
    ]
    .into_iter()
    .map(|(name, url)| {
        let manifest = manifest_with_artifact_url(url);

        Ok(InvalidUrlSnapshot {
            name,
            error: parse_manifest_error(&manifest)?,
        })
    })
    .collect::<Result<Vec<_>>>()?;

    assert_debug_snapshot!(invalid_urls);

    Ok(())
}

#[test]
fn manifest_allows_same_version_exact_artifacts_for_different_targets() -> Result<()> {
    let manifest_json = VALID_MANIFEST.replacen(
        "\"artifacts\": [",
        r#""artifacts": [
            {
              "artifact_version": "7.2.5-pv1",
              "upstream_version": "7.2.5",
              "pv_build_revision": "pv1",
              "platform": "darwin-amd64",
              "url": "https://artifacts.example.test/redis-7.2.5-pv1-darwin-amd64.tar.gz",
              "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
              "size": 12345,
              "published_at": "2026-05-26T14:30:00Z"
            },"#,
        1,
    );
    let manifest = ArtifactManifest::parse(&manifest_json)?;
    let resource = ResourceName::new("redis")?;
    let track = TrackName::new("7")?;

    let arm64 = manifest.select_latest(&resource, &track, TargetPlatform::new("darwin-arm64")?)?;
    let amd64 = manifest.select_latest(&resource, &track, TargetPlatform::new("darwin-amd64")?)?;

    assert_eq!(arm64.artifact().platform(), ArtifactPlatform::DarwinArm64);
    assert_eq!(amd64.artifact().platform(), ArtifactPlatform::DarwinAmd64);

    Ok(())
}

fn manifest_with_php_extension_metadata() -> String {
    VALID_MANIFEST.replacen(
        r#""published_at": "2026-05-26T14:30:00Z""#,
        r#""published_at": "2026-05-26T14:30:00Z",
              "php_extensions": [
                {"name":"redis","load_kind":"extension","path":"lib/php/extensions/redis.so"},
                {"name":"xdebug","load_kind":"zend_extension","path":"lib/php/extensions/xdebug.so"}
              ]"#,
        1,
    )
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

const REVOKED_SELECTION_MANIFEST: &str = r#"
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
              "sha256": "2222222222222222222222222222222222222222222222222222222222222222",
              "size": 12345,
              "published_at": "2026-05-26T14:30:00Z"
            },
            {
              "artifact_version": "7.2.6-pv1",
              "upstream_version": "7.2.6",
              "pv_build_revision": "pv1",
              "platform": "darwin-arm64",
              "url": "https://artifacts.example.test/redis-7.2.6-pv1-darwin-arm64.tar.gz",
              "sha256": "3333333333333333333333333333333333333333333333333333333333333333",
              "size": 12345,
              "published_at": "2026-05-27T14:30:00Z",
              "revoked": true,
              "revocation_reason": "bad package"
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
            manifest
        )),
        Err(error) => Ok(error),
    }
}

fn manifest_with_artifact_url(url: &str) -> String {
    let escaped_url = url.replace('\\', "\\\\").replace('"', "\\\"");

    VALID_MANIFEST.replacen(
        "\"url\": \"https://artifacts.example.test/redis-7.2.5-pv1-darwin-arm64.tar.gz\"",
        &format!("\"url\": \"{escaped_url}\""),
        1,
    )
}

fn select_latest_error(
    manifest: &ArtifactManifest,
    resource: &ResourceName,
    track: &TrackName,
) -> Result<ResourcesError> {
    match manifest.select_latest(resource, track, TargetPlatform::new("darwin-arm64")?) {
        Ok(selection) => Err(anyhow::anyhow!(
            "manifest selected successfully: {:#?}",
            selection
        )),
        Err(error) => Ok(error),
    }
}
