use anyhow::Result;
use insta::{Settings, assert_debug_snapshot};
use self_update::{AppUpdateManifest, AppUpdateManifestError, AppUpdatePlatform, AppUpdateVersion};

#[test]
fn app_update_manifest_parses_stable_release_and_selects_platform() -> Result<()> {
    let manifest = AppUpdateManifest::parse(VALID_MANIFEST)?;
    let selected = manifest.select_platform(AppUpdatePlatform::DarwinArm64)?;

    assert_eq!(selected.platform(), AppUpdatePlatform::DarwinArm64);
    assert_debug_snapshot!(manifest);

    Ok(())
}

#[test]
fn app_update_manifest_rejects_schema_channel_version_and_compatibility_errors() -> Result<()> {
    let unsupported_schema =
        VALID_MANIFEST.replacen("\"schema_version\": 2", "\"schema_version\": 3", 1);
    let preview_channel =
        VALID_MANIFEST.replacen("\"channel\": \"stable\"", "\"channel\": \"preview\"", 1);
    let invalid_version =
        VALID_MANIFEST.replacen("\"version\": \"0.2.0\"", "\"version\": \"0.02.0\"", 1);
    let newer_minimum = VALID_MANIFEST.replacen(
        "\"minimum_pv_version\": \"0.1.0\"",
        "\"minimum_pv_version\": \"999.0.0\"",
        1,
    );

    let errors = [
        ("unsupported_schema", unsupported_schema),
        ("preview_channel", preview_channel),
        ("invalid_version", invalid_version),
        ("newer_minimum", newer_minimum),
    ]
    .into_iter()
    .map(|(name, manifest)| Ok((name, parse_manifest_error(&manifest)?)))
    .collect::<Result<Vec<_>>>()?;

    let mut settings = Settings::clone_current();
    settings.add_filter(
        r#"current_pv_version: "[^"]+""#,
        r#"current_pv_version: "<current-pv-version>""#,
    );
    settings.bind(|| assert_debug_snapshot!(errors));

    Ok(())
}

#[test]
fn app_update_manifest_rejects_invalid_asset_fields() -> Result<()> {
    let invalid_url = VALID_MANIFEST.replacen(
        "https://downloads.example.test/pv/0.2.0/pv-darwin-arm64",
        "http://downloads.example.test/pv/0.2.0/pv-darwin-arm64",
        1,
    );
    let invalid_checksum = VALID_MANIFEST.replacen(
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "not-a-sha",
        1,
    );
    let invalid_timestamp = VALID_MANIFEST.replacen(
        "\"published_at\": \"2026-06-11T12:00:00Z\"",
        "\"published_at\": \"not-a-date\"",
        1,
    );
    let invalid_platform =
        VALID_MANIFEST.replacen("\"platform\": \"darwin-arm64\"", "\"platform\": \"any\"", 1);
    let zero_size = VALID_MANIFEST.replacen("\"size\": 12345678", "\"size\": 0", 1);
    let duplicate_platform = VALID_MANIFEST.replacen(
        "\"platform\": \"darwin-amd64\"",
        "\"platform\": \"darwin-arm64\"",
        1,
    );

    let errors = [
        ("invalid_url", invalid_url),
        ("invalid_checksum", invalid_checksum),
        ("invalid_timestamp", invalid_timestamp),
        ("invalid_platform", invalid_platform),
        ("zero_size", zero_size),
        ("duplicate_platform", duplicate_platform),
    ]
    .into_iter()
    .map(|(name, manifest)| Ok((name, parse_manifest_error(&manifest)?)))
    .collect::<Result<Vec<_>>>()?;

    assert_debug_snapshot!(errors);

    Ok(())
}

#[test]
fn app_update_manifest_rejects_invalid_helper_fields() -> Result<()> {
    let invalid_version =
        VALID_MANIFEST.replacen("\"version\": \"1.0.0\"", "\"version\": \"01.0.0\"", 1);
    let invalid_protocol =
        VALID_MANIFEST.replacen("\"protocol_version\": 1", "\"protocol_version\": 0", 1);
    let invalid_url = VALID_MANIFEST.replacen(
        "https://downloads.example.test/pv/0.2.0/pv-helper-1.0.0-darwin-arm64",
        "http://downloads.example.test/pv/0.2.0/pv-helper-1.0.0-darwin-arm64",
        1,
    );
    let invalid_checksum = VALID_MANIFEST.replacen(
        "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "not-a-helper-sha",
        1,
    );
    let zero_size = VALID_MANIFEST.replacen("\"size\": 2345678", "\"size\": 0", 1);

    let errors = [
        ("invalid_helper_version", invalid_version),
        ("invalid_helper_protocol", invalid_protocol),
        ("invalid_helper_url", invalid_url),
        ("invalid_helper_checksum", invalid_checksum),
        ("zero_helper_size", zero_size),
    ]
    .into_iter()
    .map(|(name, manifest)| Ok((name, parse_manifest_error(&manifest)?)))
    .collect::<Result<Vec<_>>>()?;

    assert_debug_snapshot!(errors);

    Ok(())
}

#[test]
fn app_update_manifest_selection_reports_missing_platform() -> Result<()> {
    let arm64_only = VALID_MANIFEST.replace(
        r#",
    {
      "platform": "darwin-amd64",
      "url": "https://downloads.example.test/pv/0.2.0/pv-darwin-amd64",
      "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
      "size": 12345678,
      "helper": {
        "version": "1.0.0",
        "protocol_version": 1,
        "url": "https://downloads.example.test/pv/0.2.0/pv-helper-1.0.0-darwin-amd64",
        "sha256": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        "size": 2345678
      }
    }"#,
        "",
    );
    let manifest = AppUpdateManifest::parse(&arm64_only)?;
    let error = select_platform_error(&manifest, AppUpdatePlatform::DarwinAmd64)?;

    assert_debug_snapshot!(error);

    Ok(())
}

#[test]
fn app_update_versions_reject_path_unsafe_values() {
    for value in ["", ".", "..", "../0.2.0", "0.2.0/pv", "0.2.0\\pv"] {
        assert!(AppUpdateVersion::parse(value).is_err());
    }

    assert!(AppUpdateVersion::parse("0.2.0").is_ok());
}

fn parse_manifest_error(json: &str) -> Result<AppUpdateManifestError> {
    match AppUpdateManifest::parse(json) {
        Ok(manifest) => anyhow::bail!("manifest parsed successfully: {manifest:#?}"),
        Err(error) => Ok(error),
    }
}

fn select_platform_error(
    manifest: &AppUpdateManifest,
    platform: AppUpdatePlatform,
) -> Result<AppUpdateManifestError> {
    match manifest.select_platform(platform) {
        Ok(asset) => anyhow::bail!("asset selected successfully: {asset:#?}"),
        Err(error) => Ok(error),
    }
}

const VALID_MANIFEST: &str = r#"
{
  "schema_version": 2,
  "channel": "stable",
  "version": "0.2.0",
  "minimum_pv_version": "0.1.0",
  "published_at": "2026-06-11T12:00:00Z",
  "assets": [
    {
      "platform": "darwin-arm64",
      "url": "https://downloads.example.test/pv/0.2.0/pv-darwin-arm64",
      "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "size": 12345678,
      "helper": {
        "version": "1.0.0",
        "protocol_version": 1,
        "url": "https://downloads.example.test/pv/0.2.0/pv-helper-1.0.0-darwin-arm64",
        "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "size": 2345678
      }
    },
    {
      "platform": "darwin-amd64",
      "url": "https://downloads.example.test/pv/0.2.0/pv-darwin-amd64",
      "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
      "size": 12345678,
      "helper": {
        "version": "1.0.0",
        "protocol_version": 1,
        "url": "https://downloads.example.test/pv/0.2.0/pv-helper-1.0.0-darwin-amd64",
        "sha256": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        "size": 2345678
      }
    }
  ]
}
"#;
