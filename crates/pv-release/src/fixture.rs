use camino::Utf8Path;
use flate2::Compression;
use flate2::write::GzEncoder;
use resources::ArtifactPlatform;
use serde::Serialize;
use std::io::Write;
use tar::{Builder, Header};

use crate::recipe::{ComposerRecipe, PhpRecipe, PhpTrack};

const PUBLISHED_AT: &str = "2026-01-01T00:00:00Z";

pub fn generate_recipe_fixtures(
    php: &Utf8Path,
    composer: &Utf8Path,
    archives: &Utf8Path,
    records: &Utf8Path,
    pv_commit: &str,
    build_run_id: &str,
) -> crate::Result<()> {
    let php = PhpRecipe::load(php)?;
    let composer = ComposerRecipe::load(composer)?;

    for track in php.tracks() {
        for platform in php.platforms() {
            write_php_fixture(
                &php,
                track,
                *platform,
                archives,
                records,
                pv_commit,
                build_run_id,
            )?;
            write_frankenphp_fixture(
                &php,
                track,
                *platform,
                archives,
                records,
                pv_commit,
                build_run_id,
            )?;
        }
    }
    write_composer_fixture(&composer, archives, records, pv_commit, build_run_id)
}

fn write_php_fixture(
    recipe: &PhpRecipe,
    track: &PhpTrack,
    platform: ArtifactPlatform,
    archives: &Utf8Path,
    records: &Utf8Path,
    pv_commit: &str,
    build_run_id: &str,
) -> crate::Result<()> {
    let upstream_version = track.php_version();
    let pv_build_revision = recipe.pv_build_revision();
    let recipe_path = recipe_provenance_path(recipe.path());
    let artifact = FixtureArtifact {
        resource: "php",
        track: track.name().as_str(),
        upstream_version,
        pv_build_revision,
        platform,
        payload_path: "bin/php",
        source_url: track.php_source_url(),
        source_sha256: track.php_source_sha256().as_str(),
        recipe: &recipe_path,
        minimum_pv_version: recipe.minimum_pv_version().as_str(),
        license_files: recipe.license_files(),
        notice_files: recipe.notice_files(),
        pv_commit,
        build_run_id,
    };

    write_fixture_artifact(archives, records, &artifact)
}

fn write_frankenphp_fixture(
    recipe: &PhpRecipe,
    track: &PhpTrack,
    platform: ArtifactPlatform,
    archives: &Utf8Path,
    records: &Utf8Path,
    pv_commit: &str,
    build_run_id: &str,
) -> crate::Result<()> {
    let upstream_version = format!(
        "{}-frankenphp{}",
        track.php_version(),
        recipe.frankenphp_version()
    );
    let pv_build_revision = recipe.pv_build_revision();
    let recipe_path = recipe_provenance_path(recipe.path());
    let artifact = FixtureArtifact {
        resource: "frankenphp",
        track: track.name().as_str(),
        upstream_version: &upstream_version,
        pv_build_revision,
        platform,
        payload_path: "bin/frankenphp",
        source_url: recipe.frankenphp_source_url(),
        source_sha256: recipe.frankenphp_source_sha256().as_str(),
        recipe: &recipe_path,
        minimum_pv_version: recipe.minimum_pv_version().as_str(),
        license_files: recipe.license_files(),
        notice_files: recipe.notice_files(),
        pv_commit,
        build_run_id,
    };

    write_fixture_artifact(archives, records, &artifact)
}

fn write_composer_fixture(
    recipe: &ComposerRecipe,
    archives: &Utf8Path,
    records: &Utf8Path,
    pv_commit: &str,
    build_run_id: &str,
) -> crate::Result<()> {
    let upstream_version = recipe.upstream_version();
    let pv_build_revision = recipe.pv_build_revision();
    let recipe_path = recipe_provenance_path(recipe.path());
    let artifact = FixtureArtifact {
        resource: "composer",
        track: recipe.track().as_str(),
        upstream_version,
        pv_build_revision,
        platform: recipe.platform(),
        payload_path: "composer.phar",
        source_url: recipe.source_url(),
        source_sha256: recipe.source_sha256().as_str(),
        recipe: &recipe_path,
        minimum_pv_version: recipe.minimum_pv_version().as_str(),
        license_files: recipe.license_files(),
        notice_files: recipe.notice_files(),
        pv_commit,
        build_run_id,
    };

    write_fixture_artifact(archives, records, &artifact)
}

struct FixtureArtifact<'a> {
    resource: &'a str,
    track: &'a str,
    upstream_version: &'a str,
    pv_build_revision: &'a str,
    platform: ArtifactPlatform,
    payload_path: &'a str,
    source_url: &'a str,
    source_sha256: &'a str,
    recipe: &'a str,
    minimum_pv_version: &'a str,
    license_files: &'a [String],
    notice_files: &'a [String],
    pv_commit: &'a str,
    build_run_id: &'a str,
}

#[derive(Serialize)]
struct ReleaseRecordJson<'a> {
    resource: &'a str,
    track: &'a str,
    upstream_version: &'a str,
    pv_build_revision: &'a str,
    artifact_version: &'a str,
    platform: &'a str,
    object_key: &'a str,
    sha256: &'a str,
    size: u64,
    published_at: &'a str,
    minimum_pv_version: &'a str,
    license_files: &'a [String],
    notice_files: &'a [String],
    provenance: ProvenanceJson<'a>,
}

#[derive(Serialize)]
struct ProvenanceJson<'a> {
    source_url: &'a str,
    source_sha256: &'a str,
    recipe: &'a str,
    pv_commit: &'a str,
    build_run_id: &'a str,
}

fn write_fixture_artifact(
    archives: &Utf8Path,
    records: &Utf8Path,
    artifact: &FixtureArtifact<'_>,
) -> crate::Result<()> {
    let artifact_version = format!(
        "{}-{}",
        artifact.upstream_version, artifact.pv_build_revision
    );
    let platform = artifact.platform.as_str();
    let artifact_basename = artifact_basename(artifact.resource, &artifact_version, platform);
    let object_key = object_key(
        artifact.resource,
        artifact.track,
        &artifact_version,
        platform,
        &artifact_basename,
    );
    let archive = archives.join(&object_key);
    write_fixture_archive(&archive, artifact, &artifact_basename)?;

    let license_files = artifact
        .license_files
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let notice_files = artifact
        .notice_files
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let validation = crate::archive::validate_archive(&archive, &license_files, &notice_files)?;
    let record = ReleaseRecordJson {
        resource: artifact.resource,
        track: artifact.track,
        upstream_version: artifact.upstream_version,
        pv_build_revision: artifact.pv_build_revision,
        artifact_version: &artifact_version,
        platform,
        object_key: &object_key,
        sha256: validation.sha256(),
        size: validation.size(),
        published_at: PUBLISHED_AT,
        minimum_pv_version: artifact.minimum_pv_version,
        license_files: artifact.license_files,
        notice_files: artifact.notice_files,
        provenance: ProvenanceJson {
            source_url: artifact.source_url,
            source_sha256: artifact.source_sha256,
            recipe: artifact.recipe,
            pv_commit: artifact.pv_commit,
            build_run_id: artifact.build_run_id,
        },
    };
    let record_path = records.join(record_key(
        artifact.resource,
        artifact.track,
        &artifact_version,
        platform,
        &artifact_basename,
    ));
    write_record(&record_path, &record)?;
    crate::archive::validate_archive_for_record_file(&archive, &record_path)
}

fn artifact_basename(resource: &str, artifact_version: &str, platform: &str) -> String {
    format!("{resource}-{artifact_version}-{platform}")
}

fn object_key(
    resource: &str,
    track: &str,
    artifact_version: &str,
    platform: &str,
    artifact_basename: &str,
) -> String {
    format!("resources/{resource}/{track}/{artifact_version}/{platform}/{artifact_basename}.tar.gz")
}

fn record_key(
    resource: &str,
    track: &str,
    artifact_version: &str,
    platform: &str,
    artifact_basename: &str,
) -> String {
    format!("resources/{resource}/{track}/{artifact_version}/{platform}/{artifact_basename}.json")
}

fn write_fixture_archive(
    archive: &Utf8Path,
    artifact: &FixtureArtifact<'_>,
    archive_root: &str,
) -> crate::Result<()> {
    if let Some(parent) = archive.parent() {
        create_dir_all(parent)?;
    }

    let file = create_file(archive)?;
    let encoder = GzEncoder::new(file, Compression::best());
    let mut builder = Builder::new(encoder);
    for license_file in artifact.license_files {
        append_archive_file(
            archive,
            &mut builder,
            &format!("{archive_root}/{license_file}"),
            b"fixture license\n",
            0o644,
        )?;
    }
    for notice_file in artifact.notice_files {
        append_archive_file(
            archive,
            &mut builder,
            &format!("{archive_root}/{notice_file}"),
            b"fixture notice\n",
            0o644,
        )?;
    }
    append_archive_file(
        archive,
        &mut builder,
        &format!("{archive_root}/{}", artifact.payload_path),
        b"fixture binary\n",
        if artifact.payload_path.starts_with("bin/") {
            0o755
        } else {
            0o644
        },
    )?;
    let encoder = builder
        .into_inner()
        .map_err(|error| filesystem_error(archive, error))?;
    encoder
        .finish()
        .map_err(|error| filesystem_error(archive, error))?;

    Ok(())
}

fn append_archive_file<W: Write>(
    archive: &Utf8Path,
    builder: &mut Builder<W>,
    path: &str,
    content: &[u8],
    mode: u32,
) -> crate::Result<()> {
    let mut header = Header::new_gnu();
    header.set_size(content.len() as u64);
    header.set_mode(mode);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_cksum();
    builder
        .append_data(&mut header, path, content)
        .map_err(|error| filesystem_error(archive, error))
}

fn write_record(path: &Utf8Path, record: &ReleaseRecordJson<'_>) -> crate::Result<()> {
    if let Some(parent) = path.parent() {
        create_dir_all(parent)?;
    }
    let mut json = serde_json::to_string_pretty(record).map_err(|error| {
        crate::ReleaseError::InvalidReleaseRecord {
            path: path.to_string(),
            reason: error.to_string(),
        }
    })?;
    json.push('\n');
    write(path, json.as_bytes())
}

fn recipe_provenance_path(path: &Utf8Path) -> String {
    let release_root = Utf8Path::new("release").join("artifacts");
    if let Ok(path) = path.strip_prefix(&release_root) {
        return release_root.join(path).to_string();
    }

    let mut components = Vec::new();
    let mut found_release_root = false;
    for component in path.components() {
        if found_release_root {
            components.push(component.as_str());
            continue;
        }
        if component.as_str() == "release" {
            components.push(component.as_str());
            found_release_root = true;
        }
    }

    if components.get(1).copied() == Some("artifacts") {
        components.join("/")
    } else {
        path.to_string()
    }
}

#[expect(
    clippy::disallowed_types,
    reason = "PV release tooling owns direct archive fixture file creation"
)]
fn create_file(path: &Utf8Path) -> crate::Result<std::fs::File> {
    std::fs::File::create(path).map_err(|error| filesystem_error(path, error))
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV release tooling creates generated fixture directories"
)]
fn create_dir_all(path: &Utf8Path) -> crate::Result<()> {
    std::fs::create_dir_all(path).map_err(|error| filesystem_error(path, error))
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV release tooling writes generated fixture records"
)]
fn write(path: &Utf8Path, content: &[u8]) -> crate::Result<()> {
    std::fs::write(path, content).map_err(|error| filesystem_error(path, error))
}

fn filesystem_error(path: &Utf8Path, error: impl ToString) -> crate::ReleaseError {
    crate::ReleaseError::Filesystem {
        path: path.to_string(),
        reason: error.to_string(),
    }
}
