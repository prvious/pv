use anyhow::Result;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::assert_snapshot;
use pv_release::archive::validate_archive;
use pv_release::manifest::generate_manifest_file_with_defaults;
use pv_release::record::{ReleaseRecord, load_release_records};
use resources::ArtifactManifest;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ArchiveRoot {
    resource: String,
    track: String,
    platform: String,
    root: String,
}

#[test]
fn recipe_fixture_generation_validates_archives_records_and_manifest() -> Result<()> {
    let workspace_root = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let tempdir = tempdir()?;
    let archives = tempdir.path().join("archives");
    let records = tempdir.path().join("records");
    let revocations = tempdir.path().join("revocations");
    let manifest = tempdir.path().join("manifest.json");
    let php = workspace_root.join("release/artifacts/recipes/php/tracks.toml");
    let composer = workspace_root.join("release/artifacts/recipes/composer/composer.toml");
    let defaults = workspace_root.join("release/artifacts/default-tracks.toml");

    create_dir_all(&revocations)?;
    pv_release::fixture::generate_recipe_fixtures(
        &php,
        &composer,
        &archives,
        &records,
        "0123456789abcdef0123456789abcdef01234567",
        "local-test",
    )?;
    let archive_roots = generated_archive_roots(&archives, &records)?;
    assert_eq!(
        representative_archive_roots(&archive_roots),
        vec![
            "composer-2.10.1-pv1-any",
            "frankenphp-8.4.20-frankenphp1.12.3-pv1-darwin-arm64",
            "php-8.4.20-pv1-darwin-arm64",
        ],
    );
    generate_manifest_file_with_defaults(
        &records,
        &revocations,
        Some(&defaults),
        &manifest,
        "https://artifacts.example.test",
    )?;

    let manifest_json = read_to_string(&manifest)?;
    ArtifactManifest::parse(&manifest_json)?;
    assert_snapshot!(manifest_json);

    Ok(())
}

fn generated_archive_roots(archives: &Utf8Path, records: &Utf8Path) -> Result<Vec<ArchiveRoot>> {
    let mut archive_roots = load_release_records(records)?
        .iter()
        .map(|record| generated_archive_root(archives, record))
        .collect::<Result<Vec<_>>>()?;
    archive_roots.sort();
    Ok(archive_roots)
}

fn generated_archive_root(archives: &Utf8Path, record: &ReleaseRecord) -> Result<ArchiveRoot> {
    let license_files = record
        .license_files()
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let notice_files = record
        .notice_files()
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let archive = archives.join(record.object_key());
    let validation = validate_archive(&archive, &license_files, &notice_files)?;

    Ok(ArchiveRoot {
        resource: record.resource().to_string(),
        track: record.track().to_string(),
        platform: record.platform().as_str().to_string(),
        root: validation.root().to_string(),
    })
}

fn representative_archive_roots(archive_roots: &[ArchiveRoot]) -> Vec<&str> {
    archive_roots
        .iter()
        .filter(|archive_root| {
            matches!(
                (
                    archive_root.resource.as_str(),
                    archive_root.track.as_str(),
                    archive_root.platform.as_str(),
                ),
                ("composer", "2", "any")
                    | ("frankenphp", "8.4", "darwin-arm64")
                    | ("php", "8.4", "darwin-arm64")
            )
        })
        .map(|archive_root| archive_root.root.as_str())
        .collect()
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests create local revocation fixture directories"
)]
fn create_dir_all(path: &Utf8Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests read generated local manifests"
)]
fn read_to_string(path: &Utf8Path) -> Result<String> {
    Ok(std::fs::read_to_string(path)?)
}
