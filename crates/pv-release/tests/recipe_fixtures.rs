use anyhow::{Result, bail};
use camino::Utf8Path;
use camino_tempfile::tempdir;
use flate2::read::GzDecoder;
use insta::{assert_debug_snapshot, assert_snapshot};
use pv_release::archive::validate_archive;
use pv_release::fixture::generate_recipe_fixtures_with_backing;
use pv_release::manifest::generate_manifest_file_with_defaults;
use pv_release::recipe::{BackingRecipe, BackingRecipeKind};
use pv_release::record::{ReleaseRecord, load_release_records};
use resources::ArtifactManifest;
use tar::Archive;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ArchiveRoot {
    resource: String,
    track: String,
    platform: String,
    root: String,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct BackingFixtureSummary {
    track: String,
    upstream_version: String,
    artifact_version: String,
    platform: String,
    object_key: String,
    record_key: String,
    source_url: String,
    source_sha256: String,
    recipe: String,
    archive_entries: Vec<String>,
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
    let redis = workspace_root.join("release/artifacts/recipes/redis/recipe.toml");
    let mysql = workspace_root.join("release/artifacts/recipes/mysql/recipe.toml");
    let postgres = workspace_root.join("release/artifacts/recipes/postgres/recipe.toml");
    let mailpit = workspace_root.join("release/artifacts/recipes/mailpit/recipe.toml");
    let rustfs = workspace_root.join("release/artifacts/recipes/rustfs/recipe.toml");
    let defaults = workspace_root.join("release/artifacts/default-tracks.toml");

    create_dir_all(&revocations)?;
    let redis_recipe = BackingRecipe::load(&redis, BackingRecipeKind::Redis)?;
    let Some(redis_track) = redis_recipe.tracks().first() else {
        bail!("committed Redis recipe should define a track");
    };
    let redis_upstream_version = redis_track.upstream_version().to_string();
    generate_recipe_fixtures_with_backing(
        &php,
        &composer,
        &[
            (BackingRecipeKind::Redis, redis.clone()),
            (BackingRecipeKind::Mysql, mysql),
            (BackingRecipeKind::Postgres, postgres),
            (BackingRecipeKind::Mailpit, mailpit),
            (BackingRecipeKind::Rustfs, rustfs),
        ],
        &archives,
        &records,
        "0123456789abcdef0123456789abcdef01234567",
        "local-test",
    )?;
    let archive_roots = generated_archive_roots(&archives, &records)?;
    assert_eq!(
        archive_roots,
        vec![
            ArchiveRoot::new("composer", "2", "any", "composer-2.10.1-pv1-any"),
            ArchiveRoot::new(
                "frankenphp",
                "8.3",
                "darwin-amd64",
                "frankenphp-8.3.31-frankenphp1.12.4-pv1-darwin-amd64",
            ),
            ArchiveRoot::new(
                "frankenphp",
                "8.3",
                "darwin-arm64",
                "frankenphp-8.3.31-frankenphp1.12.4-pv1-darwin-arm64",
            ),
            ArchiveRoot::new(
                "frankenphp",
                "8.4",
                "darwin-amd64",
                "frankenphp-8.4.22-frankenphp1.12.4-pv1-darwin-amd64",
            ),
            ArchiveRoot::new(
                "frankenphp",
                "8.4",
                "darwin-arm64",
                "frankenphp-8.4.22-frankenphp1.12.4-pv1-darwin-arm64",
            ),
            ArchiveRoot::new(
                "frankenphp",
                "8.5",
                "darwin-amd64",
                "frankenphp-8.5.7-frankenphp1.12.4-pv1-darwin-amd64",
            ),
            ArchiveRoot::new(
                "frankenphp",
                "8.5",
                "darwin-arm64",
                "frankenphp-8.5.7-frankenphp1.12.4-pv1-darwin-arm64",
            ),
            ArchiveRoot::new(
                "mailpit",
                "1",
                "darwin-amd64",
                "mailpit-1.30.1-pv1-darwin-amd64",
            ),
            ArchiveRoot::new(
                "mailpit",
                "1",
                "darwin-arm64",
                "mailpit-1.30.1-pv1-darwin-arm64",
            ),
            ArchiveRoot::new(
                "mysql",
                "8.0",
                "darwin-amd64",
                "mysql-8.0.46-pv1-darwin-amd64"
            ),
            ArchiveRoot::new(
                "mysql",
                "8.0",
                "darwin-arm64",
                "mysql-8.0.46-pv1-darwin-arm64"
            ),
            ArchiveRoot::new(
                "mysql",
                "8.4",
                "darwin-amd64",
                "mysql-8.4.9-pv1-darwin-amd64"
            ),
            ArchiveRoot::new(
                "mysql",
                "8.4",
                "darwin-arm64",
                "mysql-8.4.9-pv1-darwin-arm64"
            ),
            ArchiveRoot::new(
                "mysql",
                "9.7",
                "darwin-amd64",
                "mysql-9.7.0-pv1-darwin-amd64"
            ),
            ArchiveRoot::new(
                "mysql",
                "9.7",
                "darwin-arm64",
                "mysql-9.7.0-pv1-darwin-arm64"
            ),
            ArchiveRoot::new("php", "8.3", "darwin-amd64", "php-8.3.31-pv1-darwin-amd64"),
            ArchiveRoot::new("php", "8.3", "darwin-arm64", "php-8.3.31-pv1-darwin-arm64"),
            ArchiveRoot::new("php", "8.4", "darwin-amd64", "php-8.4.22-pv1-darwin-amd64"),
            ArchiveRoot::new("php", "8.4", "darwin-arm64", "php-8.4.22-pv1-darwin-arm64"),
            ArchiveRoot::new("php", "8.5", "darwin-amd64", "php-8.5.7-pv1-darwin-amd64"),
            ArchiveRoot::new("php", "8.5", "darwin-arm64", "php-8.5.7-pv1-darwin-arm64"),
            ArchiveRoot::new(
                "postgres",
                "17",
                "darwin-amd64",
                "postgres-17.10-pv1-darwin-amd64",
            ),
            ArchiveRoot::new(
                "postgres",
                "17",
                "darwin-arm64",
                "postgres-17.10-pv1-darwin-arm64",
            ),
            ArchiveRoot::new(
                "postgres",
                "18",
                "darwin-amd64",
                "postgres-18.4-pv1-darwin-amd64",
            ),
            ArchiveRoot::new(
                "postgres",
                "18",
                "darwin-arm64",
                "postgres-18.4-pv1-darwin-arm64",
            ),
            ArchiveRoot::new(
                "redis",
                "8.8",
                "darwin-amd64",
                &format!("redis-{redis_upstream_version}-pv1-darwin-amd64"),
            ),
            ArchiveRoot::new(
                "redis",
                "8.8",
                "darwin-arm64",
                &format!("redis-{redis_upstream_version}-pv1-darwin-arm64"),
            ),
            ArchiveRoot::new(
                "rustfs",
                "1",
                "darwin-amd64",
                "rustfs-1.0.0-beta.7-pv1-darwin-amd64",
            ),
            ArchiveRoot::new(
                "rustfs",
                "1",
                "darwin-arm64",
                "rustfs-1.0.0-beta.7-pv1-darwin-arm64",
            ),
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

#[test]
fn recipe_fixture_generation_validates_archives_records_and_manifest_with_backing_recipe()
-> Result<()> {
    let workspace_root = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let tempdir = tempdir()?;
    let archives = tempdir.path().join("archives");
    let records = tempdir.path().join("records");
    let revocations = tempdir.path().join("revocations");
    let manifest = tempdir.path().join("manifest.json");
    let php = workspace_root.join("release/artifacts/recipes/php/tracks.toml");
    let composer = workspace_root.join("release/artifacts/recipes/composer/composer.toml");
    let mailpit = workspace_root.join("release/artifacts/recipes/mailpit/recipe.toml");
    let rustfs = workspace_root.join("release/artifacts/recipes/rustfs/recipe.toml");
    let defaults = workspace_root.join("release/artifacts/default-tracks.toml");
    let mysql = workspace_root.join("release/artifacts/recipes/mysql/recipe.toml");
    let postgres = workspace_root.join("release/artifacts/recipes/postgres/recipe.toml");
    let redis = tempdir
        .path()
        .join("release/artifacts/recipes/redis/recipe.toml");

    create_dir_all(&revocations)?;
    write_file(&redis, VALID_REDIS_TOML)?;
    generate_recipe_fixtures_with_backing(
        &php,
        &composer,
        &[
            (BackingRecipeKind::Redis, redis.clone()),
            (BackingRecipeKind::Mysql, mysql),
            (BackingRecipeKind::Postgres, postgres),
            (BackingRecipeKind::Mailpit, mailpit),
            (BackingRecipeKind::Rustfs, rustfs),
        ],
        &archives,
        &records,
        "0123456789abcdef0123456789abcdef01234567",
        "local-test",
    )?;

    let mut backing_summaries = load_release_records(&records)?
        .iter()
        .filter(|record| record.resource().as_str() == "redis")
        .map(|record| backing_fixture_summary(&archives, &records, record))
        .collect::<Result<Vec<_>>>()?;
    backing_summaries.sort();
    assert_debug_snapshot!(backing_summaries);

    generate_manifest_file_with_defaults(
        &records,
        &revocations,
        Some(&defaults),
        &manifest,
        "https://artifacts.example.test",
    )?;
    let manifest_json = read_to_string(&manifest)?;
    ArtifactManifest::parse(&manifest_json)?;

    Ok(())
}

#[test]
fn recipe_fixture_generation_validates_committed_mailpit_and_rustfs_recipes() -> Result<()> {
    let workspace_root = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let tempdir = tempdir()?;
    let archives = tempdir.path().join("archives");
    let records = tempdir.path().join("records");
    let revocations = tempdir.path().join("revocations");
    let manifest = tempdir.path().join("manifest.json");
    let php = workspace_root.join("release/artifacts/recipes/php/tracks.toml");
    let composer = workspace_root.join("release/artifacts/recipes/composer/composer.toml");
    let mailpit = workspace_root.join("release/artifacts/recipes/mailpit/recipe.toml");
    let rustfs = workspace_root.join("release/artifacts/recipes/rustfs/recipe.toml");
    let defaults = tempdir.path().join("default-tracks.toml");

    create_dir_all(&revocations)?;
    write_file(&defaults, MAILPIT_RUSTFS_FIXTURE_DEFAULTS)?;
    generate_recipe_fixtures_with_backing(
        &php,
        &composer,
        &[
            (BackingRecipeKind::Mailpit, mailpit),
            (BackingRecipeKind::Rustfs, rustfs),
        ],
        &archives,
        &records,
        "0123456789abcdef0123456789abcdef01234567",
        "local-test",
    )?;

    let mut backing_summaries = load_release_records(&records)?
        .iter()
        .filter(|record| matches!(record.resource().as_str(), "mailpit" | "rustfs"))
        .map(|record| backing_fixture_summary(&archives, &records, record))
        .collect::<Result<Vec<_>>>()?;
    backing_summaries.sort();
    assert_debug_snapshot!(backing_summaries);

    generate_manifest_file_with_defaults(
        &records,
        &revocations,
        Some(&defaults),
        &manifest,
        "https://artifacts.example.test",
    )?;
    let manifest_json = read_to_string(&manifest)?;
    ArtifactManifest::parse(&manifest_json)?;

    Ok(())
}

impl ArchiveRoot {
    fn new(resource: &str, track: &str, platform: &str, root: &str) -> Self {
        Self {
            resource: resource.to_owned(),
            track: track.to_owned(),
            platform: platform.to_owned(),
            root: root.to_owned(),
        }
    }
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

fn backing_fixture_summary(
    archives: &Utf8Path,
    records: &Utf8Path,
    record: &ReleaseRecord,
) -> Result<BackingFixtureSummary> {
    let record_key = record_key(record)?;
    let record_path = records.join(&record_key);
    assert!(
        path_exists(&record_path),
        "expected generated record path `{record_path}` to exist"
    );

    Ok(BackingFixtureSummary {
        track: record.track().as_str().to_string(),
        upstream_version: record.upstream_version().to_string(),
        artifact_version: record.artifact_version().as_str().to_string(),
        platform: record.platform().as_str().to_string(),
        object_key: record.object_key().to_string(),
        record_key,
        source_url: record.provenance().source_url().to_string(),
        source_sha256: record.provenance().source_sha256().to_string(),
        recipe: record.provenance().recipe().to_string(),
        archive_entries: archive_entries(&archives.join(record.object_key()))?,
    })
}

fn record_key(record: &ReleaseRecord) -> Result<String> {
    let Some(prefix) = record.object_key().strip_suffix(".tar.gz") else {
        bail!("object key `{}` must end with .tar.gz", record.object_key());
    };
    Ok(format!("{prefix}.json"))
}

fn archive_entries(path: &Utf8Path) -> Result<Vec<String>> {
    let file = open_file(path)?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);
    let mut entries = Vec::new();
    for entry in archive.entries()? {
        let entry = entry?;
        entries.push(entry.path()?.to_string_lossy().into_owned());
    }
    entries.sort();
    Ok(entries)
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

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests write local recipe metadata"
)]
fn write_file(path: &Utf8Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(())
}

fn path_exists(path: &Utf8Path) -> bool {
    path.exists()
}

#[expect(
    clippy::disallowed_types,
    reason = "release tooling tests read generated tar archives"
)]
fn open_file(path: &Utf8Path) -> Result<std::fs::File> {
    Ok(std::fs::File::open(path)?)
}

const VALID_REDIS_TOML: &str = r#"
[recipe]
resources = ["redis"]
default_track = "8.8"
platforms = ["darwin-arm64", "darwin-amd64"]
minimum_pv_version = "0.1.0"
pv_build_revision = "pv1"
license_files = ["LICENSE"]
notice_files = ["NOTICE"]

[artifact]
payload_paths = ["bin/redis-server", "bin/redis-cli"]

[[tracks]]
name = "8.8"
upstream_version = "8.8.0"
source_url = "https://download.redis.io/releases/redis-8.8.0.tar.gz"
source_sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
"#;

const MAILPIT_RUSTFS_FIXTURE_DEFAULTS: &str = r#"
[[resource]]
name = "php"
default_track = "8.5"

[[resource]]
name = "frankenphp"
default_track = "8.5"

[[resource]]
name = "composer"
default_track = "2"

[[resource]]
name = "mailpit"
default_track = "1"

[[resource]]
name = "rustfs"
default_track = "1"
"#;
