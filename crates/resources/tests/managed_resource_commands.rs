use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::io::{Error, Write};

use anyhow::{Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use flate2::Compression;
use flate2::write::GzEncoder;
use insta::assert_debug_snapshot;
use resources::{
    ArtifactManifestSource, ManagedResourceCommands, ManagedResourceInstall,
    ManagedResourceRemovalIntent, ManagedResourceTrack, ManagedResourceUninstallOptions,
    ManagedResourceUpdate, ResourceAdapter, ResourceHttpClient, ResourceName, ResourcesError,
    TargetPlatform, TrackName, TrackSelector, frankenphp_adapter, php_adapter,
};
use sha2::{Digest, Sha256};
use state::{Database, ManagedResourceTrackRecord, PvPaths};
use tar::{Builder, Header};

#[test]
fn managed_resource_commands_install_update_list_and_uninstall_fake_adapter() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let commands =
        ManagedResourceCommands::new(paths.clone(), MANIFEST_URL, TargetPlatform::DarwinArm64);
    let adapter = FakeAdapter::new("redis", &["bin/pv-fake-resource"])?;
    let first_artifact = fixture_artifact("7.2.5-pv1", "first")?;
    let second_artifact = fixture_artifact("7.2.6-pv1", "second")?;
    let first_manifest = manifest_with_artifacts(&[&first_artifact]);
    let second_manifest = manifest_with_artifacts(&[&first_artifact, &second_artifact]);
    let client = ScriptedClient::new()
        .with_text(&first_manifest)
        .with_bytes(first_artifact.bytes())
        .with_text(&second_manifest)
        .with_bytes(second_artifact.bytes());

    let installed = commands.install(&adapter, TrackSelector::Latest, &client)?;
    let updated = commands.update(&adapter, &client)?;
    let listed_after_update = commands.list(Some(adapter.resource_name()))?;
    let removal_intent = commands.uninstall(
        adapter.resource_name(),
        updated.installs()[0].track(),
        ManagedResourceUninstallOptions::default(),
    )?;
    let listed_after_uninstall = commands.list(Some(adapter.resource_name()))?;
    let state_after_uninstall = raw_track_records_summary(&paths, tempdir.path())?;

    assert_debug_snapshot!((
        install_summary(&installed, tempdir.path())?,
        update_summary(&updated, tempdir.path())?,
        track_records_summary(&listed_after_update, tempdir.path())?,
        removal_intent_summary(&removal_intent),
        track_records_summary(&listed_after_uninstall, tempdir.path())?,
        state_after_uninstall,
    ));

    Ok(())
}

#[test]
fn managed_resource_commands_keep_installed_state_when_update_validation_fails() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let commands =
        ManagedResourceCommands::new(paths.clone(), MANIFEST_URL, TargetPlatform::DarwinArm64);
    let adapter = FakeAdapter::new("redis", &["bin/pv-fake-resource"])?;
    let first_artifact = fixture_artifact("7.2.5-pv1", "first")?;
    let broken_artifact = fixture_artifact_with_entries("7.2.6-pv1", &[("README.md", "broken")])?;
    let first_manifest = manifest_with_artifacts(&[&first_artifact]);
    let broken_manifest = manifest_with_artifacts(&[&first_artifact, &broken_artifact]);
    let client = ScriptedClient::new()
        .with_text(&first_manifest)
        .with_bytes(first_artifact.bytes())
        .with_text(&broken_manifest)
        .with_bytes(broken_artifact.bytes());

    let installed = commands.install(&adapter, TrackSelector::Latest, &client)?;
    let failed_update = commands.update(&adapter, &client);
    let listed_after_failure = commands.list(Some(adapter.resource_name()))?;

    assert_debug_snapshot!((
        install_summary(&installed, tempdir.path())?,
        failed_update,
        track_records_summary(&listed_after_failure, tempdir.path())?,
    ));

    Ok(())
}

#[test]
fn managed_resource_commands_report_revoked_latest_fallback() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let commands =
        ManagedResourceCommands::new(paths.clone(), MANIFEST_URL, TargetPlatform::DarwinArm64);
    let adapter = FakeAdapter::new("redis", &["bin/pv-fake-resource"])?;
    let fallback_artifact = fixture_artifact("7.2.6-pv1", "fallback")?;
    let revoked_artifact = revoked_fixture_artifact("7.2.7-pv1", "revoked", "bad package")?;
    let manifest = manifest_with_artifacts(&[&fallback_artifact, &revoked_artifact]);
    let client = ScriptedClient::new()
        .with_text(&manifest)
        .with_bytes(fallback_artifact.bytes());

    let installed = commands.install(&adapter, TrackSelector::Latest, &client)?;

    assert_debug_snapshot!(install_summary(&installed, tempdir.path())?);

    Ok(())
}

#[test]
fn managed_resource_commands_update_all_installed_tracks_from_one_manifest_refresh() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let commands =
        ManagedResourceCommands::new(paths.clone(), MANIFEST_URL, TargetPlatform::DarwinArm64);
    let adapter = FakeAdapter::new("redis", &["bin/pv-fake-resource"])?;
    let first_72_artifact = fixture_artifact("7.2.5-pv1", "7.2 first")?;
    let second_72_artifact = fixture_artifact("7.2.6-pv1", "7.2 second")?;
    let first_80_artifact = fixture_artifact("8.0.1-pv1", "8.0 first")?;
    let second_80_artifact = fixture_artifact("8.0.2-pv1", "8.0 second")?;
    let initial_manifest = manifest_with_tracks(&[
        ("7.2", &[&first_72_artifact]),
        ("8.0", &[&first_80_artifact]),
    ]);
    let updated_manifest = manifest_with_tracks(&[
        ("7.2", &[&first_72_artifact, &second_72_artifact]),
        ("8.0", &[&first_80_artifact, &second_80_artifact]),
    ]);
    let client = ScriptedClient::new()
        .with_text(&initial_manifest)
        .with_bytes(first_72_artifact.bytes())
        .with_text(&initial_manifest)
        .with_bytes(first_80_artifact.bytes())
        .with_text(&updated_manifest)
        .with_bytes(second_72_artifact.bytes())
        .with_bytes(second_80_artifact.bytes());

    commands.install(&adapter, TrackSelector::Latest, &client)?;
    commands.install(
        &adapter,
        TrackSelector::Track(TrackName::new("8.0")?),
        &client,
    )?;
    let updated = commands.update(&adapter, &client)?;
    let listed_after_update = commands.list(Some(adapter.resource_name()))?;
    let manifest_request_count = client.text_request_count();

    assert_debug_snapshot!((
        update_summary(&updated, tempdir.path())?,
        track_records_summary(&listed_after_update, tempdir.path())?,
        manifest_request_count,
    ));

    Ok(())
}

#[test]
fn managed_resource_commands_update_without_installed_tracks_does_not_refresh_manifest()
-> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let commands =
        ManagedResourceCommands::new(paths.clone(), MANIFEST_URL, TargetPlatform::DarwinArm64);
    let adapter = FakeAdapter::new("redis", &["bin/pv-fake-resource"])?;
    let client = ScriptedClient::new();

    let updated = commands.update(&adapter, &client)?;

    assert_debug_snapshot!(update_summary(&updated, tempdir.path())?);

    Ok(())
}

#[test]
fn managed_resource_commands_install_php_pair_resolves_latest_once_for_both_resources() -> Result<()>
{
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let commands =
        ManagedResourceCommands::new(paths.clone(), MANIFEST_URL, TargetPlatform::DarwinArm64);
    let php_84_artifact = runtime_fixture_artifact("php", "8.4.8-pv1", "bin/php", "php 8.4")?;
    let frankenphp_83_artifact = runtime_fixture_artifact(
        "frankenphp",
        "8.3.22-pv1",
        "bin/frankenphp",
        "frankenphp 8.3",
    )?;
    let frankenphp_84_artifact = runtime_fixture_artifact(
        "frankenphp",
        "8.4.8-pv1",
        "bin/frankenphp",
        "frankenphp 8.4",
    )?;
    let manifest = manifest_with_resources(&[
        manifest_resource(
            "php",
            "8.4",
            vec![manifest_track("8.4", vec![&php_84_artifact])],
        ),
        manifest_resource(
            "frankenphp",
            "8.3",
            vec![
                manifest_track("8.3", vec![&frankenphp_83_artifact]),
                manifest_track("8.4", vec![&frankenphp_84_artifact]),
            ],
        ),
    ]);
    let client = ScriptedClient::new()
        .with_text(&manifest)
        .with_bytes(php_84_artifact.bytes())
        .with_bytes(frankenphp_84_artifact.bytes());

    let installed = commands.install_php_pair(TrackSelector::Latest, &client)?;
    let listed_after_install = commands.list(None)?;
    let manifest_request_count = client.text_request_count();

    assert_debug_snapshot!((
        install_summary(installed.php(), tempdir.path())?,
        install_summary(installed.frankenphp(), tempdir.path())?,
        track_records_summary(&listed_after_install, tempdir.path())?,
        manifest_request_count,
    ));

    Ok(())
}

#[test]
fn managed_resource_commands_update_php_pairs_uses_installed_track_union_and_one_manifest_refresh()
-> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let commands =
        ManagedResourceCommands::new(paths.clone(), MANIFEST_URL, TargetPlatform::DarwinArm64);
    let php_adapter = php_adapter()?;
    let frankenphp_adapter = frankenphp_adapter()?;
    let php_83_artifact = runtime_fixture_artifact("php", "8.3.23-pv1", "bin/php", "php 8.3")?;
    let php_84_artifact = runtime_fixture_artifact("php", "8.4.8-pv1", "bin/php", "php 8.4")?;
    let frankenphp_83_artifact = runtime_fixture_artifact(
        "frankenphp",
        "8.3.23-pv1",
        "bin/frankenphp",
        "frankenphp 8.3",
    )?;
    let frankenphp_84_artifact = runtime_fixture_artifact(
        "frankenphp",
        "8.4.8-pv1",
        "bin/frankenphp",
        "frankenphp 8.4",
    )?;
    let php_83_update_artifact =
        runtime_fixture_artifact("php", "8.3.24-pv1", "bin/php", "php 8.3 update")?;
    let php_84_update_artifact =
        runtime_fixture_artifact("php", "8.4.9-pv1", "bin/php", "php 8.4 update")?;
    let frankenphp_83_update_artifact = runtime_fixture_artifact(
        "frankenphp",
        "8.3.24-pv1",
        "bin/frankenphp",
        "frankenphp 8.3 update",
    )?;
    let frankenphp_84_update_artifact = runtime_fixture_artifact(
        "frankenphp",
        "8.4.9-pv1",
        "bin/frankenphp",
        "frankenphp 8.4 update",
    )?;
    let initial_manifest = manifest_with_resources(&[
        manifest_resource(
            "php",
            "8.4",
            vec![
                manifest_track("8.3", vec![&php_83_artifact]),
                manifest_track("8.4", vec![&php_84_artifact]),
            ],
        ),
        manifest_resource(
            "frankenphp",
            "8.4",
            vec![
                manifest_track("8.3", vec![&frankenphp_83_artifact]),
                manifest_track("8.4", vec![&frankenphp_84_artifact]),
            ],
        ),
    ]);
    let updated_manifest = manifest_with_resources(&[
        manifest_resource(
            "php",
            "8.4",
            vec![
                manifest_track("8.3", vec![&php_83_artifact, &php_83_update_artifact]),
                manifest_track("8.4", vec![&php_84_artifact, &php_84_update_artifact]),
            ],
        ),
        manifest_resource(
            "frankenphp",
            "8.4",
            vec![
                manifest_track(
                    "8.3",
                    vec![&frankenphp_83_artifact, &frankenphp_83_update_artifact],
                ),
                manifest_track(
                    "8.4",
                    vec![&frankenphp_84_artifact, &frankenphp_84_update_artifact],
                ),
            ],
        ),
    ]);
    let client = ScriptedClient::new()
        .with_text(&initial_manifest)
        .with_bytes(php_84_artifact.bytes())
        .with_text(&initial_manifest)
        .with_bytes(frankenphp_83_artifact.bytes())
        .with_text(&updated_manifest)
        .with_bytes(php_83_update_artifact.bytes())
        .with_bytes(frankenphp_83_update_artifact.bytes())
        .with_bytes(php_84_update_artifact.bytes())
        .with_bytes(frankenphp_84_update_artifact.bytes());

    commands.install(&php_adapter, TrackSelector::Latest, &client)?;
    commands.install(
        &frankenphp_adapter,
        TrackSelector::Track(TrackName::new("8.3")?),
        &client,
    )?;
    let manifest_requests_before_update = client.text_request_count();
    let updated = commands.update_php_pairs(&client)?;
    let listed_after_update = commands.list(None)?;
    let manifest_refreshes_during_update =
        client.text_request_count() - manifest_requests_before_update;

    assert_debug_snapshot!((
        install_summaries(updated.installs(), tempdir.path())?,
        track_records_summary(&listed_after_update, tempdir.path())?,
        manifest_refreshes_during_update,
    ));

    Ok(())
}

#[test]
fn managed_resource_commands_update_php_pairs_without_installed_tracks_does_not_refresh_manifest()
-> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let commands =
        ManagedResourceCommands::new(paths.clone(), MANIFEST_URL, TargetPlatform::DarwinArm64);
    let client = ScriptedClient::new();

    let updated = commands.update_php_pairs(&client)?;

    assert_debug_snapshot!((
        install_summaries(updated.installs(), tempdir.path())?,
        client.text_request_count(),
    ));

    Ok(())
}

#[test]
fn managed_resource_commands_uninstall_php_pair_records_both_removal_intents() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let commands =
        ManagedResourceCommands::new(paths.clone(), MANIFEST_URL, TargetPlatform::DarwinArm64);
    let php_artifact = runtime_fixture_artifact("php", "8.4.8-pv1", "bin/php", "php 8.4")?;
    let frankenphp_artifact = runtime_fixture_artifact(
        "frankenphp",
        "8.4.8-pv1",
        "bin/frankenphp",
        "frankenphp 8.4",
    )?;
    let manifest = manifest_with_resources(&[
        manifest_resource(
            "php",
            "8.4",
            vec![manifest_track("8.4", vec![&php_artifact])],
        ),
        manifest_resource(
            "frankenphp",
            "8.4",
            vec![manifest_track("8.4", vec![&frankenphp_artifact])],
        ),
    ]);
    let client = ScriptedClient::new()
        .with_text(&manifest)
        .with_bytes(php_artifact.bytes())
        .with_bytes(frankenphp_artifact.bytes());

    commands.install_php_pair(TrackSelector::Latest, &client)?;
    let intent = commands.uninstall_php_pair(
        &TrackName::new("8.4")?,
        ManagedResourceUninstallOptions::new()
            .prune(true)
            .force(true),
    )?;
    let state_after_uninstall = raw_track_records_summary(&paths, tempdir.path())?;

    assert_debug_snapshot!((
        removal_intent_summary(intent.php()),
        removal_intent_summary(intent.frankenphp()),
        state_after_uninstall,
    ));

    Ok(())
}

#[test]
fn managed_resource_commands_install_update_and_uninstall_composer_track_two() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let commands =
        ManagedResourceCommands::new(paths.clone(), MANIFEST_URL, TargetPlatform::DarwinArm64);
    let first_artifact = composer_fixture_artifact("2.8.0-pv1", "first")?;
    let second_artifact = composer_fixture_artifact("2.8.1-pv1", "second")?;
    let first_manifest = manifest_with_resources(&[manifest_resource(
        "composer",
        "2",
        vec![manifest_track("2", vec![&first_artifact])],
    )]);
    let second_manifest = manifest_with_resources(&[manifest_resource(
        "composer",
        "2",
        vec![manifest_track("2", vec![&first_artifact, &second_artifact])],
    )]);
    let client = ScriptedClient::new()
        .with_text(&first_manifest)
        .with_bytes(first_artifact.bytes())
        .with_text(&second_manifest)
        .with_bytes(second_artifact.bytes());

    let installed = commands.install_composer(&client)?;
    let updated = commands.update_composer(&client)?;
    let intent = commands.uninstall_composer(ManagedResourceUninstallOptions::new().prune(true))?;
    let state_after_uninstall = raw_track_records_summary(&paths, tempdir.path())?;

    assert_debug_snapshot!((
        install_summary(&installed, tempdir.path())?,
        update_summary(&updated, tempdir.path())?,
        removal_intent_summary(&intent),
        state_after_uninstall,
    ));

    Ok(())
}

#[test]
fn managed_resource_commands_update_requires_fresh_manifest_after_cache_exists() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let commands =
        ManagedResourceCommands::new(paths.clone(), MANIFEST_URL, TargetPlatform::DarwinArm64);
    let adapter = FakeAdapter::new("redis", &["bin/pv-fake-resource"])?;
    let artifact = fixture_artifact("7.2.5-pv1", "first")?;
    let manifest = manifest_with_artifacts(&[&artifact]);
    let client = ScriptedClient::new()
        .with_text(&manifest)
        .with_bytes(artifact.bytes())
        .with_text_error(ResourcesError::HttpRequestFailed {
            url: MANIFEST_URL.to_string(),
            reason: "offline".to_string(),
        });

    commands.install(&adapter, TrackSelector::Latest, &client)?;
    let update = commands.update(&adapter, &client);

    assert_debug_snapshot!(update);

    Ok(())
}

#[test]
fn managed_resource_commands_install_reports_cached_manifest_fallback() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let commands =
        ManagedResourceCommands::new(paths.clone(), MANIFEST_URL, TargetPlatform::DarwinArm64);
    let adapter = FakeAdapter::new("redis", &["bin/pv-fake-resource"])?;
    let artifact = fixture_artifact("7.2.5-pv1", "first")?;
    let manifest = manifest_with_artifacts(&[&artifact]);
    let client = ScriptedClient::new()
        .with_text(&manifest)
        .with_bytes(artifact.bytes())
        .with_text_error(ResourcesError::HttpRequestFailed {
            url: MANIFEST_URL.to_string(),
            reason: "offline".to_string(),
        });

    commands.install(&adapter, TrackSelector::Latest, &client)?;
    let reinstalled = commands.install(&adapter, TrackSelector::Latest, &client)?;

    assert_debug_snapshot!(install_summary(&reinstalled, tempdir.path())?);

    Ok(())
}

#[test]
fn managed_resource_commands_install_uses_existing_release_without_downloading_again() -> Result<()>
{
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let commands =
        ManagedResourceCommands::new(paths.clone(), MANIFEST_URL, TargetPlatform::DarwinArm64);
    let adapter = FakeAdapter::new("redis", &["bin/pv-fake-resource"])?;
    let artifact = fixture_artifact("7.2.5-pv1", "first")?;
    let manifest = manifest_with_artifacts(&[&artifact]);
    let client = ScriptedClient::new()
        .with_text(&manifest)
        .with_bytes(artifact.bytes())
        .with_text(&manifest);

    let installed = commands.install(&adapter, TrackSelector::Latest, &client)?;
    remove_download_cache(&paths)?;
    let reinstalled = commands.install(&adapter, TrackSelector::Latest, &client)?;
    let artifact_download_count = client.byte_request_count();

    assert_debug_snapshot!((
        install_summary(&installed, tempdir.path())?,
        install_summary(&reinstalled, tempdir.path())?,
        artifact_download_count,
    ));

    Ok(())
}

#[test]
fn managed_resource_commands_uninstall_records_prune_and_force_intent() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let commands =
        ManagedResourceCommands::new(paths.clone(), MANIFEST_URL, TargetPlatform::DarwinArm64);
    let adapter = FakeAdapter::new("redis", &["bin/pv-fake-resource"])?;
    let artifact = fixture_artifact("7.2.5-pv1", "first")?;
    let manifest = manifest_with_artifacts(&[&artifact]);
    let client = ScriptedClient::new()
        .with_text(&manifest)
        .with_bytes(artifact.bytes());
    let resource_name = ResourceName::new("redis")?;
    let track = TrackName::new("7.2")?;

    commands.install(&adapter, TrackSelector::Latest, &client)?;
    let intent = commands.uninstall(
        &resource_name,
        &track,
        ManagedResourceUninstallOptions::new()
            .prune(true)
            .force(true),
    )?;
    let database = Database::open(&paths)?;
    let removal_prune = state::testing::query_i64(
        &database,
        "SELECT removal_prune FROM managed_resource_tracks WHERE resource_name = 'redis' AND track = '7.2'",
    )?;
    let removal_force = state::testing::query_i64(
        &database,
        "SELECT removal_force FROM managed_resource_tracks WHERE resource_name = 'redis' AND track = '7.2'",
    )?;

    assert_debug_snapshot!((
        removal_intent_summary(&intent),
        removal_prune,
        removal_force
    ));

    Ok(())
}

#[test]
fn managed_resource_commands_uninstall_rejects_in_use_tracks_without_force() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let commands =
        ManagedResourceCommands::new(paths.clone(), MANIFEST_URL, TargetPlatform::DarwinArm64);
    let adapter = FakeAdapter::new("redis", &["bin/pv-fake-resource"])?;
    let artifact = fixture_artifact("7.2.5-pv1", "first")?;
    let manifest = manifest_with_artifacts(&[&artifact]);
    let client = ScriptedClient::new()
        .with_text(&manifest)
        .with_bytes(artifact.bytes());
    let resource_name = ResourceName::new("redis")?;
    let track = TrackName::new("7.2")?;

    commands.install(&adapter, TrackSelector::Latest, &client)?;
    set_usage_count(&paths, "redis", "7.2", 2)?;

    let rejected = commands.uninstall(
        &resource_name,
        &track,
        ManagedResourceUninstallOptions::default(),
    );
    let forced = commands.uninstall(
        &resource_name,
        &track,
        ManagedResourceUninstallOptions::new().force(true),
    )?;

    assert_debug_snapshot!((rejected, removal_intent_summary(&forced)));

    Ok(())
}

#[test]
fn managed_resource_commands_uninstall_rejects_tracks_that_are_not_installed() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let commands =
        ManagedResourceCommands::new(paths.clone(), MANIFEST_URL, TargetPlatform::DarwinArm64);
    let resource_name = ResourceName::new("redis")?;
    let track = TrackName::new("7.2")?;

    let result = commands.uninstall(
        &resource_name,
        &track,
        ManagedResourceUninstallOptions::default(),
    );

    assert_debug_snapshot!(result);

    Ok(())
}

#[test]
fn managed_resource_commands_uninstall_rejects_latest_alias_as_concrete_track() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let commands =
        ManagedResourceCommands::new(paths.clone(), MANIFEST_URL, TargetPlatform::DarwinArm64);
    let resource_name = ResourceName::new("redis")?;
    let latest = TrackName::new("latest")?;

    let result = commands.uninstall(
        &resource_name,
        &latest,
        ManagedResourceUninstallOptions::default(),
    );

    assert_debug_snapshot!(result);

    Ok(())
}

#[test]
fn scripted_client_reports_destination_write_failures_separately() -> Result<()> {
    let client = ScriptedClient::new().with_bytes(b"artifact");
    let mut writer = FailingWriter;
    let url = "https://artifacts.example.test/redis.tar.gz";
    let result = client.download(url, &mut writer);

    let Err(ResourcesError::DownloadWriteFailed {
        url: error_url,
        reason,
    }) = result
    else {
        bail!("expected DownloadWriteFailed, got {result:?}");
    };

    assert_eq!(error_url, url);
    assert_eq!(reason, "disk full");

    Ok(())
}

struct FakeAdapter {
    resource_name: ResourceName,
    required_paths: Vec<Utf8PathBuf>,
}

impl FakeAdapter {
    fn new(resource_name: &str, required_paths: &[&str]) -> Result<Self> {
        Ok(Self {
            resource_name: ResourceName::new(resource_name)?,
            required_paths: required_paths.iter().map(Utf8PathBuf::from).collect(),
        })
    }
}

impl ResourceAdapter for FakeAdapter {
    fn resource_name(&self) -> &ResourceName {
        &self.resource_name
    }

    fn validate_installation(&self, root: &Utf8Path) -> resources::Result<()> {
        for required_path in &self.required_paths {
            if !root.join(required_path).exists() {
                return Err(ResourcesError::InvalidArtifactLayout {
                    resource: self.resource_name.as_str().to_string(),
                    reason: format!("missing required path `{required_path}`"),
                });
            }
        }

        Ok(())
    }
}

#[derive(Clone, Debug)]
struct FixtureArtifact {
    resource_name: String,
    version: String,
    platform: String,
    bytes: Vec<u8>,
    sha256: String,
    revoked_reason: Option<String>,
}

impl FixtureArtifact {
    fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

struct ManifestResourceFixture<'a> {
    name: &'a str,
    default_track: &'a str,
    tracks: Vec<ManifestTrackFixture<'a>>,
}

struct ManifestTrackFixture<'a> {
    name: &'a str,
    artifacts: Vec<&'a FixtureArtifact>,
}

#[derive(Debug)]
#[expect(
    dead_code,
    reason = "snapshot-only structure is read through derived Debug"
)]
struct InstallSnapshot {
    resource_name: String,
    track: String,
    artifact_version: String,
    current_artifact_path: String,
    manifest_source: String,
    revoked_latest: Option<RevokedLatestSnapshot>,
    downloaded_from_cache: bool,
}

#[derive(Debug)]
#[expect(
    dead_code,
    reason = "snapshot-only structure is read through derived Debug"
)]
struct TrackRecordSnapshot {
    resource_name: String,
    track: String,
    installed_version: String,
    current_artifact_path: String,
    usage_count: i64,
}

#[derive(Debug)]
#[expect(
    dead_code,
    reason = "snapshot-only structure is read through derived Debug"
)]
struct RevokedLatestSnapshot {
    artifact_version: String,
    reason: String,
}

#[derive(Debug)]
#[expect(
    dead_code,
    reason = "snapshot-only structure is read through derived Debug"
)]
struct RemovalIntentSnapshot {
    resource_name: String,
    track: String,
    prune: bool,
    force: bool,
}

#[derive(Debug)]
#[expect(
    dead_code,
    reason = "snapshot-only structure is read through derived Debug"
)]
struct RawTrackRecordSnapshot {
    resource_name: String,
    track: String,
    desired_state: String,
    installed_version: Option<String>,
    current_artifact_path: Option<String>,
    usage_count: i64,
    removal_prune: bool,
    removal_force: bool,
}

fn update_summary(update: &ManagedResourceUpdate, root: &Utf8Path) -> Result<Vec<InstallSnapshot>> {
    install_summaries(update.installs(), root)
}

fn install_summaries(
    installs: &[ManagedResourceInstall],
    root: &Utf8Path,
) -> Result<Vec<InstallSnapshot>> {
    installs
        .iter()
        .map(|install| install_summary(install, root))
        .collect()
}

fn install_summary(install: &ManagedResourceInstall, root: &Utf8Path) -> Result<InstallSnapshot> {
    Ok(InstallSnapshot {
        resource_name: install.resource_name().as_str().to_string(),
        track: install.track().as_str().to_string(),
        artifact_version: install.artifact_version().as_str().to_string(),
        current_artifact_path: install
            .current_artifact_path()
            .strip_prefix(root)?
            .to_string(),
        manifest_source: manifest_source_summary(install.manifest_source()),
        revoked_latest: install
            .revoked_latest()
            .map(|revoked_latest| RevokedLatestSnapshot {
                artifact_version: revoked_latest.artifact_version().as_str().to_string(),
                reason: revoked_latest.reason().to_string(),
            }),
        downloaded_from_cache: install.downloaded_from_cache(),
    })
}

fn manifest_source_summary(source: &ArtifactManifestSource) -> String {
    match source {
        ArtifactManifestSource::Latest => "Latest".to_string(),
        ArtifactManifestSource::Cached { reason } => format!("Cached: {reason}"),
    }
}

fn track_records_summary(
    records: &[ManagedResourceTrack],
    root: &Utf8Path,
) -> Result<Vec<TrackRecordSnapshot>> {
    records
        .iter()
        .map(|record| track_record_summary(record, root))
        .collect()
}

fn track_record_summary(
    record: &ManagedResourceTrack,
    root: &Utf8Path,
) -> Result<TrackRecordSnapshot> {
    Ok(TrackRecordSnapshot {
        resource_name: record.resource_name().as_str().to_string(),
        track: record.track().as_str().to_string(),
        installed_version: record.installed_version().as_str().to_string(),
        current_artifact_path: record
            .current_artifact_path()
            .strip_prefix(root)?
            .to_string(),
        usage_count: record.usage_count(),
    })
}

fn removal_intent_summary(intent: &ManagedResourceRemovalIntent) -> RemovalIntentSnapshot {
    RemovalIntentSnapshot {
        resource_name: intent.resource_name().as_str().to_string(),
        track: intent.track().as_str().to_string(),
        prune: intent.prune(),
        force: intent.force(),
    }
}

fn raw_track_records_summary(
    paths: &PvPaths,
    root: &Utf8Path,
) -> Result<Vec<RawTrackRecordSnapshot>> {
    let database = Database::open(paths)?;
    database
        .managed_resource_tracks()?
        .iter()
        .map(|record| raw_track_record_summary(record, root))
        .collect()
}

fn raw_track_record_summary(
    record: &ManagedResourceTrackRecord,
    root: &Utf8Path,
) -> Result<RawTrackRecordSnapshot> {
    Ok(RawTrackRecordSnapshot {
        resource_name: record.resource_name.clone(),
        track: record.track.clone(),
        desired_state: format!("{:?}", record.desired_state),
        installed_version: record.installed_version.clone(),
        current_artifact_path: record
            .current_artifact_path
            .as_ref()
            .map(|path| path.strip_prefix(root).map(Utf8Path::to_string))
            .transpose()?,
        usage_count: record.usage_count,
        removal_prune: record.removal_prune,
        removal_force: record.removal_force,
    })
}

#[derive(Debug)]
struct ScriptedClient {
    text_responses: RefCell<VecDeque<Result<String, ResourcesError>>>,
    byte_responses: RefCell<VecDeque<Result<Vec<u8>, ResourcesError>>>,
    text_request_count: Cell<usize>,
    byte_request_count: Cell<usize>,
}

impl ScriptedClient {
    fn new() -> Self {
        Self {
            text_responses: RefCell::new(VecDeque::new()),
            byte_responses: RefCell::new(VecDeque::new()),
            text_request_count: Cell::new(0),
            byte_request_count: Cell::new(0),
        }
    }

    fn with_text(self, text: &str) -> Self {
        self.text_responses
            .borrow_mut()
            .push_back(Ok(text.to_string()));
        self
    }

    fn with_text_error(self, error: ResourcesError) -> Self {
        self.text_responses.borrow_mut().push_back(Err(error));
        self
    }

    fn with_bytes(self, bytes: &[u8]) -> Self {
        self.byte_responses
            .borrow_mut()
            .push_back(Ok(bytes.to_vec()));
        self
    }

    fn text_request_count(&self) -> usize {
        self.text_request_count.get()
    }

    fn byte_request_count(&self) -> usize {
        self.byte_request_count.get()
    }
}

impl ResourceHttpClient for ScriptedClient {
    fn get_text(&self, url: &str) -> resources::Result<String> {
        self.text_request_count
            .set(self.text_request_count.get() + 1);
        self.text_responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| {
                Err(ResourcesError::HttpRequestFailed {
                    url: url.to_string(),
                    reason: "no scripted text response".to_string(),
                })
            })
    }

    fn download(&self, url: &str, writer: &mut dyn Write) -> resources::Result<()> {
        self.byte_request_count
            .set(self.byte_request_count.get() + 1);
        let bytes = self
            .byte_responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| {
                Err(ResourcesError::HttpRequestFailed {
                    url: url.to_string(),
                    reason: "no scripted byte response".to_string(),
                })
            })?;
        writer
            .write_all(&bytes)
            .map_err(|source| ResourcesError::DownloadWriteFailed {
                url: url.to_string(),
                reason: source.to_string(),
            })
    }
}

struct FailingWriter;

impl Write for FailingWriter {
    fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
        Err(Error::other("disk full"))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn fixture_artifact(version: &str, marker: &str) -> Result<FixtureArtifact> {
    fixture_artifact_with_entries(
        version,
        &[("bin/pv-fake-resource", &format!("fake resource {marker}"))],
    )
}

fn revoked_fixture_artifact(version: &str, marker: &str, reason: &str) -> Result<FixtureArtifact> {
    let mut artifact = fixture_artifact(version, marker)?;
    artifact.revoked_reason = Some(reason.to_string());

    Ok(artifact)
}

fn runtime_fixture_artifact(
    resource_name: &str,
    version: &str,
    executable_path: &str,
    marker: &str,
) -> Result<FixtureArtifact> {
    fixture_artifact_for(
        resource_name,
        version,
        "darwin-arm64",
        &[(executable_path, marker)],
    )
}

fn composer_fixture_artifact(version: &str, marker: &str) -> Result<FixtureArtifact> {
    fixture_artifact_for(
        "composer",
        version,
        "any",
        &[("composer.phar", &format!("composer {marker}"))],
    )
}

fn fixture_artifact_with_entries(
    version: &str,
    entries: &[(&str, &str)],
) -> Result<FixtureArtifact> {
    fixture_artifact_for("redis", version, "darwin-arm64", entries)
}

fn fixture_artifact_for(
    resource_name: &str,
    version: &str,
    platform: &str,
    entries: &[(&str, &str)],
) -> Result<FixtureArtifact> {
    let root = format!("{resource_name}-{version}-{platform}");
    let bytes = fixture_archive_bytes(&root, entries)?;
    let sha256 = sha256_hex(&bytes);

    Ok(FixtureArtifact {
        resource_name: resource_name.to_string(),
        version: version.to_string(),
        platform: platform.to_string(),
        bytes,
        sha256,
        revoked_reason: None,
    })
}

fn fixture_archive_bytes(root: &str, entries: &[(&str, &str)]) -> Result<Vec<u8>> {
    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut builder = Builder::new(encoder);

    for (path, content) in entries {
        let path = format!("{root}/{path}");
        let mut header = Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder.append_data(&mut header, path, content.as_bytes())?;
    }

    let encoder = builder.into_inner()?;
    Ok(encoder.finish()?)
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);

    for byte in digest {
        hex.push(HEX[(byte >> 4) as usize] as char);
        hex.push(HEX[(byte & 0x0f) as usize] as char);
    }

    hex
}

fn manifest_with_artifacts(artifacts: &[&FixtureArtifact]) -> String {
    manifest_with_tracks(&[("7.2", artifacts)])
}

fn manifest_with_tracks(tracks: &[(&str, &[&FixtureArtifact])]) -> String {
    let tracks = tracks
        .iter()
        .map(|(track, artifacts)| manifest_track(track, artifacts.to_vec()))
        .collect::<Vec<_>>();

    manifest_with_resources(&[manifest_resource("redis", "7.2", tracks)])
}

fn manifest_resource<'a>(
    name: &'a str,
    default_track: &'a str,
    tracks: Vec<ManifestTrackFixture<'a>>,
) -> ManifestResourceFixture<'a> {
    ManifestResourceFixture {
        name,
        default_track,
        tracks,
    }
}

fn manifest_track<'a>(
    name: &'a str,
    artifacts: Vec<&'a FixtureArtifact>,
) -> ManifestTrackFixture<'a> {
    ManifestTrackFixture { name, artifacts }
}

fn manifest_with_resources(resources: &[ManifestResourceFixture<'_>]) -> String {
    let resources = resources
        .iter()
        .map(|resource| {
            let tracks = resource
                .tracks
                .iter()
                .map(manifest_track_json)
                .collect::<Vec<_>>()
                .join(",");

            format!(
                r#"{{
      "name": "{}",
      "default_track": "{}",
      "tracks": [
        {tracks}
      ]
    }}"#,
                resource.name, resource.default_track,
            )
        })
        .collect::<Vec<_>>()
        .join(",");

    format!(
        r#"
{{
  "schema_version": 1,
  "minimum_pv_version": "0.1.0",
  "resources": [
    {resources}
  ]
}}
"#
    )
}

fn manifest_track_json(track: &ManifestTrackFixture<'_>) -> String {
    let artifacts = track
        .artifacts
        .iter()
        .map(|artifact| {
            let revocation = artifact
                .revoked_reason
                .as_ref()
                .map_or_else(String::new, |reason| {
                    format!(
                        r#",
              "revoked": true,
              "revocation_reason": "{reason}""#
                    )
                });

            format!(
                r#"{{
              "artifact_version": "{}",
              "upstream_version": "{}",
              "pv_build_revision": "1",
              "platform": "{}",
              "url": "https://artifacts.example.test/{}-{}-{}.tar.gz",
              "sha256": "{}",
              "size": {},
              "published_at": "{}"{revocation}
            }}"#,
                artifact.version,
                artifact.version.trim_end_matches("-pv1"),
                artifact.platform,
                artifact.resource_name,
                artifact.version,
                artifact.platform,
                artifact.sha256,
                artifact.bytes.len(),
                published_at_for(&artifact.version),
            )
        })
        .collect::<Vec<_>>()
        .join(",");

    format!(
        r#"{{
          "name": "{}",
          "artifacts": [
            {artifacts}
          ]
        }}"#,
        track.name,
    )
}

fn published_at_for(version: &str) -> &'static str {
    match version {
        "2.8.0-pv1" => "2026-05-26T16:30:00Z",
        "2.8.1-pv1" => "2026-05-27T16:30:00Z",
        "8.3.22-pv1" => "2026-05-25T12:30:00Z",
        "8.3.23-pv1" => "2026-05-26T12:30:00Z",
        "8.3.24-pv1" => "2026-05-27T12:30:00Z",
        "8.4.8-pv1" => "2026-05-26T13:30:00Z",
        "8.4.9-pv1" => "2026-05-27T13:30:00Z",
        "7.2.5-pv1" => "2026-05-26T14:30:00Z",
        "7.2.6-pv1" => "2026-05-27T14:30:00Z",
        "7.2.7-pv1" => "2026-05-28T14:30:00Z",
        "8.0.1-pv1" => "2026-05-26T15:30:00Z",
        "8.0.2-pv1" => "2026-05-27T15:30:00Z",
        _ => "2026-05-28T14:30:00Z",
    }
}

const MANIFEST_URL: &str = "https://artifacts.example.test/manifest.json";

#[expect(
    clippy::disallowed_methods,
    reason = "test removes the artifact cache to prove existing releases avoid downloads"
)]
fn remove_download_cache(paths: &PvPaths) -> Result<()> {
    match std::fs::remove_dir_all(paths.downloads()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn set_usage_count(
    paths: &PvPaths,
    resource_name: &str,
    track: &str,
    usage_count: i64,
) -> Result<()> {
    let mut database = Database::open(paths)?;
    state::testing::transaction(&mut database, |transaction| {
        transaction.execute(
            "UPDATE managed_resource_tracks
            SET usage_count = ?1
            WHERE resource_name = ?2 AND track = ?3",
            (usage_count, resource_name, track),
        )?;

        Ok(())
    })?;

    Ok(())
}
