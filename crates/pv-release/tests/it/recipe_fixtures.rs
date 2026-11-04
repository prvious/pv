use std::process::Output;

use anyhow::{Context, Result, bail};
use camino::Utf8Path;
use camino_tempfile::tempdir;
use pv_release::archive::validate_archive;
use pv_release::record::load_release_records;
use resources::ArtifactManifest;

#[expect(
    clippy::disallowed_types,
    reason = "release tooling CLI tests execute the pv-release binary"
)]
type StdCommand = std::process::Command;

#[test]
fn recipe_fixture_generation_validates_archives_records_and_manifest() -> Result<()> {
    let workspace_root = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let tempdir = tempdir()?;
    let archives = tempdir.path().join("archives");
    let records = tempdir.path().join("records");
    let revocations = workspace_root.join("release/artifacts/revocations");
    let manifest = tempdir.path().join("manifest.json");
    let php = workspace_root.join("release/artifacts/recipes/php/tracks.toml");
    let composer = workspace_root.join("release/artifacts/recipes/composer/composer.toml");
    let redis = workspace_root.join("release/artifacts/recipes/redis/recipe.toml");
    let mysql = workspace_root.join("release/artifacts/recipes/mysql/recipe.toml");
    let postgres = workspace_root.join("release/artifacts/recipes/postgres/recipe.toml");
    let mailpit = workspace_root.join("release/artifacts/recipes/mailpit/recipe.toml");
    let caddy = workspace_root.join("release/artifacts/recipes/caddy/recipe.toml");
    let rustfs = workspace_root.join("release/artifacts/recipes/rustfs/recipe.toml");
    let defaults = workspace_root.join("release/artifacts/default-tracks.toml");

    let fixture_output = run_pv_release(&[
        "generate-recipe-fixtures",
        "--php",
        php.as_str(),
        "--composer",
        composer.as_str(),
        "--redis",
        redis.as_str(),
        "--mysql",
        mysql.as_str(),
        "--postgres",
        postgres.as_str(),
        "--mailpit",
        mailpit.as_str(),
        "--caddy",
        caddy.as_str(),
        "--rustfs",
        rustfs.as_str(),
        "--archives",
        archives.as_str(),
        "--records",
        records.as_str(),
        "--pv-commit",
        "0123456789abcdef0123456789abcdef01234567",
        "--build-run-id",
        "local-test",
    ])?;
    assert_command_success(&fixture_output, "generate-recipe-fixtures")?;

    // Every archive unpacks into one folder named after its published file.
    for record in load_release_records(&records)? {
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
        let archive_name = archive
            .file_name()
            .and_then(|name| name.strip_suffix(".tar.gz"))
            .context("archive object key must end with .tar.gz")?;
        assert_eq!(validation.root(), archive_name);
    }

    let manifest_output = run_pv_release(&[
        "generate-manifest",
        "--records",
        records.as_str(),
        "--revocations",
        revocations.as_str(),
        "--defaults",
        defaults.as_str(),
        "--output",
        manifest.as_str(),
        "--base-url",
        "https://artifacts.example.test",
    ])?;
    assert_command_success(&manifest_output, "generate-manifest")?;
    ArtifactManifest::parse(&read_to_string(&manifest)?)?;

    Ok(())
}

fn run_pv_release(args: &[&str]) -> Result<Output> {
    StdCommand::new(env!("CARGO_BIN_EXE_pv-release"))
        .args(args)
        .output()
        .context("failed to execute pv-release")
}

fn assert_command_success(output: &Output, label: &str) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }

    bail!(
        "{label} failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests read generated local manifests"
)]
fn read_to_string(path: &Utf8Path) -> Result<String> {
    Ok(std::fs::read_to_string(path)?)
}
